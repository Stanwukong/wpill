// Prevents an extra console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // `cay.exe hook` is a Claude Code hook command: forward stdin to the running Cay and exit,
    // before Tauri (and its single-instance check) starts.
    if std::env::args_os().nth(1).is_some_and(|arg| arg == "hook") {
        cay_lib::forward_hook();
        return;
    }
    // `cay.exe mcp` is Cay's MCP server for agent runs, started by Claude Code over stdio.
    if std::env::args_os().nth(1).is_some_and(|arg| arg == "mcp") {
        std::process::exit(cay_lib::mcp_bridge());
    }
    cay_lib::run()
}
