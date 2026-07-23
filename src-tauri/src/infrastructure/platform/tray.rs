use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tauri::{
    image::Image,
    menu::{MenuBuilder, MenuItemBuilder},
    tray::TrayIconBuilder,
    App, Manager,
};

pub static QUIT_REQUESTED: AtomicBool = AtomicBool::new(false);
#[cfg(test)]
pub(crate) static QUIT_REQUESTED_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(crate) fn mark_quit_requested() {
    QUIT_REQUESTED.store(true, Ordering::SeqCst);
}

pub mod ids {
    pub const SHOW_WINDOW: &str = "tray-show-window";
    pub const QUIT: &str = "tray-quit";
}

type QuitHandler = Arc<dyn Fn(tauri::AppHandle) + Send + Sync + 'static>;

pub fn setup_tray(
    app: &App,
    on_quit_requested: impl Fn(tauri::AppHandle) + Send + Sync + 'static,
) -> Result<(), Box<dyn std::error::Error>> {
    let handle = app.handle();
    let on_quit_requested: QuitHandler = Arc::new(on_quit_requested);

    let show_window = MenuItemBuilder::with_id(ids::SHOW_WINDOW, "Show Releash").build(handle)?;
    let quit = MenuItemBuilder::with_id(ids::QUIT, "Quit").build(handle)?;

    let menu = MenuBuilder::new(handle)
        .item(&show_window)
        .separator()
        .item(&quit)
        .build()?;

    let icon = Image::from_bytes(include_bytes!("../../../icons/32x32.png"))?;

    TrayIconBuilder::new()
        .icon(icon)
        .menu(&menu)
        .tooltip("Releash")
        .on_menu_event(move |app, event| {
            handle_menu_event(app, event, Arc::clone(&on_quit_requested));
        })
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

fn handle_menu_event(
    app: &tauri::AppHandle,
    event: tauri::menu::MenuEvent,
    on_quit_requested: QuitHandler,
) {
    match event.id().as_ref() {
        ids::SHOW_WINDOW => {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }
        ids::QUIT => {
            on_quit_requested(app.clone());
        }
        _ => {}
    }
}
