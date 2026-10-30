mod activity;
mod apps;
mod autostart;
mod claude;
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

pub fn run() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    tauri::Builder::default()
        // Must be registered first: a second launch exits before creating another island.
        .plugin(tauri_plugin_single_instance::init(|_app, _args, _cwd| {
            log::info!("second launch ignored; TopBar is already running");
        }))
        .manage(hover::Island::default())
        .manage(apps::IconCache::default())
        .manage(media::Media::default())
        .manage(activity::Activities::default())
        .manage(claude::Claude::default())
        .setup(|app| {
            autostart::repair();
            let handle = app.handle().clone();
            let (island, geometry) = window::create(&handle)?;
            hover::spawn(handle.clone(), &island, geometry)?;
            media::spawn(handle.clone(), &app.state::<media::Media>())?;
            activity::spawn(&handle, &app.state::<activity::Activities>());
            claude::spawn(&handle, &app.state::<activity::Activities>());
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
        ])
        .build(tauri::generate_context!())
        .expect("error while building TopBar")
        .run(|app, event| {
            if let RunEvent::Exit = event {
                app.state::<media::Media>().shutdown();
                app.state::<activity::Activities>().shutdown();
            }
        });
}
