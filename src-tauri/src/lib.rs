mod activity;
mod agent;
mod apps;
mod autostart;
mod desktop;
mod duplex;
mod error;
mod fullscreen;
mod google;
mod hover;
mod ipc;
mod media;
mod migrate;
mod processes;
mod sessions;
mod tray;
mod win32;
mod window;

use tauri::{Manager, RunEvent};

/// `cay.exe hook`: forwards a Claude Code hook to the running Cay and returns. Called from `main.rs`
/// before `run()`, so no Tauri, WebView2 or single-instance check is involved.
pub fn forward_hook() {
    sessions::forward_hook();
}

/// `cay.exe statusline`: Claude Code's status line command after Connect (2.8f). Sends the usage limits to the
/// running Cay, then runs the user's own status line. Called from `main.rs` before `run()`, like `forward_hook`.
pub fn forward_statusline() {
    sessions::forward_statusline();
}

/// `cay.exe mcp`: the stdio MCP bridge Claude Code starts for an agent run. Returns the exit code.
pub fn mcp_bridge() -> i32 {
    agent::mcp_bridge()
}

pub fn run() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    tauri::Builder::default()
        // Must be registered first: a second launch exits before creating another island.
        .plugin(tauri_plugin_single_instance::init(|_app, _args, _cwd| {
            #[cfg(debug_assertions)]
            if _args.get(1).is_some_and(|a| a == "agent-test") {
                agent::debug_command(_app, &_args[2..]);
                return;
            }
            #[cfg(debug_assertions)]
            if _args.get(1).is_some_and(|a| a == "google-test") {
                google::debug_command(_app, &_args[2..]);
                return;
            }
            log::info!("second launch ignored; Cay is already running");
        }))
        .manage(hover::Island::default())
        .manage(apps::IconCache::default())
        .manage(media::Media::default())
        .manage(activity::Activities::default())
        .manage(sessions::Sessions::default())
        .manage(sessions::Asks::default())
        .manage(agent::Agents::default())
        .manage(google::Google::default())
        .manage(processes::Processes::default())
        .setup(|app| {
            let handle = app.handle().clone();
            migrate::from_topbar(&handle);
            autostart::repair();
            let (island, geometry) = window::create(&handle)?;
            hover::spawn(handle.clone(), &island, geometry)?;
            media::spawn(handle.clone(), &app.state::<media::Media>())?;
            activity::spawn(&handle, &app.state::<activity::Activities>());
            sessions::spawn(&handle, &app.state::<activity::Activities>());
            agent::spawn(&handle, &app.state::<activity::Activities>());
            google::spawn(&handle, &app.state::<activity::Activities>());
            processes::spawn(&handle, &app.state::<activity::Activities>());
            tray::create(&handle)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            apps::list_windows,
            apps::activate_window,
            hover::set_panel_rect,
            hover::request_collapse,
            hover::get_island_state,
            media::get_media_state,
            media::media_play_pause,
            media::media_next,
            media::media_prev,
            media::media_seek,
            activity::activity_drag_screenshot,
            activity::activity_open_screenshot,
            sessions::get_sessions,
            sessions::session_focus,
            sessions::session_decide,
            sessions::session_asks,
            sessions::session_answer,
            sessions::session_plan,
            sessions::session_ask_preview,
            sessions::claude_hooks_status,
            sessions::claude_hooks_install,
            sessions::claude_hooks_remove,
            agent::agent_start,
            agent::agent_backends,
            agent::agent_send,
            agent::agent_stop,
            agent::agent_remove,
            agent::agent_runs,
            agent::agent_transcript,
            agent::agent_open_link,
            agent::agent_paste_attachments,
            agent::agent_describe_attachments,
            agent::agent_pick_attachments,
            agent::agent_decide,
            agent::agent_options,
            agent::agent_pick_folder,
            agent::command_window_show,
            agent::command_window_show_ask,
            agent::command_take_focus,
            agent::agent_approval_preview,
            agent::agent_answer,
            agent::agent_set_mode,
            agent::agent_cycle_mode,
            agent::agent_plan,
            agent::command_window_hide,
            agent::routines_list,
            agent::routine_save,
            agent::routine_delete,
            agent::routine_run,
            agent::routines_pause,
            google::google_state,
            google::calendar_upcoming,
            google::google_connect,
            google::google_disconnect,
            google::calendar_open,
            google::calendar_join,
            processes::list_heavy_processes,
            processes::end_process_group,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Cay")
        .run(|app, event| {
            if let RunEvent::Exit = event {
                app.state::<agent::Agents>().shutdown();
                app.state::<media::Media>().shutdown();
                app.state::<activity::Activities>().shutdown();
            }
        });
}
