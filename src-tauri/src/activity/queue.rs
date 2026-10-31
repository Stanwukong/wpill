//! Which activity is on screen, what waits, and how bursts coalesce. Pure; unit-tested.
//!
//! Two classes. *Feedback* (volume, brightness, Caps Lock) answers a key the user just pressed: it shows at
//! once, replaces whatever is showing, and is dropped if it can't show (feedback seconds later is noise).
//! *Notices* (Bluetooth, power, screenshot, Claude Code) wait while the panel is open and go stale after a while.
//! Activities with the same key update in place: ten volume presses are one activity.
//!
//! A *sticky* notice (a Claude Code session needs your input) has no end time and never goes stale. It stays
//! until its source retracts it or the user acknowledges it (`finish`). It steps aside for every other notice and
//! comes back after it, and opening the panel parks it instead of dropping it.

use std::cmp::Reverse;
use std::time::{Duration, Instant};

use crate::ipc::{Activity, ActivityPayload, PowerEvent};

/// A notice interrupted by feedback resumes with at least this long left.
pub const MIN_RESUME: Duration = Duration::from_millis(1500);
/// While an interactive activity is held (hovered or dragged), it stays at least this long after release.
pub const HOLD_GRACE: Duration = Duration::from_millis(1000);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Feedback,
    Notice,
}

struct Rules {
    class: Class,
    priority: u8,
    duration: Duration,
    /// Notices only: a waiting notice older than this is dropped.
    stale_after: Duration,
    /// Shown until retracted; `duration` and `stale_after` don't apply.
    sticky: bool,
}

const ACTING_PRIORITY: u8 = 4;

const fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn rules(activity: &Activity) -> Rules {
    let feedback = |duration| Rules {
        class: Class::Feedback,
        priority: 1,
        duration,
        stale_after: Duration::ZERO,
        sticky: false,
    };
    let notice = |priority, duration, stale_after| Rules {
        class: Class::Notice,
        priority,
        duration,
        stale_after,
        sticky: false,
    };
    match activity {
        Activity::Volume(_) | Activity::Brightness(_) => feedback(ms(1500)),
        Activity::CapsLock(_) => feedback(ms(1200)),
        Activity::Power(p) if p.event == PowerEvent::Low => notice(3, ms(5000), ms(60_000)),
        Activity::Power(_) => notice(2, ms(3000), ms(10_000)),
        Activity::Bluetooth(_) => notice(2, ms(3000), ms(10_000)),
        Activity::Screenshot(_) => notice(2, ms(6000), ms(30_000)),
        Activity::ClaudeDone(_) => notice(2, ms(5000), ms(120_000)),
        Activity::ClaudeNeedsInput(_) | Activity::AgentApproval(_) => Rules {
            sticky: true,
            ..notice(3, Duration::ZERO, Duration::ZERO)
        },
        // Above approvals: while TopBar acts you should see it, and other runs' approvals wait their turn.
        Activity::AgentActing(_) => Rules {
            sticky: true,
            ..notice(ACTING_PRIORITY, Duration::ZERO, Duration::ZERO)
        },
        Activity::AgentFinished(_) => notice(2, ms(5000), ms(120_000)),
    }
}

/// Activities with equal keys coalesce. A plug-in replaces a waiting low-battery notice, which is intended.
fn key(activity: &Activity) -> String {
    match activity {
        Activity::Volume(_) => "volume".into(),
        Activity::Brightness(_) => "brightness".into(),
        Activity::CapsLock(_) => "capsLock".into(),
        Activity::Power(_) => "power".into(),
        Activity::Bluetooth(b) => format!("bluetooth:{}", b.container),
        Activity::Screenshot(_) => "screenshot".into(),
        // One key per session, so "done" replaces that session's "needs input" in place.
        Activity::ClaudeDone(c) => claude_key(c.session_id),
        Activity::ClaudeNeedsInput(c) => claude_key(c.session_id),
        // One per request, so approvals from several runs take turns like several sessions' questions.
        Activity::AgentApproval(a) => approval_key(a.request_id),
        Activity::AgentActing(_) => ACTING_KEY.into(),
        Activity::AgentFinished(f) => format!("agent:{}", f.run_id),
    }
}

/// One action runs at a time, so one key: waiting → acting updates it in place.
pub const ACTING_KEY: &str = "acting";

/// The key of an agent approval, for `Signal::Retract` once it's answered.
pub fn approval_key(request_id: u32) -> String {
    format!("approval:{request_id}")
}

/// The key a Claude Code session's activities share, for `Signal::Retract`.
pub fn claude_key(session_id: u32) -> String {
    format!("claude:{session_id}")
}

/// Interactive activities accept clicks while hovered and pause their countdown.
pub fn interactive(activity: &Activity) -> bool {
    matches!(
        activity,
        Activity::Screenshot(_)
            | Activity::ClaudeDone(_)
            | Activity::ClaudeNeedsInput(_)
            | Activity::AgentApproval(_)
            | Activity::AgentFinished(_)
    )
}

fn preempts(new: &Rules, current: &Shown) -> bool {
    if new.class == Class::Feedback {
        return true;
    }
    // TopBar acting on the desktop always shows at once (an approval on screen steps aside and comes back).
    if new.priority >= ACTING_PRIORITY {
        return true;
    }
    // A sticky notice on screen steps aside for any other notice; a new sticky one waits its turn.
    if current.ends.is_none() || new.sticky {
        return !new.sticky;
    }
    new.priority > rules(&current.activity).priority
}

#[derive(Debug)]
struct Shown {
    id: u32,
    key: String,
    activity: Activity,
    /// `None` while a sticky notice shows.
    ends: Option<Instant>,
}

#[derive(Debug)]
struct Waiting {
    key: String,
    activity: Activity,
    since: Instant,
    /// Set when it was interrupted: the time it had left.
    left: Option<Duration>,
}

#[derive(Debug, Default)]
pub struct Queue {
    current: Option<Shown>,
    waiting: Vec<Waiting>,
    next_id: u32,
}

impl Queue {
    /// Returns the payload to emit when this changes what's on screen (shown now, or updated in place).
    /// `can_show` is false while the panel is open or the island is hidden.
    pub fn offer(
        &mut self,
        activity: Activity,
        now: Instant,
        can_show: bool,
    ) -> Option<ActivityPayload> {
        let r = rules(&activity);
        let key = key(&activity);

        if let Some(current) = &mut self.current {
            if current.key == key {
                current.activity = activity;
                current.ends = (!r.sticky).then(|| match current.ends {
                    Some(ends) => ends.max(now + r.duration),
                    None => now + r.duration,
                });
                return Some(payload(current, now));
            }
        }
        if let Some(waiting) = self.waiting.iter_mut().find(|w| w.key == key) {
            waiting.activity = activity;
            waiting.since = now;
            waiting.left = None;
            return None;
        }

        let shows = can_show && self.current.as_ref().is_none_or(|c| preempts(&r, c));
        if !shows {
            if r.class == Class::Notice {
                self.waiting.push(Waiting {
                    key,
                    activity,
                    since: now,
                    left: None,
                });
            }
            return None;
        }
        if let Some(interrupted) = self.current.take() {
            self.requeue(interrupted, now);
        }
        let duration = (!r.sticky).then_some(r.duration);
        Some(self.show(key, activity, duration, now))
    }

    /// Shows the best waiting notice, if any is still fresh. Call only when nothing is showing.
    /// Sticky notices come last, so they never hold up a notice that can go stale.
    pub fn take_next(&mut self, now: Instant) -> Option<ActivityPayload> {
        if self.current.is_some() {
            return None;
        }
        self.waiting.retain(|w| {
            let r = rules(&w.activity);
            r.sticky || now.saturating_duration_since(w.since) <= r.stale_after
        });
        let best = (0..self.waiting.len()).max_by_key(|&i| {
            let r = rules(&self.waiting[i].activity);
            (!r.sticky, r.priority, Reverse(i))
        })?;
        let w = self.waiting.remove(best);
        let r = rules(&w.activity);
        let duration = (!r.sticky).then(|| w.left.unwrap_or(r.duration));
        Some(self.show(w.key, w.activity, duration, now))
    }

    /// Keeps the current activity up while it's hovered or dragged.
    pub fn hold(&mut self, now: Instant) {
        if let Some(Shown {
            ends: Some(ends), ..
        }) = &mut self.current
        {
            *ends = (*ends).max(now + HOLD_GRACE);
        }
    }

    pub fn is_over(&self, now: Instant) -> bool {
        self.current
            .as_ref()
            .is_none_or(|c| c.ends.is_some_and(|ends| now >= ends))
    }

    /// Ends the current activity for good (timed out or acknowledged); returns its id for `activity:hide`.
    pub fn finish(&mut self) -> Option<u32> {
        self.current.take().map(|c| c.id)
    }

    /// Takes the current activity off screen because something covers it (the panel, a fullscreen app).
    /// A sticky notice goes back to the front of the line; anything else ends.
    pub fn park(&mut self, now: Instant) -> Option<u32> {
        let current = self.current.take()?;
        let id = current.id;
        if current.ends.is_none() {
            self.waiting.insert(
                0,
                Waiting {
                    key: current.key,
                    activity: current.activity,
                    since: now,
                    left: None,
                },
            );
        }
        Some(id)
    }

    /// The source withdrew `key`: it leaves the line, and ends if it's on screen.
    pub fn retract(&mut self, key: &str) -> Option<u32> {
        self.waiting.retain(|w| w.key != key);
        if self.current.as_ref().is_some_and(|c| c.key == key) {
            return self.finish();
        }
        None
    }

    pub fn current_id(&self) -> Option<u32> {
        self.current.as_ref().map(|c| c.id)
    }

    pub fn current_interactive(&self) -> bool {
        self.current
            .as_ref()
            .is_some_and(|c| interactive(&c.activity))
    }

    fn requeue(&mut self, interrupted: Shown, now: Instant) {
        if rules(&interrupted.activity).class == Class::Notice {
            let left = interrupted
                .ends
                .map(|ends| ends.saturating_duration_since(now).max(MIN_RESUME));
            self.waiting.insert(
                0,
                Waiting {
                    key: interrupted.key,
                    activity: interrupted.activity,
                    since: now,
                    left,
                },
            );
        }
    }

    fn show(
        &mut self,
        key: String,
        activity: Activity,
        duration: Option<Duration>,
        now: Instant,
    ) -> ActivityPayload {
        self.next_id = self.next_id.wrapping_add(1).max(1);
        let shown = Shown {
            id: self.next_id,
            key,
            activity,
            ends: duration.map(|d| now + d),
        };
        let p = payload(&shown, now);
        self.current = Some(shown);
        p
    }
}

fn payload(shown: &Shown, now: Instant) -> ActivityPayload {
    ActivityPayload {
        id: shown.id,
        activity: shown.activity.clone(),
        duration_ms: shown
            .ends
            .map(|ends| ends.saturating_duration_since(now).as_millis() as u64),
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::ipc::{
        BluetoothActivity, CapsLockActivity, ClaudeDoneActivity, ClaudeNeedsInputActivity,
        PowerActivity, ScreenshotActivity, VolumeActivity,
    };

    pub fn volume(level: u8) -> Activity {
        Activity::Volume(VolumeActivity {
            level,
            muted: false,
            device: "Speakers".into(),
        })
    }

    pub fn screenshot() -> Activity {
        Activity::Screenshot(ScreenshotActivity {
            file_name: "Screenshot.png".into(),
            thumbnail: String::new(),
            width: 256,
            height: 144,
            path: "C:\\Screenshot.png".into(),
        })
    }

    pub fn needs_input(session_id: u32) -> Activity {
        Activity::ClaudeNeedsInput(ClaudeNeedsInputActivity {
            session_id,
            project: "wpill".into(),
        })
    }

    pub fn done(session_id: u32) -> Activity {
        Activity::ClaudeDone(ClaudeDoneActivity {
            session_id,
            project: "wpill".into(),
            failed: false,
        })
    }

    fn bluetooth(container: &str) -> Activity {
        Activity::Bluetooth(BluetoothActivity {
            name: "Headphones".into(),
            connected: true,
            battery: None,
            container: container.into(),
        })
    }

    fn low_battery() -> Activity {
        Activity::Power(PowerActivity {
            event: PowerEvent::Low,
            level: 10,
        })
    }

    const HOUR: Duration = Duration::from_secs(3600);

    #[test]
    fn burst_updates_one_activity_in_place() {
        let t0 = Instant::now();
        let mut q = Queue::default();
        let first = q.offer(volume(10), t0, true).unwrap();
        for i in 1..10 {
            let p = q
                .offer(volume(10 + i), t0 + ms(50 * i as u64), true)
                .unwrap();
            assert_eq!(p.id, first.id);
        }
        assert!(q.waiting.is_empty());
        // The last press restarts the countdown.
        assert!(!q.is_over(t0 + ms(450) + ms(1400)));
        assert!(q.is_over(t0 + ms(450) + ms(1500)));
    }

    #[test]
    fn feedback_is_dropped_and_notices_wait_while_hidden() {
        let t0 = Instant::now();
        let mut q = Queue::default();
        assert!(q.offer(volume(5), t0, false).is_none());
        assert!(q.offer(screenshot(), t0, false).is_none());
        let next = q.take_next(t0 + ms(2000)).unwrap();
        assert!(matches!(next.activity, Activity::Screenshot(_)));
        assert_eq!(q.take_next(t0 + ms(2000)), None);
    }

    #[test]
    fn stale_notices_are_dropped() {
        let t0 = Instant::now();
        let mut q = Queue::default();
        q.offer(bluetooth("a"), t0, false);
        assert_eq!(q.take_next(t0 + ms(10_001)), None);
    }

    #[test]
    fn feedback_interrupts_a_notice_which_then_resumes() {
        let t0 = Instant::now();
        let mut q = Queue::default();
        let shot = q.offer(screenshot(), t0, true).unwrap();
        let vol = q.offer(volume(30), t0 + ms(5500), true).unwrap();
        assert_ne!(vol.id, shot.id);
        q.finish();
        let resumed = q.take_next(t0 + ms(7000)).unwrap();
        assert!(matches!(resumed.activity, Activity::Screenshot(_)));
        // Only 0.5 s was left; it gets MIN_RESUME.
        assert_eq!(resumed.duration_ms, Some(MIN_RESUME.as_millis() as u64));
    }

    #[test]
    fn notices_queue_behind_each_other_and_priority_wins() {
        let t0 = Instant::now();
        let mut q = Queue::default();
        q.offer(bluetooth("a"), t0, true).unwrap();
        assert!(q.offer(screenshot(), t0, true).is_none());
        // Low battery outranks the Bluetooth notice that's showing.
        let low = q.offer(low_battery(), t0, true).unwrap();
        assert!(matches!(low.activity, Activity::Power(_)));
        q.finish();
        // The interrupted Bluetooth notice is first in line; it ties with the screenshot and was there first.
        let next = q.take_next(t0 + ms(100)).unwrap();
        assert!(matches!(next.activity, Activity::Bluetooth(_)));
    }

    #[test]
    fn different_devices_do_not_coalesce() {
        let t0 = Instant::now();
        let mut q = Queue::default();
        q.offer(bluetooth("a"), t0, true).unwrap();
        assert!(q.offer(bluetooth("b"), t0, true).is_none());
        assert!(q.offer(bluetooth("b"), t0, true).is_none());
        assert_eq!(q.waiting.len(), 1);
    }

    #[test]
    fn feedback_replaces_feedback() {
        let t0 = Instant::now();
        let mut q = Queue::default();
        q.offer(volume(1), t0, true).unwrap();
        let caps = q
            .offer(Activity::CapsLock(CapsLockActivity { on: true }), t0, true)
            .unwrap();
        assert!(matches!(caps.activity, Activity::CapsLock(_)));
        q.finish();
        assert_eq!(q.take_next(t0), None);
    }

    #[test]
    fn hold_extends_and_only_screenshots_and_claude_are_interactive() {
        let t0 = Instant::now();
        let mut q = Queue::default();
        q.offer(screenshot(), t0, true).unwrap();
        assert!(q.current_interactive());
        q.hold(t0 + ms(5900));
        assert!(!q.is_over(t0 + ms(6500)));
        assert!(q.is_over(t0 + ms(6900)));
        assert!(!interactive(&volume(1)));
        assert!(interactive(&done(1)));
        assert!(interactive(&needs_input(1)));
    }

    #[test]
    fn payload_serializes_as_kind_and_payload() {
        let mut q = Queue::default();
        let p = q
            .offer(
                Activity::CapsLock(CapsLockActivity { on: true }),
                Instant::now(),
                true,
            )
            .unwrap();
        let json = serde_json::to_value(&p).unwrap();
        assert_eq!(json["kind"], "capsLock");
        assert_eq!(json["payload"]["on"], true);
        assert_eq!(json["durationMs"], 1200);
        let shot = serde_json::to_value(screenshot()).unwrap();
        assert!(shot["payload"].get("path").is_none());
        assert_eq!(shot["payload"]["fileName"], "Screenshot.png");
    }

    #[test]
    fn needs_input_stays_until_retracted_and_serializes_without_a_duration() {
        let t0 = Instant::now();
        let mut q = Queue::default();
        let p = q.offer(needs_input(7), t0, true).unwrap();
        let json = serde_json::to_value(&p).unwrap();
        assert_eq!(json["kind"], "claudeNeedsInput");
        assert_eq!(json["payload"]["sessionId"], 7);
        assert!(json["durationMs"].is_null());
        q.hold(t0 + HOUR);
        assert!(!q.is_over(t0 + 24 * HOUR));
        assert_eq!(q.retract(&claude_key(8)), None);
        assert_eq!(q.retract(&claude_key(7)), Some(p.id));
        assert_eq!(q.take_next(t0 + HOUR), None);
    }

    #[test]
    fn done_replaces_needs_input_in_place_and_then_times_out() {
        let t0 = Instant::now();
        let mut q = Queue::default();
        let asked = q.offer(needs_input(1), t0, true).unwrap();
        let finished = q.offer(done(1), t0 + HOUR, true).unwrap();
        assert_eq!(finished.id, asked.id);
        assert_eq!(finished.duration_ms, Some(5000));
        assert!(q.is_over(t0 + HOUR + ms(5000)));
        // And back: a new question makes it sticky again.
        q.offer(needs_input(1), t0 + HOUR + ms(100), true).unwrap();
        assert!(!q.is_over(t0 + 3 * HOUR));
    }

    #[test]
    fn needs_input_steps_aside_for_notices_and_resumes_after_them() {
        let t0 = Instant::now();
        let mut q = Queue::default();
        let asked = q.offer(needs_input(1), t0, true).unwrap();
        let shot = q.offer(screenshot(), t0 + ms(100), true).unwrap();
        assert_ne!(shot.id, asked.id);
        assert!(q.is_over(t0 + ms(6100)));
        q.finish();
        let back = q.take_next(t0 + ms(6100)).unwrap();
        assert!(matches!(back.activity, Activity::ClaudeNeedsInput(_)));
        assert_eq!(back.duration_ms, None);
        // Feedback interrupts it too, and it comes back sticky.
        q.offer(volume(3), t0 + ms(7000), true).unwrap();
        q.finish();
        let again = q.take_next(t0 + ms(9000)).unwrap();
        assert_eq!(again.duration_ms, None);
    }

    #[test]
    fn needs_input_waits_for_the_notice_on_screen_and_never_goes_stale() {
        let t0 = Instant::now();
        let mut q = Queue::default();
        q.offer(screenshot(), t0, true).unwrap();
        assert!(q.offer(needs_input(1), t0, true).is_none());
        q.finish();
        let next = q.take_next(t0 + 10 * HOUR).unwrap();
        assert!(matches!(next.activity, Activity::ClaudeNeedsInput(_)));
    }

    #[test]
    fn waiting_notices_show_before_a_sticky_one_and_sessions_take_turns() {
        let t0 = Instant::now();
        let mut q = Queue::default();
        q.offer(needs_input(1), t0, false);
        q.offer(needs_input(2), t0 + ms(10), false);
        q.offer(bluetooth("a"), t0 + ms(20), false);
        let first = q.take_next(t0 + ms(30)).unwrap();
        assert!(matches!(first.activity, Activity::Bluetooth(_)));
        q.finish();
        let older = q.take_next(t0 + ms(40)).unwrap();
        assert_eq!(older.activity, needs_input(1));
        // A second session's question doesn't push the first one aside.
        assert!(q.offer(needs_input(3), t0 + ms(50), true).is_none());
        q.finish();
        assert_eq!(q.take_next(t0 + ms(60)).unwrap().activity, needs_input(2));
    }

    #[test]
    fn parking_keeps_a_sticky_notice_and_drops_the_rest() {
        let t0 = Instant::now();
        let mut q = Queue::default();
        let asked = q.offer(needs_input(1), t0, true).unwrap();
        assert_eq!(q.park(t0 + ms(10)), Some(asked.id));
        let back = q.take_next(t0 + HOUR).unwrap();
        assert_eq!(back.activity, needs_input(1));
        assert_ne!(back.id, asked.id);
        q.finish();
        q.offer(done(1), t0 + HOUR, true).unwrap();
        q.park(t0 + HOUR);
        assert_eq!(q.take_next(t0 + HOUR), None);
    }

    #[test]
    fn retract_also_clears_a_waiting_notice() {
        let t0 = Instant::now();
        let mut q = Queue::default();
        q.offer(needs_input(1), t0, false);
        q.offer(done(2), t0, false);
        assert_eq!(q.retract(&claude_key(1)), None);
        assert_eq!(q.take_next(t0).unwrap().activity, done(2));
    }
}
