use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{
    image::Image,
    menu::{MenuBuilder, MenuItemBuilder},
    tray::TrayIconBuilder,
    App, Manager,
};

pub static QUIT_REQUESTED: AtomicBool = AtomicBool::new(false);

pub mod ids {
    pub const SHOW_WINDOW: &str = "tray-show-window";
    pub const QUIT: &str = "tray-quit";
}

pub fn setup_tray(app: &App) -> Result<(), Box<dyn std::error::Error>> {
    let handle = app.handle();

    let show_window = MenuItemBuilder::with_id(ids::SHOW_WINDOW, "Show Releash").build(handle)?;
    let quit = MenuItemBuilder::with_id(ids::QUIT, "Quit").build(handle)?;

    let menu = MenuBuilder::new(handle)
        .item(&show_window)
        .separator()
        .item(&quit)
        .build()?;

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
        ids::QUIT => {
            QUIT_REQUESTED.store(true, Ordering::SeqCst);
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                // Kill all agent sessions before stopping the server
                if let Some(handles) = app.try_state::<std::sync::Arc<
                    tokio::sync::Mutex<
                        crate::infrastructure::agent_session::runtime::AgentProcessMap,
                    >,
                >>() {
                    crate::infrastructure::agent_session::runtime::close_all_agent_sessions(
                        &app,
                        handles.inner(),
                    )
                    .await;
                }
                app.exit(0);
            });
        }
        _ => {}
    }
}
