use std::sync::atomic::Ordering;

use tauri::Manager;

pub const NORMAL_WINDOW_LABEL: &str = "main";
pub const STARTUP_FAILURE_WINDOW_LABEL: &str = "startup-failure";

pub fn handle_run_event<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    event: tauri::RunEvent,
    quit: impl FnOnce(i32),
    show: impl FnOnce(),
) {
    handle_run_event_with_window(event, quit, show, |label| app.get_webview_window(label));
}

trait WindowVisibility {
    fn hide(&self) -> Result<(), tauri::Error>;
}

impl<R: tauri::Runtime> WindowVisibility for tauri::WebviewWindow<R> {
    fn hide(&self) -> Result<(), tauri::Error> {
        tauri::WebviewWindow::hide(self)
    }
}

fn handle_run_event_with_window<W: WindowVisibility>(
    event: tauri::RunEvent,
    quit: impl FnOnce(i32),
    show: impl FnOnce(),
    window: impl FnOnce(&str) -> Option<W>,
) {
    match event {
        tauri::RunEvent::Exit => log::logger().flush(),
        tauri::RunEvent::WindowEvent {
            event: tauri::WindowEvent::CloseRequested { api, .. },
            label,
            ..
        } => {
            api.prevent_close();
            if let Some(window) = window(&label) {
                let _ = window.hide();
            }
        }
        tauri::RunEvent::ExitRequested { api, code, .. } if should_prevent_exit() => {
            api.prevent_exit();
            quit(code.unwrap_or(0));
        }
        #[cfg(target_os = "macos")]
        tauri::RunEvent::Reopen { .. } => show(),
        _ => {
            let _ = show;
        }
    }
}

pub(crate) fn should_prevent_exit() -> bool {
    !super::tray::QUIT_REQUESTED.load(Ordering::SeqCst)
}

#[cfg(test)]
#[path = "window_lifecycle_test.rs"]
mod window_lifecycle_tests;
