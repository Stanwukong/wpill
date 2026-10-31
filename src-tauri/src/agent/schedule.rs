//! When routines are due (2.7e). Pure; unit-tested with fake time zones.
//!
//! Times are Unix ms. Local times go through a `Zone` (minutes east of UTC at an instant), so DST is a property
//! of the zone: a local time skipped when clocks go forward runs at the first minute that exists after it, and a
//! local time that happens twice when they go back runs the first time only. `every` counts real hours from its
//! anchor, so DST doesn't shift it.
//!
//! The scheduler thread asks `check` about each routine: the latest due time after the last one handled is run
//! on time (within two minutes), or, when it was missed (TopBar closed, the PC asleep), run once after things
//! settle or skipped. Only the latest missed time counts, so there's never a burst of catch-up runs.

use crate::ipc::{IfMissed, Schedule, SkipReason, Weekday};

pub const MINUTE: i64 = 60_000;
pub const HOUR: i64 = 60 * MINUTE;
pub const DAY: i64 = 24 * HOUR;
/// A due time this recent is on time; older ones were missed.
pub const ON_TIME: i64 = 2 * MINUTE;
/// A missed due time older than this is skipped even with `ifMissed: once`.
pub const CATCH_UP_WITHIN: i64 = DAY;
/// Catch-up runs wait this long after TopBar starts or the PC wakes.
pub const SETTLE: i64 = 2 * MINUTE;
/// `start` routines run this long after TopBar starts.
pub const START_DELAY: i64 = 2 * MINUTE;
pub const MIN_HOURS: u32 = 1;
pub const MAX_HOURS: u32 = 24;
/// Scheduled runs going at once, at most.
pub const MAX_SCHEDULED: usize = 2;

/// A time zone: its offset from UTC at an instant.
pub trait Zone {
    /// Minutes east of UTC at `utc_ms`.
    fn offset_min(&self, utc_ms: i64) -> i64;
}

/// `HH:MM` → minutes after midnight.
pub fn parse_time(text: &str) -> Option<i64> {
    let (h, m) = text.trim().split_once(':')?;
    let (h, m): (i64, i64) = (h.trim().parse().ok()?, m.trim().parse().ok()?);
    ((0..24).contains(&h) && (0..60).contains(&m) && text.trim().len() <= 5).then_some(h * 60 + m)
}

/// Checks a schedule's fields; `Err` says what's wrong.
pub fn validate(s: &Schedule) -> Result<(), String> {
    match s {
        Schedule::Off | Schedule::Start => Ok(()),
        Schedule::Days { time, days } => {
            parse_time(time).ok_or("Pick a time like 09:00.")?;
            if days.is_empty() {
                return Err("Pick at least one day.".into());
            }
            Ok(())
        }
        Schedule::Every { hours, from } => {
            parse_time(from).ok_or("Pick a start time like 08:00.")?;
            if !(MIN_HOURS..=MAX_HOURS).contains(hours) {
                return Err(format!("Every {MIN_HOURS} to {MAX_HOURS} hours."));
            }
            Ok(())
        }
    }
}

/// Days since 1970-01-01 in local time.
fn local_day(zone: &dyn Zone, utc_ms: i64) -> i64 {
    (utc_ms + zone.offset_min(utc_ms) * MINUTE).div_euclid(DAY)
}

fn weekday(day: i64) -> Weekday {
    // 1970-01-01 was a Thursday.
    match (day + 3).rem_euclid(7) {
        0 => Weekday::Mon,
        1 => Weekday::Tue,
        2 => Weekday::Wed,
        3 => Weekday::Thu,
        4 => Weekday::Fri,
        5 => Weekday::Sat,
        _ => Weekday::Sun,
    }
}

/// The instant local `day` at `minute` after midnight happens: the first time if it happens twice, the first
/// minute that exists after it if it doesn't exist.
fn at_local(zone: &dyn Zone, day: i64, minute: i64) -> i64 {
    let naive = day * DAY + minute * MINUTE;
    // At most one transition within a day either side.
    let before = zone.offset_min(naive - DAY);
    let after = zone.offset_min(naive + DAY);
    let mut valid: Vec<i64> = [before, after]
        .into_iter()
        .map(|o| naive - o * MINUTE)
        .filter(|&u| zone.offset_min(u) * MINUTE == naive - u)
        .collect();
    valid.sort_unstable();
    if let Some(first) = valid.first() {
        return *first;
    }
    // A gap: the transition is between the two readings; find the minute the offset changes.
    let (mut lo, mut hi) = (
        naive - before.max(after) * MINUTE,
        naive - before.min(after) * MINUTE,
    );
    let start = zone.offset_min(lo);
    while hi - lo > MINUTE {
        let mid = lo + (hi - lo) / 2 / MINUTE * MINUTE;
        let mid = if mid == lo { lo + MINUTE } else { mid };
        if zone.offset_min(mid) == start {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    hi
}

/// The routine's times, as the scheduler needs them.
#[derive(Debug, Clone, Copy)]
pub struct Times {
    /// When the schedule was set or turned on: earlier due times don't count, and `every` counts from its day.
    pub since_ms: i64,
    /// When TopBar started (for `start`).
    pub launched_ms: i64,
}

/// The latest due time in `(after_ms, now]`.
pub fn latest(s: &Schedule, t: Times, after_ms: i64, now: i64, zone: &dyn Zone) -> Option<i64> {
    let found = match s {
        Schedule::Off => None,
        Schedule::Start => Some(t.launched_ms + START_DELAY).filter(|&d| d <= now),
        Schedule::Days { time, days } => {
            let minute = parse_time(time)?;
            let today = local_day(zone, now);
            (0..=7)
                .map(|back| today - back)
                .filter(|&d| days.contains(&weekday(d)))
                .map(|d| at_local(zone, d, minute))
                .find(|&at| at <= now)
        }
        Schedule::Every { hours, from } => {
            let period = i64::from((*hours).clamp(MIN_HOURS, MAX_HOURS)) * HOUR;
            let anchor = at_local(zone, local_day(zone, t.since_ms), parse_time(from)?);
            Some(anchor + (now - anchor).div_euclid(period) * period)
        }
    }?;
    (found > after_ms && found > t.since_ms).then_some(found)
}

/// The first due time after `now`.
pub fn next(s: &Schedule, t: Times, now: i64, zone: &dyn Zone) -> Option<i64> {
    match s {
        Schedule::Off => None,
        Schedule::Start => Some(t.launched_ms + START_DELAY).filter(|&d| d > now),
        Schedule::Days { time, days } => {
            let minute = parse_time(time)?;
            let today = local_day(zone, now);
            (0..=8)
                .map(|ahead| today + ahead)
                .filter(|&d| days.contains(&weekday(d)))
                .map(|d| at_local(zone, d, minute))
                .find(|&at| at > now)
        }
        Schedule::Every { hours, from } => {
            let period = i64::from((*hours).clamp(MIN_HOURS, MAX_HOURS)) * HOUR;
            let anchor = at_local(zone, local_day(zone, t.since_ms), parse_time(from)?);
            Some(anchor + ((now - anchor).div_euclid(period) + 1) * period)
        }
    }
}

/// What the scheduler knows about one routine.
#[derive(Debug, Clone)]
pub struct Check<'a> {
    pub schedule: &'a Schedule,
    pub times: Times,
    pub enabled: bool,
    pub if_missed: IfMissed,
    pub last_due_ms: i64,
    /// Catch-up runs wait until then (TopBar's start or the PC's wake, plus `SETTLE`).
    pub settle_until_ms: i64,
    pub paused: bool,
    /// Its previous run is still going.
    pub running: bool,
    /// `MAX_SCHEDULED` scheduled runs are going.
    pub at_limit: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Verdict {
    Idle,
    /// Start it; `late` when it's a catch-up.
    Fire {
        due_ms: i64,
        late: bool,
    },
    Skip {
        due_ms: i64,
        why: SkipReason,
    },
    /// A missed run waits for things to settle; ask again then.
    Hold {
        until_ms: i64,
    },
}

pub fn check(c: &Check<'_>, now: i64, zone: &dyn Zone) -> Verdict {
    if !c.enabled {
        return Verdict::Idle;
    }
    let Some(due_ms) = latest(c.schedule, c.times, c.last_due_ms, now, zone) else {
        return Verdict::Idle;
    };
    let late = !matches!(c.schedule, Schedule::Start) && now - due_ms > ON_TIME;
    let skip = |why| Verdict::Skip { due_ms, why };
    if c.paused {
        return skip(SkipReason::Paused);
    }
    if late {
        if c.if_missed == IfMissed::Skip || now - due_ms > CATCH_UP_WITHIN {
            return skip(SkipReason::Missed);
        }
        if now < c.settle_until_ms {
            return Verdict::Hold {
                until_ms: c.settle_until_ms,
            };
        }
    }
    if c.running {
        return skip(SkipReason::StillRunning);
    }
    if c.at_limit {
        return skip(SkipReason::TooMany);
    }
    Verdict::Fire { due_ms, late }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixed offset.
    struct Fixed(i64);
    impl Zone for Fixed {
        fn offset_min(&self, _: i64) -> i64 {
            self.0
        }
    }

    /// +60 normally, +120 from `start` until `end` (UTC instants): a European-style summer time.
    struct Summer {
        start: i64,
        end: i64,
    }
    impl Zone for Summer {
        fn offset_min(&self, utc: i64) -> i64 {
            if (self.start..self.end).contains(&utc) {
                120
            } else {
                60
            }
        }
    }

    /// 2026-03-29 (a Sunday) as days since 1970: EU clocks go forward at 01:00 UTC; back 2026-10-25 01:00 UTC.
    const MAR29: i64 = 20_541;
    const OCT25: i64 = 20_751;
    fn eu() -> Summer {
        Summer {
            start: MAR29 * DAY + HOUR,
            end: OCT25 * DAY + HOUR,
        }
    }

    fn utc(day: i64, h: i64, m: i64) -> i64 {
        day * DAY + h * HOUR + m * MINUTE
    }

    fn weekdays(time: &str) -> Schedule {
        use Weekday::*;
        Schedule::Days {
            time: time.into(),
            days: vec![Mon, Tue, Wed, Thu, Fri],
        }
    }

    fn times(since: i64) -> Times {
        Times {
            since_ms: since,
            launched_ms: since,
        }
    }

    #[test]
    fn times_and_weekdays() {
        assert_eq!(parse_time("09:05"), Some(545));
        assert_eq!(parse_time("9:05"), Some(545));
        assert_eq!(parse_time("24:00"), None);
        assert_eq!(parse_time("09:60"), None);
        assert_eq!(parse_time("nine"), None);
        assert_eq!(weekday(0), Weekday::Thu);
        assert_eq!(weekday(MAR29), Weekday::Sun);
        assert!(validate(&weekdays("09:00")).is_ok());
        assert!(validate(&Schedule::Days {
            time: "09:00".into(),
            days: vec![]
        })
        .is_err());
        assert!(validate(&Schedule::Every {
            hours: 0,
            from: "08:00".into()
        })
        .is_err());
        assert!(validate(&Schedule::Every {
            hours: 25,
            from: "08:00".into()
        })
        .is_err());
    }

    #[test]
    fn days_at_a_time_in_local_time() {
        let z = Fixed(60);
        let s = weekdays("09:00");
        // Friday 2026-10-30, 07:00 UTC = 08:00 local: next is 09:00 local = 08:00 UTC today.
        let fri = 20_756;
        assert_eq!(weekday(fri), Weekday::Fri);
        let now = utc(fri, 7, 0);
        assert_eq!(next(&s, times(0), now, &z), Some(utc(fri, 8, 0)));
        // After it: Monday's.
        assert_eq!(
            next(&s, times(0), utc(fri, 8, 0), &z),
            Some(utc(fri + 3, 8, 0))
        );
        // The latest before Friday 07:00 UTC was Thursday's.
        assert_eq!(latest(&s, times(0), 0, now, &z), Some(utc(fri - 1, 8, 0)));
        // Not before the schedule was set, and not one already handled.
        assert_eq!(latest(&s, times(utc(fri - 1, 9, 0)), 0, now, &z), None);
        assert_eq!(latest(&s, times(0), utc(fri - 1, 8, 0), now, &z), None);
    }

    #[test]
    fn dst_gap_and_repeat() {
        let z = eu();
        // 02:30 local doesn't exist on 2026-03-29 (02:00 → 03:00): it runs at 03:00 local = 01:00 UTC.
        assert_eq!(at_local(&z, MAR29, 150), utc(MAR29, 1, 0));
        // 09:00 that day is summer time: 07:00 UTC.
        assert_eq!(at_local(&z, MAR29, 540), utc(MAR29, 7, 0));
        // 02:30 happens twice on 2026-10-25 (03:00 → 02:00): the first, still summer time, is 00:30 UTC.
        assert_eq!(at_local(&z, OCT25, 150), utc(OCT25, 0, 30));
        assert_eq!(at_local(&z, OCT25, 540), utc(OCT25, 8, 0));
        // A daily 02:30 routine fires once on the repeated night, not twice.
        let daily = Schedule::Days {
            time: "02:30".into(),
            days: vec![Weekday::Sun],
        };
        let first = utc(OCT25, 0, 30);
        assert_eq!(
            next(&daily, times(0), utc(OCT25 - 1, 12, 0), &z),
            Some(first)
        );
        assert_eq!(latest(&daily, times(0), first, utc(OCT25, 1, 30), &z), None);
        assert_eq!(
            next(&daily, times(0), first, &z),
            Some(utc(OCT25 + 7, 1, 30))
        );
    }

    #[test]
    fn every_counts_real_hours() {
        let z = eu();
        let s = Schedule::Every {
            hours: 4,
            from: "08:00".into(),
        };
        // Saved on 2026-03-28 at noon UTC; anchor 08:00 local = 07:00 UTC that day.
        let since = utc(MAR29 - 1, 12, 0);
        let t = times(since);
        assert_eq!(next(&s, t, since, &z), Some(utc(MAR29 - 1, 15, 0)));
        // Across the clock change it stays 4 real hours apart.
        assert_eq!(next(&s, t, utc(MAR29, 2, 0), &z), Some(utc(MAR29, 3, 0)));
        assert_eq!(
            latest(&s, t, 0, utc(MAR29, 2, 0), &z),
            Some(utc(MAR29 - 1, 23, 0))
        );
        // Nothing before it was saved.
        assert_eq!(latest(&s, t, 0, since, &z), None);
    }

    fn check_at(c: &Check<'_>, now: i64) -> Verdict {
        check(c, now, &Fixed(0))
    }

    #[test]
    fn on_time_late_and_gates() {
        let s = Schedule::Days {
            time: "09:00".into(),
            days: vec![
                Weekday::Mon,
                Weekday::Tue,
                Weekday::Wed,
                Weekday::Thu,
                Weekday::Fri,
                Weekday::Sat,
                Weekday::Sun,
            ],
        };
        let day = 20_756;
        let due = utc(day, 9, 0);
        let base = Check {
            schedule: &s,
            times: times(utc(day - 3, 0, 0)),
            enabled: true,
            if_missed: IfMissed::Once,
            last_due_ms: utc(day - 1, 9, 0),
            settle_until_ms: utc(day - 3, 0, 2),
            paused: false,
            running: false,
            at_limit: false,
        };
        assert_eq!(check_at(&base, due - MINUTE), Verdict::Idle);
        assert_eq!(
            check_at(&base, due + 30_000),
            Verdict::Fire {
                due_ms: due,
                late: false
            }
        );
        // TopBar was closed from 08:00 to 13:00: one catch-up once it has settled, for the latest due time only.
        let reopened = Check {
            settle_until_ms: utc(day, 13, 2),
            last_due_ms: utc(day - 3, 9, 0),
            ..base.clone()
        };
        assert_eq!(
            check_at(&reopened, utc(day, 13, 0)),
            Verdict::Hold {
                until_ms: utc(day, 13, 2)
            }
        );
        assert_eq!(
            check_at(&reopened, utc(day, 13, 2)),
            Verdict::Fire {
                due_ms: due,
                late: true
            }
        );
        // Then nothing until tomorrow: no burst for the two earlier missed days.
        let after = Check {
            last_due_ms: due,
            ..reopened.clone()
        };
        assert_eq!(check_at(&after, utc(day, 13, 3)), Verdict::Idle);
        // Set to skip, or missed by more than a day.
        let skip = Check {
            if_missed: IfMissed::Skip,
            ..reopened.clone()
        };
        assert_eq!(
            check_at(&skip, utc(day, 13, 2)),
            Verdict::Skip {
                due_ms: due,
                why: SkipReason::Missed
            }
        );
        let fridays = Schedule::Days {
            time: "09:00".into(),
            days: vec![Weekday::Fri],
        };
        let old = Check {
            schedule: &fridays,
            last_due_ms: utc(day - 7, 9, 0),
            settle_until_ms: 0,
            ..base.clone()
        };
        assert_eq!(
            check_at(&old, utc(day + 2, 10, 0)),
            Verdict::Skip {
                due_ms: due,
                why: SkipReason::Missed
            }
        );
        // Gates.
        for (c, why) in [
            (
                Check {
                    paused: true,
                    ..base.clone()
                },
                SkipReason::Paused,
            ),
            (
                Check {
                    running: true,
                    ..base.clone()
                },
                SkipReason::StillRunning,
            ),
            (
                Check {
                    at_limit: true,
                    ..base.clone()
                },
                SkipReason::TooMany,
            ),
        ] {
            assert_eq!(check_at(&c, due + 1000), Verdict::Skip { due_ms: due, why });
        }
        assert_eq!(
            check_at(
                &Check {
                    enabled: false,
                    ..base.clone()
                },
                due + 1000
            ),
            Verdict::Idle
        );
    }

    #[test]
    fn at_start_runs_once_per_launch() {
        let s = Schedule::Start;
        let launched = utc(20_756, 8, 0);
        let c = Check {
            schedule: &s,
            times: Times {
                since_ms: launched - DAY,
                launched_ms: launched,
            },
            enabled: true,
            if_missed: IfMissed::Skip,
            last_due_ms: launched - DAY,
            settle_until_ms: launched + SETTLE,
            paused: false,
            running: false,
            at_limit: false,
        };
        assert_eq!(check_at(&c, launched + MINUTE), Verdict::Idle);
        let due = launched + START_DELAY;
        // Never "late", even if the thread checks a while after.
        assert_eq!(
            check_at(&c, due + 10 * MINUTE),
            Verdict::Fire {
                due_ms: due,
                late: false
            }
        );
        let done = Check {
            last_due_ms: due,
            ..c.clone()
        };
        assert_eq!(check_at(&done, due + HOUR), Verdict::Idle);
        assert_eq!(next(&s, done.times, due + HOUR, &Fixed(0)), None);
    }
}
