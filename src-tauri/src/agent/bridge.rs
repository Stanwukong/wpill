//! `topbar.exe mcp`: the stdio MCP server Claude Code starts for an agent run (from the run's generated
//! `--mcp-config`). Like `topbar.exe hook` it runs before Tauri (checked in `main.rs`) and never prints
//! anything of its own: it relays newline-delimited JSON-RPC between stdio and the agent pipe, one message per
//! pipe message, after a hello carrying the run's token (from the config's `env`). TopBar terminates MCP.

use std::io::{BufRead, Write};
use std::sync::{Arc, Mutex};

use serde_json::Value;
use windows::Win32::System::Threading::INFINITE;

use super::mcp::{self, Hello, HELLO_VERSION};
use super::pipe::{Pipe, Read, MAX_MESSAGE};

pub const TOKEN_ENV: &str = "TOPBAR_AGENT_TOKEN";
pub const RUN_ENV: &str = "TOPBAR_AGENT_RUN";

/// Returns the process exit code: 0 when Claude Code closed stdin, non-zero if TopBar couldn't be reached.
pub fn run() -> i32 {
    let (Ok(token), Ok(run)) = (std::env::var(TOKEN_ENV), std::env::var(RUN_ENV)) else {
        return 2;
    };
    let Ok(pipe) = Pipe::open() else {
        return 3;
    };
    let pipe = Arc::new(pipe);
    let hello = Hello {
        topbar: HELLO_VERSION,
        run,
        token,
    };
    let Ok(hello) = serde_json::to_vec(&hello) else {
        return 4;
    };
    if pipe.write(&hello).is_err() {
        return 3;
    }
    let stdout = Arc::new(Mutex::new(std::io::stdout()));

    // TopBar → Claude Code. When TopBar goes away there's nobody left to answer: end the bridge.
    let (from, out) = (pipe.clone(), stdout.clone());
    std::thread::spawn(move || loop {
        match from.read(&[], INFINITE) {
            Ok(Read::Message(message)) => {
                let mut out = out.lock().unwrap_or_else(|e| e.into_inner());
                if out.write_all(&message).is_err()
                    || out.write_all(b"\n").is_err()
                    || out.flush().is_err()
                {
                    std::process::exit(0);
                }
            }
            Ok(Read::TooLong) => {}
            _ => std::process::exit(0),
        }
    });

    // Claude Code → TopBar, until Claude Code closes stdin.
    let mut stdin = std::io::stdin().lock();
    let mut line = Vec::new();
    loop {
        line.clear();
        match stdin.read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => return 0,
            Ok(_) => {}
        }
        let message = line.trim_ascii();
        if message.is_empty() {
            continue;
        }
        if message.len() > MAX_MESSAGE {
            // Answer it here so Claude Code doesn't wait forever (a denied approval, in practice).
            let id = serde_json::from_slice::<Value>(message)
                .ok()
                .and_then(|m| m.get("id").cloned());
            if let Some(id) = id {
                let reply = mcp::error(id, -32600, "message too large for TopBar");
                let mut out = stdout.lock().unwrap_or_else(|e| e.into_inner());
                let _ = writeln!(out, "{reply}");
                let _ = out.flush();
            }
            continue;
        }
        if pipe.write(message).is_err() {
            return 3;
        }
    }
}
