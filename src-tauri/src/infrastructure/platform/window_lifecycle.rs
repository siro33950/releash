use std::sync::atomic::Ordering;
use std::sync::Arc;

use tauri::Manager;

use crate::domain::app_config::ConfigRepository;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartupVisibilityAction {
    Hide,
    Minimize,
}

pub fn handle_run_event(app_handle: &tauri::AppHandle, event: tauri::RunEvent) {
    match event {
        tauri::RunEvent::WindowEvent {
            event: tauri::WindowEvent::CloseRequested { api, .. },
            label,
            ..
        } => {
            api.prevent_close();
            close_window_to_configured_destination(app_handle, &label);
        }
        tauri::RunEvent::ExitRequested { api, .. } if should_prevent_exit() => {
            api.prevent_exit();
        }
        #[cfg(target_os = "macos")]
        tauri::RunEvent::Reopen { .. } => {
            show_and_focus_main_window(app_handle);
        }
        _ => {}
    }
}

pub fn apply_startup_visibility(
    app_handle: &tauri::AppHandle,
    config_repository: &dyn ConfigRepository,
) {
    if !is_hidden_startup(std::env::args()) {
        return;
    }

    if let Some(action) = hidden_startup_visibility_action(config_repository) {
        if let Some(window) = app_handle.get_webview_window("main") {
            match action {
                StartupVisibilityAction::Hide => {
                    let _ = window.hide();
                }
                StartupVisibilityAction::Minimize => {
                    let _ = window.minimize();
                }
            }
        }
    }
}

fn close_window_to_configured_destination(app_handle: &tauri::AppHandle, label: &str) {
    let close_to_tray = app_handle
        .try_state::<Arc<dyn ConfigRepository>>()
        .and_then(|cfg| cfg.load().ok())
        .is_none_or(|c| c.app.close_to_tray);

    if let Some(window) = app_handle.get_webview_window(label) {
        if close_to_tray {
            let _ = window.hide();
        } else {
            let _ = window.minimize();
        }
    }
}

fn is_hidden_startup(args: impl IntoIterator<Item = String>) -> bool {
    args.into_iter().any(|arg| arg == "--hidden")
}

fn hidden_startup_visibility_action(
    config_repository: &dyn ConfigRepository,
) -> Option<StartupVisibilityAction> {
    let start_minimized = config_repository
        .load()
        .is_ok_and(|config| config.app.start_minimized);

    if !start_minimized {
        return None;
    }

    let close_to_tray = config_repository
        .load()
        .is_ok_and(|config| config.app.close_to_tray);

    Some(startup_visibility_action(close_to_tray))
}

fn startup_visibility_action(close_to_tray: bool) -> StartupVisibilityAction {
    if close_to_tray {
        StartupVisibilityAction::Hide
    } else {
        StartupVisibilityAction::Minimize
    }
}

fn should_prevent_exit() -> bool {
    !super::tray::QUIT_REQUESTED.load(Ordering::SeqCst)
}

#[cfg(target_os = "macos")]
fn show_and_focus_main_window(app_handle: &tauri::AppHandle) {
    if let Some(window) = app_handle.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::domain::app_config::repository::ConfigUpdate;
    use crate::domain::app_config::value_objects::{
        AppConfigDocument, AppSettings, ServerConfig, TelemetryConfig, TlsConfig, WorkflowConfig,
    };
    use crate::domain::app_config::AppConfigError;
    use crate::domain::notification::{DesktopNotifyMode, NotifyConfig};

    struct StubConfigRepository {
        config: AppConfigDocument,
    }

    impl ConfigRepository for StubConfigRepository {
        fn load(&self) -> Result<AppConfigDocument, AppConfigError> {
            Ok(self.config.clone())
        }

        fn save(&self, _config: AppConfigDocument) -> Result<(), AppConfigError> {
            Err(AppConfigError::Repository(
                "save is not used in window lifecycle tests".to_string(),
            ))
        }

        fn update(&self, _f: ConfigUpdate) -> Result<(), AppConfigError> {
            Err(AppConfigError::Repository(
                "update is not used in window lifecycle tests".to_string(),
            ))
        }
    }

    #[test]
    fn hidden_startup_detects_hidden_flag_only() {
        assert!(is_hidden_startup([
            "releash".to_string(),
            "--hidden".to_string()
        ]));
        assert!(!is_hidden_startup([
            "releash".to_string(),
            "--other".to_string()
        ]));
    }

    #[test]
    fn hidden_startup_visibility_action_requires_start_minimized() {
        let repository = StubConfigRepository {
            config: config_with_startup_policy(false, true),
        };

        assert_eq!(hidden_startup_visibility_action(&repository), None);
    }

    #[test]
    fn hidden_startup_visibility_action_follows_close_to_tray_policy() {
        let close_to_tray_repository = StubConfigRepository {
            config: config_with_startup_policy(true, true),
        };
        let minimize_repository = StubConfigRepository {
            config: config_with_startup_policy(true, false),
        };

        assert_eq!(
            hidden_startup_visibility_action(&close_to_tray_repository),
            Some(StartupVisibilityAction::Hide)
        );
        assert_eq!(
            hidden_startup_visibility_action(&minimize_repository),
            Some(StartupVisibilityAction::Minimize)
        );
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
        super::super::tray::QUIT_REQUESTED.store(false, Ordering::SeqCst);
        assert!(should_prevent_exit());

        super::super::tray::QUIT_REQUESTED.store(true, Ordering::SeqCst);
        assert!(!should_prevent_exit());

        super::super::tray::QUIT_REQUESTED.store(false, Ordering::SeqCst);
    }

    fn config_with_startup_policy(start_minimized: bool, close_to_tray: bool) -> AppConfigDocument {
        AppConfigDocument {
            server: ServerConfig {
                bind: "127.0.0.1".to_string(),
                port: 0,
                hook_port: 0,
                token: String::new(),
                tls: TlsConfig {
                    enabled: false,
                    cert: String::new(),
                    key: String::new(),
                },
                notify: NotifyConfig {
                    webhook_url: String::new(),
                    on_running: false,
                    on_done: false,
                    on_error: false,
                    on_waiting: false,
                    desktop_mode: DesktopNotifyMode::Always,
                    inactive_timeout_minutes: 0,
                },
            },
            telemetry: TelemetryConfig {
                crash_reporting: false,
                performance_telemetry: false,
            },
            app: AppSettings {
                close_to_tray,
                auto_launch: false,
                start_minimized,
                last_root_path: String::new(),
                last_repo_paths: Vec::new(),
                external_editor: String::new(),
            },
            workflow: WorkflowConfig {
                approval_auto_approve: false,
            },
        }
    }
}
