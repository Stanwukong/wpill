//! Saved routines (2.7e): `%APPDATA%\dev.wpill.topbar\routines.json`, written by TopBar (not by hand, unlike
//! `settings.json`), replaced atomically. Validation is pure and unit-tested; the folder check is passed in.
//!
//! A routine run is an ordinary agent run. Nothing here changes what's approved: no allow list, no permission
//! mode; `desktopTools: false` only takes tools away.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
use windows::Win32::System::Time::{
    FileTimeToSystemTime, SystemTimeToFileTime, SystemTimeToTzSpecificLocalTime,
};

use super::audit::random_hex;
use super::schedule::{self, Zone};
use crate::ipc::{IfMissed, Routine, Schedule};

const VERSION: u32 = 1;
pub const MAX_ROUTINES: usize = 50;
const MAX_NAME: usize = 60;
const MAX_PROMPT: usize = 32 * 1024;
pub const MAX_TURNS: u32 = 100;
pub const MIN_BUDGET: f64 = 0.05;
pub const MAX_BUDGET: f64 = 20.0;

/// What the command window sends to `routine_save`: a new routine (no `id`) or an edit.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutineDraft {
    pub id: Option<String>,
    pub name: String,
    pub prompt: String,
    pub cwd: String,
    pub model: Option<String>,
    pub max_turns: u32,
    pub max_budget_usd: f64,
    pub claude_path: Option<String>,
    pub schedule: Schedule,
    pub enabled: bool,
    pub if_missed: IfMissed,
    pub wait_for_me: bool,
    pub desktop_tools: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct File {
    pub version: u32,
    pub paused: bool,
    pub routines: Vec<Routine>,
}

impl Default for File {
    fn default() -> Self {
        Self {
            version: VERSION,
            paused: false,
            routines: Vec::new(),
        }
    }
}

#[derive(Debug, Default)]
pub struct Store {
    path: Option<PathBuf>,
    pub file: File,
    /// The file on disk couldn't be read: it's moved aside before the first save, never overwritten.
    unreadable: bool,
}

impl Store {
    pub fn load(path: PathBuf) -> Self {
        let (file, unreadable) = match std::fs::read_to_string(&path) {
            Err(_) => (File::default(), false),
            Ok(text) => match serde_json::from_str::<File>(&text) {
                Ok(file) => (file, false),
                Err(e) => {
                    log::warn!("ignoring unreadable {}: {e}", path.display());
                    (File::default(), true)
                }
            },
        };
        log::info!("routines: {} saved", file.routines.len());
        Self {
            path: Some(path),
            file,
            unreadable,
        }
    }

    pub fn get(&self, id: &str) -> Option<&Routine> {
        self.file.routines.iter().find(|r| r.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Routine> {
        self.file.routines.iter_mut().find(|r| r.id == id)
    }

    /// Writes the file (to a temp file, then over the old one). Failures are logged.
    pub fn save(&mut self) {
        let Some(path) = &self.path else {
            return;
        };
        if self.unreadable {
            let aside = path.with_extension("unreadable.json");
            match std::fs::rename(path, &aside) {
                Ok(()) => log::warn!("kept the unreadable routines file as {}", aside.display()),
                Err(e) => log::warn!("moving the unreadable routines file aside failed: {e}"),
            }
            self.unreadable = false;
        }
        if let Err(e) = write(path, &self.file) {
            log::warn!("saving {} failed: {e}", path.display());
        }
    }

    /// Adds or replaces a checked routine. `Err` when there are too many.
    pub fn upsert(&mut self, routine: Routine) -> Result<(), String> {
        if let Some(old) = self.get_mut(&routine.id) {
            *old = routine;
        } else if self.file.routines.len() >= MAX_ROUTINES {
            return Err(format!("You can save up to {MAX_ROUTINES} routines."));
        } else {
            self.file.routines.push(routine);
        }
        Ok(())
    }

    pub fn delete(&mut self, id: &str) -> bool {
        let before = self.file.routines.len();
        self.file.routines.retain(|r| r.id != id);
        self.file.routines.len() != before
    }
}

fn write(path: &Path, file: &File) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = serde_json::to_string_pretty(file).map_err(std::io::Error::other)?;
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, text)?;
    // Replaces the old file (MoveFileExW with MOVEFILE_REPLACE_EXISTING).
    std::fs::rename(&temp, path)
}

/// Checks and normalises a draft into a routine. `folder` returns the long-path form of an existing folder.
/// Limits are clamped; a schedule that's new, changed or turned on starts counting from `now` (no run for a
/// time that already passed).
pub fn validate(
    draft: RoutineDraft,
    existing: Option<&Routine>,
    now: i64,
    folder: impl Fn(&str) -> Option<String>,
) -> Result<Routine, String> {
    let name = draft.name.trim().to_string();
    if name.is_empty() {
        return Err("Give the routine a name.".into());
    }
    if name.chars().count() > MAX_NAME {
        return Err(format!("Keep the name under {MAX_NAME} characters."));
    }
    let prompt = draft.prompt.trim().to_string();
    if prompt.is_empty() || prompt.len() > MAX_PROMPT {
        return Err("Type an instruction first.".into());
    }
    let cwd = folder(draft.cwd.trim()).ok_or("That folder doesn't exist.")?;
    if !(draft.max_budget_usd.is_finite() && draft.max_budget_usd > 0.0) {
        return Err("Routines need a budget.".into());
    }
    schedule::validate(&draft.schedule)?;
    let optional = |s: Option<String>| s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let restart = existing
        .is_none_or(|old| old.schedule != draft.schedule || (!old.enabled && draft.enabled));
    let (since, last_due) = match existing {
        Some(old) if !restart => (old.schedule_since_ms, old.last_due_ms),
        _ => (now, now),
    };
    Ok(Routine {
        id: existing.map_or_else(|| format!("rt-{}", random_hex(3)), |old| old.id.clone()),
        name,
        prompt,
        cwd,
        model: optional(draft.model),
        max_turns: draft.max_turns.clamp(1, MAX_TURNS),
        max_budget_usd: draft.max_budget_usd.clamp(MIN_BUDGET, MAX_BUDGET),
        claude_path: optional(draft.claude_path),
        schedule: draft.schedule,
        enabled: draft.enabled,
        if_missed: draft.if_missed,
        wait_for_me: draft.wait_for_me,
        desktop_tools: draft.desktop_tools,
        created_at_ms: existing.map_or(now, |old| old.created_at_ms),
        updated_at_ms: now,
        schedule_since_ms: since,
        last_due_ms: last_due,
        last_run: existing.and_then(|old| old.last_run.clone()),
        last_skip: existing
            .filter(|_| !restart)
            .and_then(|old| old.last_skip.clone()),
    })
}

/// Windows' current time zone, with its DST rules.
pub struct LocalZone;

/// 100 ns intervals between 1601-01-01 and 1970-01-01.
const EPOCH_DIFF_MS: i64 = 11_644_473_600_000;

impl Zone for LocalZone {
    fn offset_min(&self, utc_ms: i64) -> i64 {
        let ticks = (utc_ms + EPOCH_DIFF_MS) * 10_000;
        let utc = FILETIME {
            dwLowDateTime: ticks as u32,
            dwHighDateTime: (ticks >> 32) as u32,
        };
        let (mut st, mut local_st, mut local) = (
            SYSTEMTIME::default(),
            SYSTEMTIME::default(),
            FILETIME::default(),
        );
        // SAFETY: plain conversions between out-structs we own; `None` is the current time zone.
        let ok = unsafe {
            FileTimeToSystemTime(&utc, &mut st).is_ok()
                && SystemTimeToTzSpecificLocalTime(None, &st, &mut local_st).is_ok()
                && SystemTimeToFileTime(&local_st, &mut local).is_ok()
        };
        if !ok {
            return 0;
        }
        let local = (i64::from(local.dwHighDateTime) << 32) | i64::from(local.dwLowDateTime);
        (local - ticks) / 10_000 / schedule::MINUTE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::Weekday;

    fn draft() -> RoutineDraft {
        RoutineDraft {
            id: None,
            name: "  Tidy Downloads ".into(),
            prompt: " sort it ".into(),
            cwd: r"C:\x".into(),
            model: Some(" ".into()),
            max_turns: 500,
            max_budget_usd: 100.0,
            claude_path: None,
            schedule: Schedule::Days {
                time: "09:00".into(),
                days: vec![Weekday::Mon],
            },
            enabled: true,
            if_missed: IfMissed::Once,
            wait_for_me: true,
            desktop_tools: false,
        }
    }

    fn ok(p: &str) -> Option<String> {
        Some(format!("{p}-long"))
    }

    #[test]
    fn new_routines_are_checked_and_clamped() {
        let r = validate(draft(), None, 1000, ok).unwrap();
        assert!(r.id.starts_with("rt-") && r.id.len() == 9, "{}", r.id);
        assert_eq!(r.name, "Tidy Downloads");
        assert_eq!(r.prompt, "sort it");
        assert_eq!(r.cwd, r"C:\x-long");
        assert_eq!(r.model, None);
        assert_eq!(r.max_turns, MAX_TURNS);
        assert_eq!(r.max_budget_usd, MAX_BUDGET);
        assert_eq!((r.schedule_since_ms, r.last_due_ms), (1000, 1000));
        let tiny = RoutineDraft {
            max_budget_usd: 0.001,
            max_turns: 0,
            ..draft()
        };
        let r = validate(tiny, None, 0, ok).unwrap();
        assert_eq!((r.max_turns, r.max_budget_usd), (1, MIN_BUDGET));

        for bad in [
            RoutineDraft {
                name: " ".into(),
                ..draft()
            },
            RoutineDraft {
                prompt: "".into(),
                ..draft()
            },
            RoutineDraft {
                max_budget_usd: 0.0,
                ..draft()
            },
            RoutineDraft {
                max_budget_usd: f64::NAN,
                ..draft()
            },
            RoutineDraft {
                schedule: Schedule::Every {
                    hours: 0,
                    from: "08:00".into(),
                },
                ..draft()
            },
        ] {
            assert!(validate(bad, None, 0, ok).is_err());
        }
        assert!(validate(draft(), None, 0, |_| None).is_err());
    }

    #[test]
    fn edits_keep_the_schedule_clock_unless_it_changed() {
        let mut first = validate(draft(), None, 1000, ok).unwrap();
        first.last_due_ms = 5000;
        first.last_skip = Some(crate::ipc::RoutineSkip {
            at_ms: 5000,
            due_ms: 5000,
            why: crate::ipc::SkipReason::StillRunning,
        });
        // A new prompt: same id, same clock.
        let edit = RoutineDraft {
            prompt: "other".into(),
            ..draft()
        };
        let r = validate(edit, Some(&first), 9000, ok).unwrap();
        assert_eq!(r.id, first.id);
        assert_eq!((r.schedule_since_ms, r.last_due_ms), (1000, 5000));
        assert_eq!((r.created_at_ms, r.updated_at_ms), (1000, 9000));
        assert!(r.last_skip.is_some());
        // A new time: counts from now.
        let moved = RoutineDraft {
            schedule: Schedule::Days {
                time: "10:00".into(),
                days: vec![Weekday::Mon],
            },
            ..draft()
        };
        let r = validate(moved, Some(&first), 9000, ok).unwrap();
        assert_eq!((r.schedule_since_ms, r.last_due_ms), (9000, 9000));
        assert!(r.last_skip.is_none());
        // Turned back on: counts from now too.
        let off = Routine {
            enabled: false,
            ..first.clone()
        };
        let r = validate(draft(), Some(&off), 9000, ok).unwrap();
        assert_eq!(r.last_due_ms, 9000);
    }

    #[test]
    fn the_file_round_trips_and_is_replaced() {
        let dir = std::env::temp_dir().join(format!("topbar-routines-{}", random_hex(4)));
        let path = dir.join("routines.json");
        let mut store = Store::load(path.clone());
        assert!(store.file.routines.is_empty());
        store
            .upsert(validate(draft(), None, 1, ok).unwrap())
            .unwrap();
        store.file.paused = true;
        store.save();
        store.save();
        let again = Store::load(path.clone());
        assert_eq!(again.file.routines.len(), 1);
        assert!(again.file.paused);
        assert_eq!(again.file.routines[0].schedule, draft().schedule);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains(r#""kind": "days""#) && text.contains(r#""mon""#),
            "{text}"
        );
        // An unreadable file is kept aside, not overwritten.
        std::fs::write(&path, "{ nope").unwrap();
        let mut broken = Store::load(path.clone());
        assert!(broken.file.routines.is_empty());
        broken.save();
        assert_eq!(
            std::fs::read_to_string(path.with_extension("unreadable.json")).unwrap(),
            "{ nope"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_local_zone_answers() {
        // Whatever the machine's zone, offsets are whole minutes within ±14 h.
        let o = LocalZone.offset_min(1_793_370_153_000);
        assert!((-14 * 60..=14 * 60).contains(&o));
    }
}
