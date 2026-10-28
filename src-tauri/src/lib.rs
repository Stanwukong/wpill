mod apps;
mod error;
mod fullscreen;
mod hover;
mod ipc;
mod tray;
mod win32;
mod window;

pub fn run() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    tauri::Builder::default()
        // Must be registered first: a second launch exits before creating another island.
        .plugin(tauri_plugin_single_instance::init(|_app, _args, _cwd| {
            log::info!("second launch ignored; TopBar is already running");
        }))
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .app_name("TopBar")
                .build(),
        )
        .manage(hover::Island::default())
        .manage(apps::IconCache::default())
        .setup(|app| {
            let handle = app.handle().clone();
            let (island, geometry) = window::create(&handle)?;
            hover::spawn(handle.clone(), &island, geometry)?;
            tray::create(&handle)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            apps::list_windows,
            apps::activate_window,
            hover::set_panel_rect,
            hover::request_collapse,
            hover::get_island_state,
        ])
        .run(tauri::generate_context!())
        .expect("error while running TopBar");
}
