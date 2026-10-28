//! Pure hover logic: hit tests and the expand/collapse state machine. No Win32 here, so it's unit-tested.

use std::time::{Duration, Instant};

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
    }
}

pub fn force_collapse(phase: Phase) -> (Phase, Option<Transition>) {
    let transition = phase.is_expanded().then_some(Transition::Collapse);
    (Phase::Suppressed, transition)
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

    fn sample(now: Instant, in_hot_zone: bool, in_panel: bool) -> Sample {
        Sample {
            now,
            in_hot_zone,
            in_panel,
        }
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
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
