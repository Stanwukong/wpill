//! Pure hover logic: hit tests and the expand/collapse state machine. No Win32 here, so it's unit-tested.

use std::time::{Duration, Instant};

use crate::activity::Queue;
use crate::ipc::{Activity, ActivityPayload};
use crate::window::{Geometry, PANEL_TOP};

/// Cursor must rest in the hot zone this long before the island opens.
pub const ARM_DELAY: Duration = Duration::from_millis(150);
/// Cursor must stay outside the panel this long before it collapses.
pub const LEAVE_DELAY: Duration = Duration::from_millis(250);

/// Hot zone in logical px: a narrow strip along the top edge, centered.
pub const HOT_ZONE_WIDTH: f64 = 200.0;
pub const HOT_ZONE_HEIGHT: f64 = 10.0;
/// Slack around the panel before the cursor counts as having left.
pub const PANEL_MARGIN: f64 = 8.0;

/// The panel's hit area in physical px. It's horizontally centered on the monitor and
/// extends from the monitor's top edge down to `bottom`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PanelRect {
    pub width: f64,
    pub bottom: f64,
}

impl PanelRect {
    /// Rough guess used before React reports the real rect.
    pub fn default_for(geo: &Geometry) -> Self {
        Self {
            width: 640.0 * geo.scale,
            bottom: (PANEL_TOP + 220.0) * geo.scale,
        }
    }

    /// Generous guess for an activity before React reports its size (the screenshot variant is the tallest).
    pub fn default_activity(geo: &Geometry) -> Self {
        Self {
            width: 360.0 * geo.scale,
            bottom: (PANEL_TOP + 96.0) * geo.scale,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Collapsed,
    /// In the hot zone since the given instant, waiting out `ARM_DELAY`.
    Arming(Instant),
    Expanded,
    /// Outside the panel since the given instant, waiting out `LEAVE_DELAY`.
    Leaving(Instant),
    /// Collapsed on request; won't re-arm until the cursor leaves the hot zone.
    Suppressed,
    /// An activity is on screen (its end time lives in the queue).
    Showing {
        /// In the hot zone since then: dwelling there still opens the panel.
        arm: Option<Instant>,
        /// The cursor is on an interactive activity, or it's being dragged: clickable, countdown paused.
        held: bool,
        /// Entered from `Suppressed`: the hot zone doesn't arm until the cursor leaves it.
        suppressed: bool,
    },
}

impl Phase {
    pub fn is_expanded(self) -> bool {
        matches!(self, Phase::Expanded | Phase::Leaving(_))
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Sample {
    pub now: Instant,
    pub in_hot_zone: bool,
    pub in_panel: bool,
    /// Inside the reported activity rect.
    pub in_activity: bool,
    /// A screenshot drag is in progress.
    pub dragging: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transition {
    Expand,
    Collapse,
}

pub fn step(phase: Phase, s: &Sample) -> (Phase, Option<Transition>) {
    match phase {
        Phase::Collapsed if s.in_hot_zone => (Phase::Arming(s.now), None),
        Phase::Collapsed => (Phase::Collapsed, None),
        Phase::Arming(_) if !s.in_hot_zone => (Phase::Collapsed, None),
        Phase::Arming(since) if s.now.saturating_duration_since(since) >= ARM_DELAY => {
            (Phase::Expanded, Some(Transition::Expand))
        }
        Phase::Arming(_) => (phase, None),
        Phase::Expanded if s.in_panel => (Phase::Expanded, None),
        Phase::Expanded => (Phase::Leaving(s.now), None),
        Phase::Leaving(_) if s.in_panel => (Phase::Expanded, None),
        Phase::Leaving(since) if s.now.saturating_duration_since(since) >= LEAVE_DELAY => {
            (Phase::Collapsed, Some(Transition::Collapse))
        }
        Phase::Leaving(_) => (phase, None),
        Phase::Suppressed if s.in_hot_zone => (Phase::Suppressed, None),
        Phase::Suppressed => (Phase::Collapsed, None),
        // Driven by `Machine::tick`, which owns the activity queue.
        Phase::Showing { .. } => (phase, None),
    }
}

pub fn force_collapse(phase: Phase) -> (Phase, Option<Transition>) {
    let transition = phase.is_expanded().then_some(Transition::Collapse);
    (Phase::Suppressed, transition)
}

/// What the poller must emit, in order.
#[derive(Debug, Clone, PartialEq)]
pub enum Output {
    Expand,
    Collapse,
    Show(ActivityPayload),
    Hide(u32),
}

/// The hover phase plus the activity queue. An activity is a phase, so hover rules still apply
/// on top of it: dwelling in the hot zone opens the panel, and nothing ever covers an open panel.
#[derive(Debug)]
pub struct Machine {
    pub phase: Phase,
    queue: Queue,
}

impl Default for Machine {
    fn default() -> Self {
        Self {
            phase: Phase::Collapsed,
            queue: Queue::default(),
        }
    }
}

impl Machine {
    pub fn offer(&mut self, activity: Activity, now: Instant, hidden: bool, out: &mut Vec<Output>) {
        let can_show = !hidden && !self.phase.is_expanded();
        if let Some(payload) = self.queue.offer(activity, now, can_show) {
            self.enter_showing();
            out.push(Output::Show(payload));
        }
    }

    pub fn tick(&mut self, s: &Sample, out: &mut Vec<Output>) {
        let Phase::Showing {
            mut arm,
            mut suppressed,
            ..
        } = self.phase
        else {
            let (next, transition) = step(self.phase, s);
            self.phase = next;
            match transition {
                Some(Transition::Expand) => out.push(Output::Expand),
                Some(Transition::Collapse) => out.push(Output::Collapse),
                None => {}
            }
            self.show_next(s.now, out);
            return;
        };

        if s.in_hot_zone && !suppressed {
            let since = *arm.get_or_insert(s.now);
            if s.now.saturating_duration_since(since) >= ARM_DELAY {
                self.park(s.now, out);
                self.phase = Phase::Expanded;
                out.push(Output::Expand);
                return;
            }
        } else if !s.in_hot_zone {
            arm = None;
            suppressed = false;
        }

        let held = s.dragging || (s.in_activity && self.queue.current_interactive());
        if held {
            self.queue.hold(s.now);
        } else if self.queue.is_over(s.now) {
            self.finish(out);
            self.phase = idle(arm, suppressed);
            self.show_next(s.now, out);
            return;
        }
        self.phase = Phase::Showing {
            arm,
            held,
            suppressed,
        };
    }

    /// `request_collapse`: closes the panel or ends the activity (a sticky one comes back), then waits for the
    /// cursor to leave the hot zone.
    pub fn force_collapse(&mut self, now: Instant, out: &mut Vec<Output>) {
        self.park(now, out);
        let (next, transition) = force_collapse(self.phase);
        self.phase = next;
        if transition.is_some() {
            out.push(Output::Collapse);
        }
    }

    /// The island is being hidden (fullscreen app). Waiting notices stay queued and may go stale; a sticky
    /// one on screen goes back in line.
    pub fn hide(&mut self, now: Instant, out: &mut Vec<Output>) {
        if self.phase.is_expanded() {
            out.push(Output::Collapse);
        }
        self.park(now, out);
        self.phase = Phase::Collapsed;
    }

    /// A source withdrew `key` (e.g. a Claude Code session got its answer): drop it, waiting or showing.
    pub fn retract(&mut self, key: &str, out: &mut Vec<Output>) {
        if let Some(id) = self.queue.retract(key) {
            out.push(Output::Hide(id));
            if let Phase::Showing {
                arm, suppressed, ..
            } = self.phase
            {
                self.phase = idle(arm, suppressed);
            }
        }
    }

    /// Ends activity `id` early (e.g. its screenshot was opened), if it's still the one showing.
    pub fn dismiss(&mut self, id: u32, out: &mut Vec<Output>) {
        if let Phase::Showing {
            arm, suppressed, ..
        } = self.phase
        {
            if self.queue.current_id() == Some(id) {
                self.finish(out);
                self.phase = idle(arm, suppressed);
            }
        }
    }

    /// Only the open panel, or an interactive activity under the cursor, takes clicks.
    pub fn accepts_clicks(&self) -> bool {
        matches!(
            self.phase,
            Phase::Expanded | Phase::Showing { held: true, .. }
        )
    }

    #[cfg(test)]
    pub fn showing(&self) -> bool {
        matches!(self.phase, Phase::Showing { .. })
    }

    fn finish(&mut self, out: &mut Vec<Output>) {
        if let Some(id) = self.queue.finish() {
            out.push(Output::Hide(id));
        }
    }

    fn park(&mut self, now: Instant, out: &mut Vec<Output>) {
        if let Some(id) = self.queue.park(now) {
            out.push(Output::Hide(id));
        }
    }

    fn show_next(&mut self, now: Instant, out: &mut Vec<Output>) {
        if matches!(
            self.phase,
            Phase::Collapsed | Phase::Arming(_) | Phase::Suppressed
        ) {
            if let Some(payload) = self.queue.take_next(now) {
                self.enter_showing();
                out.push(Output::Show(payload));
            }
        }
    }

    fn enter_showing(&mut self) {
        let (arm, suppressed) = match self.phase {
            Phase::Arming(since) => (Some(since), false),
            Phase::Suppressed => (None, true),
            Phase::Showing { .. } => return,
            _ => (None, false),
        };
        self.phase = Phase::Showing {
            arm,
            held: false,
            suppressed,
        };
    }
}

/// Where an activity hands back to when it ends.
fn idle(arm: Option<Instant>, suppressed: bool) -> Phase {
    match (arm, suppressed) {
        (_, true) => Phase::Suppressed,
        (Some(since), false) => Phase::Arming(since),
        (None, false) => Phase::Collapsed,
    }
}

pub fn in_hot_zone(geo: &Geometry, x: i32, y: i32) -> bool {
    let dx = (x - geo.center_x).abs() as f64;
    let dy = (y - geo.monitor_top) as f64;
    dy >= 0.0 && dy < HOT_ZONE_HEIGHT * geo.scale && dx <= HOT_ZONE_WIDTH * geo.scale / 2.0
}

pub fn in_panel(geo: &Geometry, panel: PanelRect, x: i32, y: i32) -> bool {
    let margin = PANEL_MARGIN * geo.scale;
    let dx = (x - geo.center_x).abs() as f64;
    let dy = (y - geo.monitor_top) as f64;
    dy >= 0.0 && dy <= panel.bottom + margin && dx <= panel.width / 2.0 + margin
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity::claude_key;
    use crate::activity::test_activities::{needs_input, screenshot, volume};

    fn sample(now: Instant, in_hot_zone: bool, in_panel: bool) -> Sample {
        Sample {
            now,
            in_hot_zone,
            in_panel,
            in_activity: false,
            dragging: false,
        }
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn on_activity(now: Instant) -> Sample {
        Sample {
            in_activity: true,
            ..sample(now, false, true)
        }
    }

    /// Runs one tick and returns what it emitted.
    fn tick(m: &mut Machine, s: Sample) -> Vec<Output> {
        let mut out = Vec::new();
        m.tick(&s, &mut out);
        out
    }

    fn offer(m: &mut Machine, a: Activity, now: Instant) -> Vec<Output> {
        let mut out = Vec::new();
        m.offer(a, now, false, &mut out);
        out
    }

    fn shown_id(out: &[Output]) -> u32 {
        match out {
            [.., Output::Show(p)] => p.id,
            _ => panic!("expected a Show, got {out:?}"),
        }
    }

    #[test]
    fn activity_shows_from_collapsed_and_ends_after_its_duration() {
        let t0 = Instant::now();
        let mut m = Machine::default();
        let id = shown_id(&offer(&mut m, volume(40), t0));
        assert!(m.showing());
        assert!(!m.accepts_clicks());
        assert!(tick(&mut m, sample(t0 + ms(1000), false, false)).is_empty());
        let out = tick(&mut m, sample(t0 + ms(1500), false, false));
        assert_eq!(out, vec![Output::Hide(id)]);
        assert_eq!(m.phase, Phase::Collapsed);
    }

    #[test]
    fn dwelling_in_the_hot_zone_during_an_activity_opens_the_panel() {
        let t0 = Instant::now();
        let mut m = Machine::default();
        let id = shown_id(&offer(&mut m, volume(40), t0));
        assert!(tick(&mut m, sample(t0 + ms(10), true, true)).is_empty());
        let out = tick(&mut m, sample(t0 + ms(10) + ARM_DELAY, true, true));
        assert_eq!(out, vec![Output::Hide(id), Output::Expand]);
        assert_eq!(m.phase, Phase::Expanded);
    }

    #[test]
    fn an_activity_arriving_while_arming_keeps_the_arm() {
        let t0 = Instant::now();
        let mut m = Machine::default();
        tick(&mut m, sample(t0, true, true));
        offer(&mut m, volume(40), t0 + ms(100));
        let out = tick(&mut m, sample(t0 + ARM_DELAY, true, true));
        assert_eq!(out.last(), Some(&Output::Expand));
    }

    #[test]
    fn activities_never_cover_the_open_panel() {
        let t0 = Instant::now();
        let mut m = Machine {
            phase: Phase::Expanded,
            ..Machine::default()
        };
        assert!(offer(&mut m, volume(40), t0).is_empty());
        assert!(offer(&mut m, screenshot(), t0).is_empty());
        assert_eq!(m.phase, Phase::Expanded);
        // Leaving: still no activity. Once collapsed, the waiting screenshot shows; the volume was dropped.
        tick(&mut m, sample(t0 + ms(10), false, false));
        assert!(offer(&mut m, volume(41), t0 + ms(20)).is_empty());
        let out = tick(&mut m, sample(t0 + ms(10) + LEAVE_DELAY, false, false));
        assert_eq!(out[0], Output::Collapse);
        assert!(
            matches!(&out[1], Output::Show(p) if matches!(p.activity, Activity::Screenshot(_)))
        );
        assert!(m.showing());
    }

    #[test]
    fn hovering_a_screenshot_holds_it_and_makes_it_clickable() {
        let t0 = Instant::now();
        let mut m = Machine::default();
        let id = shown_id(&offer(&mut m, screenshot(), t0));
        tick(&mut m, on_activity(t0 + ms(100)));
        assert!(m.accepts_clicks());
        // Held well past its 6 s duration.
        assert!(tick(&mut m, on_activity(t0 + ms(20_000))).is_empty());
        // Released: it stays HOLD_GRACE longer, unclickable, then ends.
        assert!(tick(&mut m, sample(t0 + ms(20_100), false, false)).is_empty());
        assert!(!m.accepts_clicks());
        let out = tick(&mut m, sample(t0 + ms(21_000), false, false));
        assert_eq!(out, vec![Output::Hide(id)]);
    }

    #[test]
    fn hovering_a_volume_activity_does_not_hold_it_or_take_clicks() {
        let t0 = Instant::now();
        let mut m = Machine::default();
        offer(&mut m, volume(40), t0);
        tick(&mut m, on_activity(t0 + ms(100)));
        assert!(!m.accepts_clicks());
        let out = tick(&mut m, on_activity(t0 + ms(1500)));
        assert!(matches!(out[..], [Output::Hide(_)]));
    }

    #[test]
    fn dragging_holds_the_activity_even_off_the_island() {
        let t0 = Instant::now();
        let mut m = Machine::default();
        offer(&mut m, screenshot(), t0);
        let dragging = Sample {
            dragging: true,
            ..sample(t0 + ms(10_000), false, false)
        };
        assert!(tick(&mut m, dragging).is_empty());
        assert!(m.showing());
    }

    #[test]
    fn burst_updates_in_place_and_queued_notice_follows() {
        let t0 = Instant::now();
        let mut m = Machine::default();
        let shot = shown_id(&offer(&mut m, screenshot(), t0));
        let vol = shown_id(&offer(&mut m, volume(10), t0 + ms(100)));
        assert_ne!(vol, shot);
        assert_eq!(shown_id(&offer(&mut m, volume(11), t0 + ms(200))), vol);
        // Volume ends 1.5 s after the last press; the screenshot resumes in the same tick.
        let out = tick(&mut m, sample(t0 + ms(1700), false, false));
        assert_eq!(out[0], Output::Hide(vol));
        assert!(
            matches!(&out[1], Output::Show(p) if matches!(p.activity, Activity::Screenshot(_)))
        );
    }

    #[test]
    fn forced_collapse_and_hide_end_the_activity() {
        let t0 = Instant::now();
        let mut m = Machine::default();
        let id = shown_id(&offer(&mut m, volume(40), t0));
        let mut out = Vec::new();
        m.force_collapse(t0, &mut out);
        assert_eq!(out, vec![Output::Hide(id)]);
        assert_eq!(m.phase, Phase::Suppressed);

        // Shown from Suppressed: the hot zone doesn't arm until the cursor leaves it.
        let mut m = Machine {
            phase: Phase::Suppressed,
            ..Machine::default()
        };
        offer(&mut m, volume(40), t0);
        tick(&mut m, sample(t0, true, true));
        assert!(tick(&mut m, sample(t0 + ms(400), true, true)).is_empty());

        let mut out = Vec::new();
        m.hide(t0, &mut out);
        assert!(matches!(out[..], [Output::Hide(_)]));
        assert_eq!(m.phase, Phase::Collapsed);
        let mut out = Vec::new();
        m.offer(volume(1), t0, true, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn dismiss_ends_only_the_matching_activity() {
        let t0 = Instant::now();
        let mut m = Machine::default();
        let id = shown_id(&offer(&mut m, screenshot(), t0));
        let mut out = Vec::new();
        m.dismiss(id + 1, &mut out);
        assert!(out.is_empty());
        m.dismiss(id, &mut out);
        assert_eq!(out, vec![Output::Hide(id)]);
        assert_eq!(m.phase, Phase::Collapsed);
    }

    #[test]
    fn needs_input_comes_back_after_the_panel_closes_and_ends_when_retracted() {
        let t0 = Instant::now();
        let mut m = Machine::default();
        let asked = shown_id(&offer(&mut m, needs_input(1), t0));
        // Never times out.
        assert!(tick(&mut m, sample(t0 + ms(60_000), false, false)).is_empty());
        // Dwell opens the panel; the question is parked, not dropped.
        tick(&mut m, sample(t0 + ms(60_010), true, true));
        let out = tick(&mut m, sample(t0 + ms(60_010) + ARM_DELAY, true, true));
        assert_eq!(out, vec![Output::Hide(asked), Output::Expand]);
        tick(&mut m, sample(t0 + ms(61_000), false, false));
        let out = tick(&mut m, sample(t0 + ms(61_000) + LEAVE_DELAY, false, false));
        assert_eq!(out[0], Output::Collapse);
        let back = shown_id(&out);
        assert!(m.showing());
        // Hovering it makes it clickable.
        tick(&mut m, on_activity(t0 + ms(62_000)));
        assert!(m.accepts_clicks());
        let mut out = Vec::new();
        m.retract(&claude_key(1), &mut out);
        assert_eq!(out, vec![Output::Hide(back)]);
        assert!(!m.showing());
    }

    #[test]
    fn a_question_asked_during_fullscreen_shows_afterwards_and_dismiss_acknowledges_it() {
        let t0 = Instant::now();
        let mut m = Machine::default();
        let mut out = Vec::new();
        m.offer(needs_input(1), t0, true, &mut out);
        assert!(out.is_empty());
        let id = shown_id(&tick(&mut m, sample(t0 + 30 * 60 * ms(1000), false, false)));
        let mut out = Vec::new();
        m.dismiss(id, &mut out);
        assert_eq!(out, vec![Output::Hide(id)]);
        assert!(tick(&mut m, sample(t0 + 31 * 60 * ms(1000), false, false)).is_empty());
    }

    #[test]
    fn opens_only_after_arm_delay() {
        let t0 = Instant::now();
        let (p, t) = step(Phase::Collapsed, &sample(t0, true, true));
        assert_eq!((p, t), (Phase::Arming(t0), None));
        let (p, t) = step(p, &sample(t0 + ms(100), true, true));
        assert_eq!(t, None);
        let (p, t) = step(p, &sample(t0 + ARM_DELAY, true, true));
        assert_eq!((p, t), (Phase::Expanded, Some(Transition::Expand)));
    }

    #[test]
    fn brushing_past_does_not_open() {
        let t0 = Instant::now();
        let (p, _) = step(Phase::Collapsed, &sample(t0, true, true));
        let (p, t) = step(p, &sample(t0 + ms(50), false, false));
        assert_eq!((p, t), (Phase::Collapsed, None));
    }

    #[test]
    fn collapses_after_leave_delay_and_returning_cancels() {
        let t0 = Instant::now();
        let (p, _) = step(Phase::Expanded, &sample(t0, false, false));
        assert_eq!(p, Phase::Leaving(t0));
        let (p, t) = step(p, &sample(t0 + ms(100), false, true));
        assert_eq!((p, t), (Phase::Expanded, None));
        let (p, _) = step(p, &sample(t0 + ms(200), false, false));
        let (p, t) = step(p, &sample(t0 + ms(200) + LEAVE_DELAY, false, false));
        assert_eq!((p, t), (Phase::Collapsed, Some(Transition::Collapse)));
    }

    #[test]
    fn forced_collapse_waits_for_cursor_to_leave_hot_zone() {
        let t0 = Instant::now();
        let (p, t) = force_collapse(Phase::Expanded);
        assert_eq!((p, t), (Phase::Suppressed, Some(Transition::Collapse)));
        let (p, _) = step(p, &sample(t0, true, true));
        assert_eq!(p, Phase::Suppressed);
        let (p, _) = step(p, &sample(t0 + ms(16), false, false));
        assert_eq!(p, Phase::Collapsed);
        assert_eq!(force_collapse(Phase::Collapsed).1, None);
    }

    #[test]
    fn hit_tests_scale_with_dpi() {
        let geo = Geometry {
            monitor_top: 0,
            center_x: 1440,
            scale: 1.5,
        };
        assert!(in_hot_zone(&geo, 1440, 0));
        assert!(in_hot_zone(&geo, 1440 + 150, 14));
        assert!(!in_hot_zone(&geo, 1440 + 151, 0));
        assert!(!in_hot_zone(&geo, 1440, 15));
        assert!(!in_hot_zone(&geo, 1440, -1));

        // Physical px; the 12 px margin is 8 logical px at 150%.
        let panel = PanelRect {
            width: 960.0,
            bottom: 300.0,
        };
        assert!(in_panel(&geo, panel, 1440 + 492, 312));
        assert!(!in_panel(&geo, panel, 1440 + 493, 10));
        assert!(!in_panel(&geo, panel, 1440, 313));
    }
}
