mod apps;
mod autostart;
mod error;
mod fullscreen;
mod hover;
mod ipc;
mod media;
mod tray;
mod win32;
mod window;

use tauri::{Manager, RunEvent};

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
        .setup(|app| {
            autostart::repair();
            let handle = app.handle().clone();
            let (island, geometry) = window::create(&handle)?;
            hover::spawn(handle.clone(), &island, geometry)?;
            media::spawn(handle.clone(), &app.state::<media::Media>())?;
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
        ])
        .build(tauri::generate_context!())
        .expect("error while building TopBar")
        .run(|app, event| {
            if let RunEvent::Exit = event {
                app.state::<media::Media>().shutdown();
            }
        });
}
