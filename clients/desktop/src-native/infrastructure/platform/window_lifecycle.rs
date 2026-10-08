use std::sync::atomic::Ordering;

use tauri::Manager;

pub const NORMAL_WINDOW_LABEL: &str = "main";
pub const STARTUP_FAILURE_WINDOW_LABEL: &str = "startup-failure";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartupVisibilityAction {
    Hide,
    Minimize,
}

fn apply_window_view_close(
    action: StartupVisibilityAction,
    hide: impl FnOnce(),
    minimize: impl FnOnce(),
) {
    match action {
        StartupVisibilityAction::Hide => hide(),
        StartupVisibilityAction::Minimize => minimize(),
    }
}

pub fn handle_run_event<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    event: tauri::RunEvent,
    quit: impl FnOnce(i32),
    show: impl FnOnce(),
) {
    match event {
        tauri::RunEvent::Exit => log::logger().flush(),
        tauri::RunEvent::WindowEvent {
            event: tauri::WindowEvent::CloseRequested { api, .. },
            label,
            ..
        } => {
            api.prevent_close();
            close_window_to_configured_destination(app, &label);
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowPreferences {
    pub close_to_tray: bool,
}

pub struct WindowPreferencesState(pub(crate) parking_lot::RwLock<WindowPreferences>);

impl WindowPreferencesState {
    pub fn read(&self) -> WindowPreferences {
        *self.0.read()
    }
}

fn close_window_to_configured_destination<R: tauri::Runtime>(
    app_handle: &tauri::AppHandle<R>,
    label: &str,
) {
    let preferences = app_handle.state::<WindowPreferencesState>();
    let preferences = preferences.read();

    if let Some(window) = app_handle.get_webview_window(label) {
        apply_window_view_close(
            startup_visibility_action(preferences.close_to_tray),
            || {
                let _ = window.hide();
            },
            || {
                let _ = window.minimize();
            },
        );
    }
}

fn startup_visibility_action(close_to_tray: bool) -> StartupVisibilityAction {
    if close_to_tray {
        StartupVisibilityAction::Hide
    } else {
        StartupVisibilityAction::Minimize
    }
}

pub(crate) fn should_prevent_exit() -> bool {
    !super::tray::QUIT_REQUESTED.load(Ordering::SeqCst)
}

#[cfg(test)]
#[path = "window_lifecycle_test.rs"]
mod window_lifecycle_tests;
