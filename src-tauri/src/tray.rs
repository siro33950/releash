use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{
    image::Image,
    menu::{MenuBuilder, MenuItemBuilder},
    tray::TrayIconBuilder,
    App, Listener, Manager,
};

pub static QUIT_REQUESTED: AtomicBool = AtomicBool::new(false);

pub mod ids {
    pub const SHOW_WINDOW: &str = "tray-show-window";
    pub const START_SERVER: &str = "tray-start-server";
    pub const STOP_SERVER: &str = "tray-stop-server";
    pub const QUIT: &str = "tray-quit";
}

pub struct TrayMenuItems {
    pub start_server: tauri::menu::MenuItem<tauri::Wry>,
    pub stop_server: tauri::menu::MenuItem<tauri::Wry>,
}

pub fn setup_tray(app: &App) -> Result<(), Box<dyn std::error::Error>> {
    let handle = app.handle();

    let show_window = MenuItemBuilder::with_id(ids::SHOW_WINDOW, "Show Releash").build(handle)?;
    let start_server = MenuItemBuilder::with_id(ids::START_SERVER, "Start Server").build(handle)?;
    let stop_server = MenuItemBuilder::with_id(ids::STOP_SERVER, "Stop Server")
        .enabled(false)
        .build(handle)?;
    let quit = MenuItemBuilder::with_id(ids::QUIT, "Quit").build(handle)?;

    let menu = MenuBuilder::new(handle)
        .item(&show_window)
        .separator()
        .item(&start_server)
        .item(&stop_server)
        .separator()
        .item(&quit)
        .build()?;

    app.manage(TrayMenuItems {
        start_server: start_server.clone(),
        stop_server: stop_server.clone(),
    });

    let icon = Image::from_bytes(include_bytes!("../icons/32x32.png"))?;

    TrayIconBuilder::new()
        .icon(icon)
        .menu(&menu)
        .tooltip("Releash")
        .on_menu_event(handle_menu_event)
        .on_tray_icon_event(|tray, event| {
            if let tauri::tray::TrayIconEvent::DoubleClick { .. } = event {
                let app = tray.app_handle();
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
        })
        .build(app)?;

    Ok(())
}

fn handle_menu_event(app: &tauri::AppHandle, event: tauri::menu::MenuEvent) {
    match event.id().as_ref() {
        ids::SHOW_WINDOW => {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }
        ids::START_SERVER => {
            handle_start_server(app.clone());
        }
        ids::STOP_SERVER => {
            handle_stop_server(app.clone());
        }
        ids::QUIT => {
            QUIT_REQUESTED.store(true, Ordering::SeqCst);
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let _ = crate::ws_server::commands::stop_server_core(&app).await;
                app.exit(0);
            });
        }
        _ => {}
    }
}

fn handle_start_server(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let config = {
            let config_state = app.state::<std::sync::Arc<crate::config::AppConfig>>();
            match config_state.get_config() {
                Ok(c) => c,
                Err(e) => {
                    log::error!("Failed to get config for tray start: {e}");
                    return;
                }
            }
        };

        let last_root_path = config.app.last_root_path.clone();
        let last_bind_ip = config.app.last_bind_ip.clone();

        if last_root_path.is_empty() || last_bind_ip.is_empty() {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
            return;
        }

        let repo_paths = vec![last_root_path];
        if let Err(e) =
            crate::ws_server::commands::start_server_core(&app, repo_paths, last_bind_ip).await
        {
            log::error!("Failed to start server from tray: {e}");
        }
    });
}

fn handle_stop_server(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        if let Err(e) = crate::ws_server::commands::stop_server_core(&app).await {
            log::error!("Failed to stop server from tray: {e}");
        }
    });
}

pub fn update_tray_menu(app_handle: &tauri::AppHandle, server_running: bool) {
    if let Some(tray_items) = app_handle.try_state::<TrayMenuItems>() {
        let _ = tray_items.start_server.set_enabled(!server_running);
        let _ = tray_items.stop_server.set_enabled(server_running);
    }
}

pub fn listen_server_status(app_handle: &tauri::AppHandle) {
    let handle = app_handle.clone();
    app_handle.listen("server-status-changed", move |event| {
        if let Ok(payload) =
            serde_json::from_str::<crate::ws_server::commands::ServerStatusPayload>(event.payload())
        {
            update_tray_menu(&handle, payload.running);
        }
    });
}
