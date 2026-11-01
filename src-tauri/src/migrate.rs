//! One-time moves from TopBar, Cay's name until Phase 4a-3 (D32). Run first in `setup`, before anything reads
//! settings or routines.
//!
//! - `%APPDATA%\dev.wpill.topbar` (settings, routines, the agent audit) is *copied* to Cay's folder when Cay's doesn't
//!   exist yet. Copied, not moved: an installed TopBar keeps working until it's uninstalled, and nothing is lost if
//!   the copy fails half-way (it's retried while Cay's folder is still missing).
//! - The `TopBar` startup entry becomes `Cay` (`autostart::migrate_from_topbar`).
//!
//! Claude Code hooks aren't touched here: `~/.claude/settings.json` is only written when you click Connect. Old
//! TopBar entries don't count as connected, so the tray offers Connect, which replaces them (`claude::hooks`).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager};

use crate::autostart;

/// TopBar's bundle identifier, which named its folders.
const OLD_IDENTIFIER: &str = "dev.wpill.topbar";

pub fn from_topbar(app: &AppHandle) {
    autostart::migrate_from_topbar();
    let Ok(new) = app.path().app_config_dir() else {
        return;
    };
    let Some(old) = new.parent().map(|p| p.join(OLD_IDENTIFIER)) else {
        return;
    };
    if new.exists() || !old.is_dir() {
        return;
    }
    let partial = new.with_extension("migrating");
    let _ = fs::remove_dir_all(&partial);
    match copy_tree(&old, &partial, Path::new(""))
        .and_then(|n| fs::rename(&partial, &new).map(|()| n))
    {
        Ok(files) => log::info!(
            "copied {files} files from {} to {}",
            old.display(),
            new.display()
        ),
        Err(e) => {
            log::warn!("copying TopBar's settings failed: {e}");
            let _ = fs::remove_dir_all(&partial);
        }
    }
}

/// Copies `from` into `to`, skipping what `keep` rejects (paths relative to the top). Returns the file count.
fn copy_tree(from: &Path, to: &Path, rel: &Path) -> io::Result<usize> {
    fs::create_dir_all(to)?;
    let mut files = 0;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let rel: PathBuf = rel.join(entry.file_name());
        if !keep(&rel) {
            continue;
        }
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            files += copy_tree(&entry.path(), &target, &rel)?;
        } else {
            fs::copy(entry.path(), &target)?;
            files += 1;
        }
    }
    Ok(files)
}

/// Per-run files (`agent\runs\<runId>`) only live while a run's process does; WebView2's profile is rebuilt.
fn keep(rel: &Path) -> bool {
    let mut parts = rel.iter().map(|p| p.to_string_lossy().to_ascii_lowercase());
    !matches!(
        (parts.next().as_deref(), parts.next().as_deref()),
        (Some("ebwebview"), _) | (Some("agent"), Some("runs"))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_settings_routines_and_the_audit() {
        for rel in [
            r"settings.json",
            r"routines.json",
            r"agent",
            r"agent\20261030-113129-fc53.jsonl",
            r"agent\routines.jsonl",
        ] {
            assert!(keep(Path::new(rel)), "{rel}");
        }
        for rel in [
            r"agent\runs",
            r"agent\runs\r1\mcp.json",
            r"EBWebView",
            r"EBWebView\Default",
        ] {
            assert!(!keep(Path::new(rel)), "{rel}");
        }
    }

    #[test]
    fn copies_a_tree_without_the_skipped_parts() {
        let root = std::env::temp_dir().join(format!("cay-migrate-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let (old, new) = (root.join("old"), root.join("new"));
        fs::create_dir_all(old.join(r"agent\runs\r1")).unwrap();
        fs::write(old.join("settings.json"), "{}").unwrap();
        fs::write(old.join(r"agent\a.jsonl"), "x").unwrap();
        fs::write(old.join(r"agent\runs\r1\mcp.json"), "{}").unwrap();

        assert_eq!(copy_tree(&old, &new, Path::new("")).unwrap(), 2);
        assert_eq!(fs::read_to_string(new.join("settings.json")).unwrap(), "{}");
        assert!(new.join(r"agent\a.jsonl").is_file());
        assert!(!new.join(r"agent\runs").exists());
        let _ = fs::remove_dir_all(&root);
    }
}
