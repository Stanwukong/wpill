//! Codex CLI sessions (2.9a, D47): its hooks run the same forwarder as Claude Code's (`cay.exe hook --agent codex`),
//! so sessions, held permission requests and activities all go through the rest of `sessions/`. This module owns
//! what differs: Codex's `hooks.json` (`hooks.rs`), the version check, the tray's Connect/Disconnect Codex, and
//! "waiting for trust" (Codex skips a new hook until you trust it with `/hooks`).

pub mod hooks;

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tauri::menu::MenuItem;
use tauri::Wry;
use windows::core::HSTRING;
use windows::Win32::Storage::FileSystem::GetShortPathNameW;
use windows::Win32::System::Threading::CREATE_NO_WINDOW;
use windows::Win32::UI::WindowsAndMessaging::{
    IDYES, MB_ICONINFORMATION, MB_ICONQUESTION, MB_ICONWARNING, MB_OK, MB_YESNO, MB_YESNOCANCEL,
};

use super::{confirm, done_text, files, message, this_exe};
use hooks::State;

/// Cay's own `settings.json` block: `{ "hooksSeen": bool }`, false from Connect until a Codex hook arrives.
const BLOCK: &str = "codex";
const VERSION_TIMEOUT: Duration = Duration::from_secs(5);

/// The tray item, relabelled when the first Codex hook arrives.
static TRAY: OnceLock<MenuItem<Wry>> = OnceLock::new();
/// A Codex hook has arrived since Connect (cached copy of `hooksSeen`).
static SEEN: AtomicBool = AtomicBool::new(false);
static SEEN_LOADED: AtomicBool = AtomicBool::new(false);
static DIALOG_OPEN: AtomicBool = AtomicBool::new(false);

fn seen() -> bool {
    if !SEEN_LOADED.swap(true, Ordering::SeqCst) {
        let saved = files::cay_block(BLOCK)
            .and_then(|b| b.get("hooksSeen").and_then(Value::as_bool))
            .unwrap_or(false);
        SEEN.store(saved, Ordering::SeqCst);
    }
    SEEN.load(Ordering::SeqCst)
}

fn set_seen(value: Option<bool>) -> Result<(), String> {
    SEEN_LOADED.store(true, Ordering::SeqCst);
    SEEN.store(value.unwrap_or(false), Ordering::SeqCst);
    files::set_cay_block(BLOCK, value.map(|v| json!({ "hooksSeen": v })))
}

/// The listener got a Codex hook: Codex runs Cay's entries, so they're trusted.
pub fn saw_hook() {
    if seen() {
        return;
    }
    log::info!("Codex ran Cay's hooks: they're trusted");
    if let Err(e) = set_seen(Some(true)) {
        log::warn!("couldn't record that Codex's hooks run: {e}");
    }
    if let Some(item) = TRAY.get() {
        let _ = item.set_text(tray_label());
    }
}

fn path() -> Result<PathBuf, String> {
    hooks::path().ok_or_else(|| "couldn't find your user folder".to_string())
}

/// The hook command for this exe (`hooks::command`).
fn command() -> Result<String, String> {
    let exe = this_exe()?;
    Ok(hooks::command(&exe, short_path(&exe).as_deref()))
}

fn state() -> State {
    let (Ok(path), Ok(command)) = (path(), command()) else {
        return State::NotConnected;
    };
    match files::read(&path) {
        Ok(Some(file)) => hooks::state(&file, &command),
        _ => State::NotConnected,
    }
}

pub fn tray_label() -> &'static str {
    match state() {
        State::Connected if seen() => "Disconnect Codex…",
        State::Connected => "Finish connecting Codex…",
        State::OutOfDate => "Update Codex connection…",
        State::NotConnected => "Connect Codex…",
    }
}

/// The 8.3 form of `path`, if the volume keeps 8.3 names.
fn short_path(path: &Path) -> Option<PathBuf> {
    let long = HSTRING::from(path.as_os_str());
    let mut buf = vec![0u16; 1024];
    // SAFETY: `long` is a valid wide string and `buf` a writable buffer whose length is passed with it.
    let n = unsafe { GetShortPathNameW(&long, Some(&mut buf)) } as usize;
    (n > 0 && n < buf.len()).then(|| PathBuf::from(String::from_utf16_lossy(&buf[..n])))
}

/// The `codex` you'd run: the first on PATH, either a native `codex.exe` (asked with `--version`) or npm's shim (its
/// package's `package.json`). `None` if there's none, or it didn't say.
fn installed_version() -> Option<(u32, u32, u32)> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let exe = dir.join("codex.exe");
        if exe.is_file() {
            return run_version(&exe);
        }
        if ["codex.cmd", "codex.ps1", "codex"]
            .iter()
            .any(|n| dir.join(n).is_file())
        {
            let package = dir.join(r"node_modules\@openai\codex\package.json");
            return std::fs::read_to_string(package)
                .ok()
                .and_then(|t| serde_json::from_str::<Value>(&t).ok())
                .and_then(|v| v.get("version")?.as_str().and_then(hooks::parse_version));
        }
    }
    None
}

fn run_version(exe: &Path) -> Option<(u32, u32, u32)> {
    let mut child = Command::new(exe)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW.0)
        .spawn()
        .ok()?;
    let deadline = Instant::now() + VERSION_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                return None;
            }
        }
    }
    let output = child.wait_with_output().ok()?;
    hooks::parse_version(&String::from_utf8_lossy(&output.stdout))
}

fn version_text((a, b, c): (u32, u32, u32)) -> String {
    format!("{a}.{b}.{c}")
}

/// Tray "Connect/Disconnect Codex…", as for Claude Code: a native dialog on its own thread that shows exactly what
/// changes before writing.
pub fn toggle_from_tray(item: MenuItem<Wry>) {
    let _ = TRAY.set(item.clone());
    if DIALOG_OPEN.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        if let Err(e) = run_dialog() {
            message(
                &format!("Cay couldn't change Codex's hooks:\n\n{e}"),
                MB_OK | MB_ICONWARNING,
            );
        }
        if let Err(e) = item.set_text(tray_label()) {
            log::warn!("updating tray label failed: {e}");
        }
        DIALOG_OPEN.store(false, Ordering::SeqCst);
    });
}

/// Keeps the tray item so the first Codex hook can relabel it (`saw_hook`).
pub fn keep_tray_item(item: MenuItem<Wry>) {
    let _ = TRAY.set(item);
}

const TRUST: &str = "Codex runs a new hook only after you trust it: in Codex, type /hooks and trust the entries that \
                     run cay.exe. Sessions that are already open pick them up after a restart.";

fn run_dialog() -> Result<(), String> {
    let path = path()?;
    let state = state();
    if state == State::Connected {
        if seen() {
            if !confirm(&remove_text(&path)) {
                return Ok(());
            }
            return disconnect(&path);
        }
        let text = format!(
            "Cay's hooks are in\n{}\nbut Codex hasn't run them yet.\n\n{TRUST}\n\nDisconnect Codex instead?",
            path.display()
        );
        if message(&text, MB_YESNO | MB_ICONQUESTION) == IDYES {
            return disconnect(&path);
        }
        return Ok(());
    }
    let version = installed_version();
    if let Some(v) = version.filter(|v| *v < hooks::MIN_VERSION) {
        message(
            &format!(
                "Codex {} is older than {}, the first version Cay works with. Update it (codex update, or \
                 npm install -g @openai/codex), then connect again.",
                version_text(v),
                version_text(hooks::MIN_VERSION)
            ),
            MB_OK | MB_ICONINFORMATION,
        );
        return Ok(());
    }
    let command = command()?;
    let checked = match version {
        Some(v) => format!("Codex {} found.", version_text(v)),
        None => format!(
            "Cay couldn't find Codex to check its version; it needs {} or later.",
            version_text(hooks::MIN_VERSION)
        ),
    };
    let change = hooks::preview(&command);
    // A CODEX_HOME left over from a test points Connect elsewhere; say so (it happened in 2.9a's manual check).
    let home = match std::env::var_os("CODEX_HOME").filter(|d| !d.is_empty()) {
        Some(dir) => format!(
            "\n\nCODEX_HOME is set to {}, so this is that folder, not your usual .codex. Codex sessions started \
             without it won't see these hooks.",
            PathBuf::from(dir).display()
        ),
        None => String::new(),
    };
    let tail = format!(
        "Your other hooks stay. A backup of the file is saved next to it first.\n\n{TRUST}\n\n{checked}{home}"
    );
    if state == State::OutOfDate {
        let text = format!(
            "Cay's hooks in\n{}\nare from another copy or version of Cay. Update them to this?\n\n{change}\n\n\
             {tail}\n\nYes: update.   No: disconnect Codex instead.",
            path.display()
        );
        return match message(&text, MB_YESNOCANCEL | MB_ICONQUESTION) {
            IDYES => connect(&path, &command, "Codex's hooks are up to date."),
            windows::Win32::UI::WindowsAndMessaging::IDNO => disconnect(&path),
            _ => Ok(()),
        };
    }
    let text = format!(
        "Cay will add this to\n{}\n\n{change}\n\n{tail}",
        path.display()
    );
    if !confirm(&text) {
        return Ok(());
    }
    connect(&path, &command, "Codex is connected.")
}

fn remove_text(path: &Path) -> String {
    format!(
        "Remove Cay from\n{}?\n\nOnly hooks whose command ends in \"cay.exe hook --agent codex\" are removed; your other \
         hooks stay. A backup of the file is saved next to it first.",
        path.display()
    )
}

fn connect(path: &Path, command: &str, done: &str) -> Result<(), String> {
    let file = files::read(path)?.unwrap_or_else(|| json!({}));
    let backup = files::write(path, &hooks::with_hooks(file, command)?)?;
    set_seen(Some(false))?;
    log::info!("connected Codex in {}", path.display());
    message(
        &format!("{}\n\n{TRUST}", done_text(done, backup)),
        MB_OK | MB_ICONINFORMATION,
    );
    Ok(())
}

fn disconnect(path: &Path) -> Result<(), String> {
    let backup = match files::read(path)? {
        Some(file) => files::write(path, &hooks::without_hooks(file)?)?,
        None => None,
    };
    set_seen(None)?;
    log::info!("disconnected Codex in {}", path.display());
    message(
        &done_text("Codex is disconnected.", backup),
        MB_OK | MB_ICONINFORMATION,
    );
    Ok(())
}
