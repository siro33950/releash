use std::sync::atomic::Ordering;

use tauri::Manager;

pub(crate) const NORMAL_WINDOW_LABEL: &str = "main";
pub(crate) const STARTUP_FAILURE_WINDOW_LABEL: &str = "startup-failure";

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
pub(crate) struct WindowPreferences {
    pub(crate) close_to_tray: bool,
}

pub(crate) struct WindowPreferencesState(pub(crate) parking_lot::RwLock<WindowPreferences>);

impl WindowPreferencesState {
    pub(crate) fn read(&self) -> WindowPreferences {
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
mod tests {
    use super::*;

    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

    #[test]
    fn test_ウィンドウ閉鎖_close_requestedを処理しても終了要求せず再表示できる() {
        for close_to_tray in [true, false] {
            for label in [NORMAL_WINDOW_LABEL, STARTUP_FAILURE_WINDOW_LABEL] {
                // Given
                let app = tauri::test::mock_builder()
                    .manage(WindowPreferencesState(parking_lot::RwLock::new(
                        WindowPreferences { close_to_tray },
                    )))
                    .build(tauri::test::mock_context(tauri::test::noop_assets()))
                    .unwrap();
                let window = tauri::WebviewWindowBuilder::new(&app, label, Default::default())
                    .build()
                    .unwrap();
                let closed = std::sync::Arc::new(AtomicUsize::new(0));
                let observed = closed.clone();
                let mut cleanup = false;
                // When
                app.run_return(move |app, event| match event {
                    tauri::RunEvent::Ready => window.close().unwrap(),
                    event @ tauri::RunEvent::WindowEvent {
                        event: tauri::WindowEvent::CloseRequested { .. },
                        ..
                    } => {
                        handle_run_event(
                            app,
                            event,
                            |_| panic!("CloseRequested must not enter application quit"),
                            || panic!("CloseRequested must not reopen the window"),
                        );
                        observed.fetch_add(1, AtomicOrdering::SeqCst);
                    }
                    tauri::RunEvent::MainEventsCleared
                        if observed.load(AtomicOrdering::SeqCst) == 1 && !cleanup =>
                    {
                        // Then: prevent_close preserved the window through the event loop.
                        let window = app.get_webview_window(label).expect("window was destroyed");
                        window.show().unwrap();
                        window.unminimize().unwrap();
                        cleanup = true;
                        window.destroy().unwrap();
                    }
                    tauri::RunEvent::ExitRequested { .. } if !cleanup => {
                        panic!("window close must not request application exit");
                    }
                    _ => {}
                });
                assert_eq!(closed.load(AtomicOrdering::SeqCst), 1);
            }
        }
    }

    #[test]
    fn close_quit_window_close_is_view_only() {
        for (close_to_tray, expected_hidden, expected_minimized) in [(true, 1, 0), (false, 0, 1)] {
            let hidden = AtomicUsize::new(0);
            let minimized = AtomicUsize::new(0);

            apply_window_view_close(
                startup_visibility_action(close_to_tray),
                || {
                    hidden.fetch_add(1, AtomicOrdering::SeqCst);
                },
                || {
                    minimized.fetch_add(1, AtomicOrdering::SeqCst);
                },
            );

            assert_eq!(hidden.load(AtomicOrdering::SeqCst), expected_hidden);
            assert_eq!(minimized.load(AtomicOrdering::SeqCst), expected_minimized);
        }
    }

    #[test]
    fn startup_visibility_action_follows_close_to_tray_policy() {
        assert_eq!(
            startup_visibility_action(true),
            StartupVisibilityAction::Hide
        );
        assert_eq!(
            startup_visibility_action(false),
            StartupVisibilityAction::Minimize
        );
    }

    #[test]
    fn exit_is_prevented_until_tray_quit_is_requested() {
        let _guard = super::super::tray::QUIT_REQUESTED_TEST_LOCK.lock().unwrap();
        super::super::tray::QUIT_REQUESTED.store(false, Ordering::SeqCst);
        assert!(should_prevent_exit());

        super::super::tray::QUIT_REQUESTED.store(true, Ordering::SeqCst);
        assert!(!should_prevent_exit());

        super::super::tray::QUIT_REQUESTED.store(false, Ordering::SeqCst);
    }
}
