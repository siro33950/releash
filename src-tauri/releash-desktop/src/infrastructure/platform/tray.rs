use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tauri::{
    image::Image,
    menu::{MenuBuilder, MenuItemBuilder},
    tray::TrayIconBuilder,
    App,
};

pub static QUIT_REQUESTED: AtomicBool = AtomicBool::new(false);
pub(crate) const ICON: &[u8] = include_bytes!("../../../icons/tray-template.png");
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
    on_show: impl Fn(tauri::AppHandle) + Send + Sync + 'static,
) -> Result<(), Box<dyn std::error::Error>> {
    let handle = app.handle();
    let on_show: QuitHandler = Arc::new(on_show);
    let menu_show = on_show.clone();
    let on_quit_requested: QuitHandler = Arc::new(on_quit_requested);

    let show_window = MenuItemBuilder::with_id(ids::SHOW_WINDOW, "Show Releash").build(handle)?;
    let quit = MenuItemBuilder::with_id(ids::QUIT, "Quit").build(handle)?;

    let menu = MenuBuilder::new(handle)
        .item(&show_window)
        .separator()
        .item(&quit)
        .build()?;

    let icon = Image::from_bytes(ICON)?;

    TrayIconBuilder::new()
        .icon(icon)
        .icon_as_template(true)
        .menu(&menu)
        .tooltip("Releash")
        .on_menu_event(move |app, event| {
            dispatch_menu_event(
                event.id().as_ref(),
                || menu_show(app.clone()),
                || on_quit_requested(app.clone()),
            );
        })
        .on_tray_icon_event(move |tray, event| {
            if let tauri::tray::TrayIconEvent::DoubleClick { .. } = event {
                on_show(tray.app_handle().clone());
            }
        })
        .build(app)?;

    Ok(())
}

pub(crate) fn dispatch_menu_event(id: &str, show: impl FnOnce(), quit: impl FnOnce()) {
    match id {
        ids::SHOW_WINDOW => show(),
        ids::QUIT => quit(),
        _ => {}
    }
}
