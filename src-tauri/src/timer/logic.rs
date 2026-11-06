//! The timer's state and its rules (Phase 5a). Pure: every function takes "now" as wall-clock UTC ms, so the tests
//! drive a fake clock.
//!
//! Wall clock rather than `Instant` on purpose: a timer that ends while the PC sleeps must say when it really ended
//! ("Ended at 14:05"), and a running timer must survive a Cay restart (`timer.json`). The cost is that changing the
//! clock moves a running timer; a clock set *back* never makes elapsed time go backwards (segments clamp at zero).

use serde::{Deserialize, Serialize};

use crate::ipc::{TimerKind, TimerPhase, TimerState};

const MINUTE_MS: i64 = 60_000;
/// Custom countdowns: 1 minute to 3 hours.
pub const MAX_MINUTES: u32 = 180;
pub const FOCUS_MINUTES: u32 = 25;
pub const BREAK_MINUTES: u32 = 5;
/// An ended countdown restored after a restart is dropped once it ended this long ago.
const RESTORE_ENDED_FOR_MS: i64 = 60 * MINUTE_MS;
/// A stopwatch restored after a restart is dropped once it has run this long (it was surely forgotten).
const RESTORE_STOPWATCH_FOR_MS: i64 = 24 * 60 * MINUTE_MS;
/// An end seen this long after the deadline (the PC slept, Cay wasn't running) is "late": the island says when.
pub const LATE_AFTER_MS: i64 = 60_000;
/// The thread's longest sleep while a timer runs, so `mm:ss` changes every second.
const TICK_MS: i64 = 1000;

/// One timer: the elapsed time is `banked_ms` plus, while running, the time since `resumed_ms`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Timer {
    pub kind: TimerKind,
    pub started_ms: i64,
    /// Countdown kinds only; grows with +1 min.
    pub duration_ms: Option<i64>,
    pub banked_ms: i64,
    /// Running since (wall ms); `None` while paused or ended.
    pub resumed_ms: Option<i64>,
    /// When it reached its deadline (the deadline itself, not when Cay noticed).
    pub ended_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimerError {
    BadMinutes,
    NotRunning,
    NotPaused,
    NoDeadline,
}

impl std::fmt::Display for TimerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::BadMinutes => "A timer runs for 1 to 180 minutes.",
            Self::NotRunning => "The timer isn't running.",
            Self::NotPaused => "The timer isn't paused.",
            Self::NoDeadline => "A stopwatch has no time to add.",
        })
    }
}

impl Timer {
    /// A new timer, running from `now`. `minutes` only applies to `Countdown`; Focus and Break have fixed lengths.
    pub fn start(kind: TimerKind, minutes: Option<u32>, now: i64) -> Result<Self, TimerError> {
        let minutes = match kind {
            TimerKind::Stopwatch => None,
            TimerKind::Focus => Some(FOCUS_MINUTES),
            TimerKind::Break => Some(BREAK_MINUTES),
            TimerKind::Countdown => match minutes {
                Some(m @ 1..=MAX_MINUTES) => Some(m),
                _ => return Err(TimerError::BadMinutes),
            },
        };
        Ok(Self {
            kind,
            started_ms: now,
            duration_ms: minutes.map(|m| i64::from(m) * MINUTE_MS),
            banked_ms: 0,
            resumed_ms: Some(now),
            ended_ms: None,
        })
    }

    pub fn phase(&self) -> TimerPhase {
        if self.ended_ms.is_some() {
            TimerPhase::Ended
        } else if self.resumed_ms.is_some() {
            TimerPhase::Running
        } else {
            TimerPhase::Paused
        }
    }

    pub fn elapsed(&self, now: i64) -> i64 {
        let segment = self.resumed_ms.map_or(0, |r| (now - r).max(0));
        let elapsed = self.banked_ms + segment;
        match self.duration_ms {
            Some(d) => elapsed.min(d),
            None => elapsed,
        }
    }

    /// When a running countdown ends.
    pub fn deadline(&self) -> Option<i64> {
        Some(self.resumed_ms? + (self.duration_ms? - self.banked_ms).max(0))
    }

    pub fn pause(&mut self, now: i64) -> Result<(), TimerError> {
        if self.phase() != TimerPhase::Running {
            return Err(TimerError::NotRunning);
        }
        self.banked_ms = self.elapsed(now);
        self.resumed_ms = None;
        Ok(())
    }

    pub fn resume(&mut self, now: i64) -> Result<(), TimerError> {
        if self.phase() != TimerPhase::Paused {
            return Err(TimerError::NotPaused);
        }
        self.resumed_ms = Some(now);
        Ok(())
    }

    /// One more minute. On an ended countdown it runs again from `now` with a minute left.
    pub fn add_minute(&mut self, now: i64) -> Result<(), TimerError> {
        let duration = self.duration_ms.ok_or(TimerError::NoDeadline)?;
        if self.ended_ms.take().is_some() {
            self.banked_ms = duration;
            self.resumed_ms = Some(now);
        }
        self.duration_ms = Some(duration + MINUTE_MS);
        Ok(())
    }

    /// Ends a running countdown whose deadline has passed. Returns true when it ended just now.
    pub fn tick(&mut self, now: i64) -> bool {
        match self.deadline() {
            Some(deadline) if now >= deadline => {
                self.banked_ms = self.duration_ms.unwrap_or(self.banked_ms);
                self.resumed_ms = None;
                self.ended_ms = Some(deadline);
                true
            }
            _ => false,
        }
    }

    /// The ended timer's end was noticed late (sleep, or Cay wasn't running).
    pub fn late(&self, now: i64) -> bool {
        self.ended_ms.is_some_and(|e| now - e >= LATE_AFTER_MS)
    }

    /// How long the timer thread may sleep: until the next whole second of the display, or the deadline if sooner.
    /// `None` while nothing runs (the thread parks).
    pub fn next_wake_ms(&self, now: i64) -> Option<i64> {
        self.resumed_ms?;
        let to_second = TICK_MS - self.elapsed(now).rem_euclid(TICK_MS);
        let wait = match self.deadline() {
            Some(deadline) => to_second.min(deadline - now),
            None => to_second,
        };
        Some(wait.clamp(1, TICK_MS))
    }

    pub fn state(&self, now: i64) -> TimerState {
        TimerState {
            kind: self.kind,
            phase: self.phase(),
            started_ms: self.started_ms,
            duration_ms: self.duration_ms,
            elapsed_ms: self.elapsed(now),
            deadline_ms: self.deadline(),
            ended_at_ms: self.ended_ms,
            now_ms: now,
        }
    }

    /// Whether the numbers hold together (a hand-edited or damaged `timer.json` is dropped).
    fn sane(&self) -> bool {
        let duration_ok = match (self.kind, self.duration_ms) {
            (TimerKind::Stopwatch, None) => true,
            (TimerKind::Stopwatch, Some(_)) | (_, None) => false,
            (_, Some(d)) => d > 0 && d <= i64::from(MAX_MINUTES + 600) * MINUTE_MS,
        };
        duration_ok
            && self.started_ms > 0
            && self.banked_ms >= 0
            && self.duration_ms.is_none_or(|d| self.banked_ms <= d)
            && !(self.resumed_ms.is_some() && self.ended_ms.is_some())
    }
}

/// A timer read back from `timer.json` after a restart, brought up to `now`. Returns `None` for one to forget: a
/// damaged file, a countdown that ended over an hour ago, a stopwatch older than a day. A countdown that ended
/// while Cay wasn't running comes back ended (and late), so the island can say when.
pub fn restore(mut timer: Timer, now: i64) -> Option<Timer> {
    if !timer.sane() {
        return None;
    }
    timer.tick(now);
    if timer
        .ended_ms
        .is_some_and(|e| now - e > RESTORE_ENDED_FOR_MS)
    {
        return None;
    }
    if timer.kind == TimerKind::Stopwatch && timer.elapsed(now) > RESTORE_STOPWATCH_FOR_MS {
        return None;
    }
    Some(timer)
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_793_000_000_000;
    const S: i64 = 1000;
    const M: i64 = 60 * S;

    fn countdown(minutes: u32) -> Timer {
        Timer::start(TimerKind::Countdown, Some(minutes), T0).unwrap()
    }

    #[test]
    fn start_checks_minutes_and_fixes_focus_and_break() {
        assert_eq!(
            Timer::start(TimerKind::Countdown, None, T0),
            Err(TimerError::BadMinutes)
        );
        assert_eq!(
            Timer::start(TimerKind::Countdown, Some(0), T0),
            Err(TimerError::BadMinutes)
        );
        assert_eq!(
            Timer::start(TimerKind::Countdown, Some(181), T0),
            Err(TimerError::BadMinutes)
        );
        assert_eq!(countdown(180).duration_ms, Some(180 * M));
        let focus = Timer::start(TimerKind::Focus, Some(90), T0).unwrap();
        assert_eq!(focus.duration_ms, Some(25 * M), "focus ignores minutes");
        let rest = Timer::start(TimerKind::Break, None, T0).unwrap();
        assert_eq!(rest.duration_ms, Some(5 * M));
        let watch = Timer::start(TimerKind::Stopwatch, Some(5), T0).unwrap();
        assert_eq!(watch.duration_ms, None);
        assert_eq!(watch.phase(), TimerPhase::Running);
    }

    #[test]
    fn counts_down_and_ends_at_its_deadline() {
        let mut t = countdown(5);
        assert_eq!(t.deadline(), Some(T0 + 5 * M));
        assert!(!t.tick(T0 + 5 * M - 1));
        assert_eq!(t.elapsed(T0 + 2 * M), 2 * M);
        assert!(t.tick(T0 + 5 * M));
        assert_eq!(t.phase(), TimerPhase::Ended);
        assert_eq!(t.ended_ms, Some(T0 + 5 * M));
        assert!(!t.tick(T0 + 6 * M), "ends once");
        assert_eq!(t.elapsed(T0 + 9 * M), 5 * M);
        assert_eq!(t.deadline(), None);
    }

    #[test]
    fn a_deadline_slept_through_ends_at_the_deadline_and_is_late() {
        let mut t = countdown(10);
        let woke = T0 + 2 * 60 * M;
        assert!(t.tick(woke));
        assert_eq!(t.ended_ms, Some(T0 + 10 * M), "not when Cay woke");
        assert!(t.late(woke));
        let mut on_time = countdown(10);
        on_time.tick(T0 + 10 * M + 400);
        assert!(!on_time.late(T0 + 10 * M + 400));
    }

    #[test]
    fn pause_and_resume_move_the_deadline() {
        let mut t = countdown(5);
        t.pause(T0 + M).unwrap();
        assert_eq!(t.phase(), TimerPhase::Paused);
        assert_eq!(t.deadline(), None);
        assert_eq!(t.elapsed(T0 + 30 * M), M, "paused time doesn't count");
        assert!(!t.tick(T0 + 30 * M));
        assert_eq!(t.pause(T0 + 30 * M), Err(TimerError::NotRunning));
        t.resume(T0 + 30 * M).unwrap();
        assert_eq!(t.resume(T0 + 31 * M), Err(TimerError::NotPaused));
        assert_eq!(t.deadline(), Some(T0 + 34 * M));
    }

    #[test]
    fn a_clock_set_back_never_runs_time_backwards() {
        let mut t = countdown(5);
        assert_eq!(t.elapsed(T0 - 10 * M), 0);
        assert!(!t.tick(T0 - 10 * M));
        t.pause(T0 - 10 * M).unwrap();
        assert_eq!(t.banked_ms, 0);
        let mut watch = Timer::start(TimerKind::Stopwatch, None, T0).unwrap();
        watch.pause(T0 + 3 * M).unwrap();
        watch.resume(T0 + 4 * M).unwrap();
        assert_eq!(
            watch.elapsed(T0),
            3 * M,
            "the segment clamps, the bank stays"
        );
    }

    #[test]
    fn plus_a_minute_running_paused_and_after_the_end() {
        let mut t = countdown(1);
        t.add_minute(T0 + 30 * S).unwrap();
        assert_eq!(t.deadline(), Some(T0 + 2 * M));
        t.tick(T0 + 2 * M);
        t.add_minute(T0 + 3 * M).unwrap();
        assert_eq!(t.phase(), TimerPhase::Running);
        assert_eq!(t.elapsed(T0 + 3 * M), 2 * M);
        assert_eq!(t.deadline(), Some(T0 + 4 * M), "a minute from now");
        let mut watch = Timer::start(TimerKind::Stopwatch, None, T0).unwrap();
        assert_eq!(watch.add_minute(T0), Err(TimerError::NoDeadline));
    }

    #[test]
    fn a_stopwatch_never_ends() {
        let mut watch = Timer::start(TimerKind::Stopwatch, None, T0).unwrap();
        assert!(!watch.tick(T0 + 10 * 60 * M));
        assert_eq!(watch.elapsed(T0 + 90 * M), 90 * M);
        assert_eq!(watch.deadline(), None);
    }

    #[test]
    fn wakes_on_the_second_and_at_the_deadline_and_parks_otherwise() {
        let t = countdown(1);
        assert_eq!(t.next_wake_ms(T0), Some(1000));
        assert_eq!(t.next_wake_ms(T0 + 250), Some(750));
        assert_eq!(t.next_wake_ms(T0 + M - 1), Some(1), "the deadline itself");
        assert_eq!(t.next_wake_ms(T0 + 2 * M), Some(1), "overdue: tick at once");
        let mut paused = countdown(1);
        paused.pause(T0 + 10).unwrap();
        assert_eq!(paused.next_wake_ms(T0 + 20), None);
        let mut ended = countdown(1);
        ended.tick(T0 + M);
        assert_eq!(ended.next_wake_ms(T0 + M), None);
    }

    #[test]
    fn the_payload_says_what_react_draws() {
        let t = countdown(5);
        let s = t.state(T0 + 90 * S);
        assert_eq!(s.phase, TimerPhase::Running);
        assert_eq!(s.elapsed_ms, 90 * S);
        assert_eq!(s.duration_ms, Some(5 * M));
        assert_eq!(s.deadline_ms, Some(T0 + 5 * M));
        assert_eq!(s.now_ms, T0 + 90 * S);
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json["kind"], "countdown");
        assert_eq!(json["phase"], "running");
        assert!(json.get("deadlineMs").is_some());
    }

    #[test]
    fn restore_after_a_restart() {
        let file = serde_json::to_string(&countdown(10)).unwrap();
        let back: Timer = serde_json::from_str(&file).unwrap();
        let running = restore(back.clone(), T0 + 4 * M).unwrap();
        assert_eq!(running.phase(), TimerPhase::Running);
        assert_eq!(running.deadline(), Some(T0 + 10 * M));

        let ended = restore(back.clone(), T0 + 30 * M).unwrap();
        assert_eq!(ended.phase(), TimerPhase::Ended, "ended while Cay was off");
        assert!(ended.late(T0 + 30 * M));
        assert_eq!(
            restore(back, T0 + 2 * 60 * M),
            None,
            "ended over an hour ago"
        );

        let watch = Timer::start(TimerKind::Stopwatch, None, T0).unwrap();
        assert!(restore(watch.clone(), T0 + 60 * M).is_some());
        assert_eq!(
            restore(watch, T0 + 25 * 60 * M),
            None,
            "a forgotten stopwatch"
        );

        let mut paused = countdown(10);
        paused.pause(T0 + M).unwrap();
        let paused = restore(paused, T0 + 50 * 60 * M).unwrap();
        assert_eq!(paused.phase(), TimerPhase::Paused, "a paused timer waits");
    }

    #[test]
    fn restore_drops_a_damaged_file() {
        let mut bad = countdown(5);
        bad.banked_ms = -1;
        assert_eq!(restore(bad, T0), None);
        let mut bad = countdown(5);
        bad.duration_ms = None;
        assert_eq!(restore(bad, T0), None);
        let mut bad = Timer::start(TimerKind::Stopwatch, None, T0).unwrap();
        bad.duration_ms = Some(M);
        assert_eq!(restore(bad, T0), None);
        let mut bad = countdown(5);
        bad.ended_ms = Some(T0);
        assert_eq!(restore(bad, T0), None, "running and ended at once");
        let mut bad = countdown(5);
        bad.banked_ms = 6 * M;
        assert_eq!(restore(bad, T0), None);
        assert!(serde_json::from_str::<Timer>(r#"{"kind":"egg"}"#).is_err());
    }
}
