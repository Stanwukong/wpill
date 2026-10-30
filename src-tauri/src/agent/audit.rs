//! The trail: `%APPDATA%\dev.wpill.topbar\agent\<runId>.jsonl`, one JSON object per line, local only. Every
//! start, tool call, approval, decision and result is appended; files older than 30 days are deleted at launch.
//! Run ids double as file names: `20261030-142233-7f3a` (UTC time + random), so they sort by date.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use windows::Win32::Security::Cryptography::{BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG};

use super::describe::truncate;

const KEEP: Duration = Duration::from_secs(30 * 24 * 60 * 60);
/// Tool input kept per line (a `Write` carries a whole file).
const MAX_INPUT: usize = 4096;

pub struct Audit {
    dir: PathBuf,
}

impl Audit {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Appends `{ at, run, kind, …fields }`. Failures are logged, never fatal.
    pub fn log(&self, run_id: &str, kind: &str, fields: Value) {
        let mut line = json!({ "at": now_ms(), "run": run_id, "kind": kind });
        if let (Some(line), Value::Object(fields)) = (line.as_object_mut(), fields) {
            line.extend(fields);
        }
        let result = std::fs::create_dir_all(&self.dir).and_then(|()| {
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.dir.join(format!("{run_id}.jsonl")))?;
            writeln!(file, "{line}")
        });
        if let Err(e) = result {
            log::warn!("agent audit log: {e}");
        }
    }

    /// Deletes trails (and leftover run folders) older than 30 days.
    pub fn prune(&self) {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return;
        };
        let cutoff = SystemTime::now() - KEEP;
        for entry in entries.flatten() {
            let path = entry.path();
            let old = entry
                .metadata()
                .and_then(|m| m.modified())
                .is_ok_and(|t| t < cutoff);
            if old && path.extension().is_some_and(|e| e == "jsonl") {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
}

/// Tool input for the trail, cut to a few KB.
pub fn input_for_log(input: &Value) -> Value {
    let text = input.to_string();
    if text.len() <= MAX_INPUT {
        input.clone()
    } else {
        Value::String(truncate(&text, MAX_INPUT))
    }
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// Random bytes from the system RNG, as hex.
pub fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    // SAFETY: fills `buf`; with the system-preferred flag no algorithm handle is needed.
    let status = unsafe { BCryptGenRandom(None, &mut buf, BCRYPT_USE_SYSTEM_PREFERRED_RNG) };
    if status.is_err() {
        // Not expected; fall back to the clock so ids stay distinct (the token check still holds per run).
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        for (i, b) in buf.iter_mut().enumerate() {
            *b = (nanos >> ((i % 16) * 8)) as u8;
        }
    }
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// `20261030-142233-7f3a`.
pub fn run_id(unix_ms: i64) -> String {
    let secs = unix_ms.div_euclid(1000);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (y, m, d) = civil(days);
    format!(
        "{y:04}{m:02}{d:02}-{:02}{:02}{:02}-{}",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60,
        random_hex(2)
    )
}

/// Days since 1970-01-01 → (year, month, day), proleptic Gregorian (Howard Hinnant's algorithm).
fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_ids_are_dated() {
        // 2026-10-30 14:22:33 UTC
        let id = run_id(1_793_370_153_000);
        assert!(id.starts_with("20261030-142233-"), "{id}");
        assert_eq!(id.len(), "20261030-142233-7f3a".len());
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(-1), (1969, 12, 31));
        assert_eq!(civil(19_782), (2024, 2, 29));
    }

    #[test]
    fn randomness_and_input_cut() {
        assert_ne!(random_hex(16), random_hex(16));
        assert_eq!(random_hex(16).len(), 32);
        let big = json!({ "content": "x".repeat(10_000) });
        assert!(input_for_log(&big).as_str().unwrap().chars().count() <= MAX_INPUT);
        let small = json!({ "command": "ls" });
        assert_eq!(input_for_log(&small), small);
    }

    #[test]
    fn log_lines_append() {
        let dir = std::env::temp_dir().join(format!("topbar-audit-{}", random_hex(4)));
        let audit = Audit::new(dir.clone());
        audit.log("r1", "start", json!({ "prompt": "hi" }));
        audit.log("r1", "result", json!({ "turns": 2 }));
        let text = std::fs::read_to_string(dir.join("r1.jsonl")).unwrap();
        let lines: Vec<Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["kind"], "start");
        assert_eq!(lines[0]["prompt"], "hi");
        assert_eq!(lines[1]["turns"], 2);
        let _ = std::fs::remove_dir_all(dir);
    }
}
