mod activity;
mod agent;
mod apps;
mod autostart;
mod claude;
mod desktop;
mod error;
mod fullscreen;
mod hover;
mod ipc;
mod media;
mod tray;
mod win32;
mod window;

use tauri::{Manager, RunEvent};

/// `topbar.exe hook`: forwards a Claude Code hook to the running TopBar and returns. Called from `main.rs`
/// before `run()`, so no Tauri, WebView2 or single-instance check is involved.
pub fn forward_hook() {
    claude::forward_hook();
}

/// `topbar.exe mcp`: the stdio MCP bridge Claude Code starts for an agent run. Returns the exit code.
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
            log::info!("second launch ignored; TopBar is already running");
        }))
        .manage(hover::Island::default())
        .manage(apps::IconCache::default())
        .manage(media::Media::default())
        .manage(activity::Activities::default())
        .manage(claude::Claude::default())
        .manage(agent::Agents::default())
        .setup(|app| {
            autostart::repair();
            let handle = app.handle().clone();
            let (island, geometry) = window::create(&handle)?;
            hover::spawn(handle.clone(), &island, geometry)?;
            media::spawn(handle.clone(), &app.state::<media::Media>())?;
            activity::spawn(&handle, &app.state::<activity::Activities>());
            claude::spawn(&handle, &app.state::<activity::Activities>());
            agent::spawn(&handle, &app.state::<activity::Activities>());
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
            claude::get_claude_sessions,
            claude::claude_focus_session,
            claude::claude_hooks_status,
            claude::claude_hooks_install,
            claude::claude_hooks_remove,
            agent::agent_start,
            agent::agent_send,
            agent::agent_stop,
            agent::agent_runs,
            agent::agent_transcript,
            agent::agent_decide,
            agent::agent_options,
            agent::agent_pick_folder,
            agent::command_window_show,
            agent::command_window_hide,
            agent::routines_list,
            agent::routine_save,
            agent::routine_delete,
            agent::routine_run,
            agent::routines_pause,
        ])
        .build(tauri::generate_context!())
        .expect("error while building TopBar")
        .run(|app, event| {
            if let RunEvent::Exit = event {
                app.state::<agent::Agents>().shutdown();
                app.state::<media::Media>().shutdown();
                app.state::<activity::Activities>().shutdown();
            }
        });
}
