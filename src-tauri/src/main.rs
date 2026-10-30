// Prevents an extra console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // `topbar.exe hook` is a Claude Code hook command: forward stdin to the running TopBar and exit,
    // before Tauri (and its single-instance check) starts.
    if std::env::args_os().nth(1).is_some_and(|arg| arg == "hook") {
        topbar_lib::forward_hook();
        return;
    }
    topbar_lib::run()
}
