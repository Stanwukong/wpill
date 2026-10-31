//! The pure parts of the desktop tools: image sizes and budgets, the glow's timing, and the errors Claude sees.
//! Unit-tested.

use std::time::{Duration, Instant};

use serde::Serialize;

/// Longest image edge sent to Claude. The API shrinks anything larger for most models (and Claude Code to
/// 2000 px), so bigger only costs bytes.
pub const MAX_EDGE: u32 = 1568;
/// Claude Code re-compresses images over 512,000 bytes itself; staying under keeps what we send what it sees.
pub const MAX_IMAGE_BYTES: usize = 500_000;
/// Never send more than this (the agent pipe drops messages over 4 MiB, and base64 adds a third).
pub const HARD_MAX_IMAGE_BYTES: usize = 3 * 1024 * 1024;

/// A rectangle in physical screen px.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    pub fn from_ltrb(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self {
            x: left,
            y: top,
            width: (right - left).max(0),
            height: (bottom - top).max(0),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.width <= 0 || self.height <= 0
    }

    /// The part of `self` inside `other`.
    pub fn intersect(&self, other: &Rect) -> Rect {
        Rect::from_ltrb(
            self.x.max(other.x),
            self.y.max(other.y),
            (self.x + self.width).min(other.x + other.width),
            (self.y + self.height).min(other.y + other.height),
        )
    }
}

/// `width`×`height` scaled down (never up) so the long edge is at most `max_edge`, keeping the aspect ratio.
pub fn fit(width: u32, height: u32, max_edge: u32) -> (u32, u32) {
    let long = width.max(height);
    if long <= max_edge || long == 0 {
        return (width.max(1), height.max(1));
    }
    let scale = f64::from(max_edge) / f64::from(long);
    let w = (f64::from(width) * scale).round().max(1.0) as u32;
    let h = (f64::from(height) * scale).round().max(1.0) as u32;
    (w.min(max_edge), h.min(max_edge))
}

/// How to encode a capture, tried in order until one fits `MAX_IMAGE_BYTES`: PNG keeps text crisp; JPEG is
/// for photos and video, where PNG gets large.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Encoding {
    Png,
    Jpeg(f32),
}

pub const ENCODINGS: &[Encoding] = &[Encoding::Png, Encoding::Jpeg(0.8), Encoding::Jpeg(0.6)];

impl Encoding {
    pub fn mime(self) -> &'static str {
        match self {
            Encoding::Png => "image/png",
            Encoding::Jpeg(_) => "image/jpeg",
        }
    }
}

/// Why a look failed, as Claude sees it: a stable code and a sentence saying what to do instead.
#[derive(Debug, Clone, PartialEq)]
pub enum LookError {
    WindowGone,
    TopbarWindow,
    NotOnThisDesktop,
    Minimized,
    NotResponding,
    Elevated,
    ScreenLocked,
    Failed(String),
}

impl LookError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::WindowGone => "windowGone",
            Self::TopbarWindow => "topbarWindow",
            Self::NotOnThisDesktop => "notOnThisDesktop",
            Self::Minimized => "minimized",
            Self::NotResponding => "notResponding",
            Self::Elevated => "elevated",
            Self::ScreenLocked => "screenLocked",
            Self::Failed(_) => "failed",
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::WindowGone => {
                "That window has closed (or the hwnd is stale). Call list_windows again.".into()
            }
            Self::TopbarWindow => {
                "That's one of TopBar's own windows, which can't be looked at.".into()
            }
            Self::NotOnThisDesktop => {
                "That window isn't on the current virtual desktop, so it can't be looked at.".into()
            }
            Self::Minimized => {
                "That window is minimized, so there's nothing to capture. You can't \
                                restore it; ask the user to, or use ui_tree."
                    .into()
            }
            Self::NotResponding => "That window isn't responding. Try again later.".into(),
            Self::Elevated => {
                "That window belongs to an app running as administrator, which TopBar \
                               can't read (Windows blocks it)."
                    .into()
            }
            Self::ScreenLocked => {
                "The screen is locked or a Windows security prompt is showing, so \
                                   nothing can be looked at."
                    .into()
            }
            Self::Failed(why) => format!("The look failed: {why}"),
        }
    }
}

/// When the screen-edge glow shows: from the start of a look until `HOLD` after the last one ends, so even a
/// 50 ms look is visible.
pub const GLOW_HOLD: Duration = Duration::from_millis(1500);
/// While acting (2.7d) the hold is short: you shouldn't be told "hands off" once you have control back.
pub const ACT_GLOW_HOLD: Duration = Duration::from_millis(600);

#[derive(Debug)]
pub struct GlowState {
    active: u32,
    hold_until: Option<Instant>,
    hold: Duration,
}

impl Default for GlowState {
    fn default() -> Self {
        Self::with_hold(GLOW_HOLD)
    }
}

impl GlowState {
    pub fn with_hold(hold: Duration) -> Self {
        Self {
            active: 0,
            hold_until: None,
            hold,
        }
    }

    pub fn start(&mut self) {
        self.active += 1;
        self.hold_until = None;
    }

    pub fn end(&mut self, now: Instant) {
        self.active = self.active.saturating_sub(1);
        if self.active == 0 {
            self.hold_until = Some(now + self.hold);
        }
    }

    /// Whether the glow should be up at `now`.
    pub fn visible(&self, now: Instant) -> bool {
        self.active > 0 || self.hold_until.is_some_and(|t| now < t)
    }

    /// When it's next worth checking `visible` again (the end of the hold), if it's holding.
    pub fn next_change(&self, now: Instant) -> Option<Duration> {
        match self.hold_until {
            Some(t) if self.active == 0 && t > now => Some(t - now),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fitting() {
        assert_eq!(fit(1920, 1080, MAX_EDGE), (1568, 882));
        assert_eq!(fit(1080, 1920, MAX_EDGE), (882, 1568));
        assert_eq!(fit(800, 600, MAX_EDGE), (800, 600));
        assert_eq!(fit(3840, 2160, MAX_EDGE), (1568, 882));
        assert_eq!(fit(10_000, 3, MAX_EDGE), (1568, 1));
        assert_eq!(fit(0, 0, MAX_EDGE), (1, 1));
    }

    #[test]
    fn rects() {
        let a = Rect::from_ltrb(0, 0, 100, 50);
        assert_eq!(a.width, 100);
        assert!(Rect::from_ltrb(10, 10, 5, 5).is_empty());
        let b = Rect::from_ltrb(-20, 20, 40, 200);
        assert_eq!(a.intersect(&b), Rect::from_ltrb(0, 20, 40, 50));
        assert!(a.intersect(&Rect::from_ltrb(200, 0, 300, 10)).is_empty());
    }

    #[test]
    fn errors_have_codes_and_advice() {
        assert_eq!(LookError::WindowGone.code(), "windowGone");
        assert!(LookError::WindowGone.message().contains("list_windows"));
        assert!(LookError::Failed("x".into()).message().ends_with("x"));
        assert_eq!(ENCODINGS[0].mime(), "image/png");
        assert_eq!(ENCODINGS[1].mime(), "image/jpeg");
    }

    #[test]
    fn glow_holds_after_the_last_look() {
        let now = Instant::now();
        let mut g = GlowState::default();
        assert!(!g.visible(now));
        g.start();
        g.start();
        g.end(now);
        assert!(
            g.visible(now + Duration::from_secs(60)),
            "one look still running"
        );
        g.end(now);
        assert!(g.visible(now + Duration::from_millis(1499)));
        assert!(!g.visible(now + GLOW_HOLD));
        assert_eq!(g.next_change(now), Some(GLOW_HOLD));
        g.start();
        assert!(g.next_change(now).is_none());
        g.end(now);
        g.end(now);
        assert!(!g.visible(now + GLOW_HOLD), "extra ends don't underflow");
        let mut act = GlowState::with_hold(ACT_GLOW_HOLD);
        act.start();
        act.end(now);
        assert!(act.visible(now + Duration::from_millis(599)));
        assert!(!act.visible(now + ACT_GLOW_HOLD));
    }
}
