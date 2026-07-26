use std::sync::atomic::Ordering;
use std::sync::Arc;

use tauri::Manager;

use crate::domain::app_config::ConfigRepository;

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

pub(crate) fn native_exit_intent(
    code: Option<i32>,
) -> crate::usecase::shutdown_coordinator::ApplicationQuitIntent {
    crate::usecase::shutdown_coordinator::ApplicationQuitIntent::Exit {
        code: code.unwrap_or(0),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NativeExitRoute {
    AlreadyGranted,
    StartupFailure,
    NormalApplication,
    Unavailable,
}

fn dispatch_native_exit(
    authority: Option<&crate::usecase::application_startup::ApplicationStartupAuthority>,
    dispatch_normal_application_quit: impl FnOnce() -> bool,
) -> NativeExitRoute {
    match authority {
        Some(authority) if authority.normal_admission_ready() => {
            if dispatch_normal_application_quit() {
                NativeExitRoute::NormalApplication
            } else {
                NativeExitRoute::Unavailable
            }
        }
        Some(authority) => {
            let _ = authority.quit_after_failure();
            NativeExitRoute::StartupFailure
        }
        None => NativeExitRoute::Unavailable,
    }
}

fn handle_native_exit_requested(
    should_prevent: bool,
    authority: Option<&crate::usecase::application_startup::ApplicationStartupAuthority>,
    prevent_native_exit: impl FnOnce(),
    dispatch_normal_application_quit: impl FnOnce() -> bool,
) -> NativeExitRoute {
    if !should_prevent {
        return NativeExitRoute::AlreadyGranted;
    }
    prevent_native_exit();
    dispatch_native_exit(authority, dispatch_normal_application_quit)
}

fn active_window_label(
    authority: Option<&crate::usecase::application_startup::ApplicationStartupAuthority>,
) -> Option<&'static str> {
    authority.map(|authority| {
        if authority.normal_admission_ready() {
            NORMAL_WINDOW_LABEL
        } else {
            STARTUP_FAILURE_WINDOW_LABEL
        }
    })
}

#[cfg(any(target_os = "macos", test))]
fn show_and_focus_active_surface(
    authority: Option<&crate::usecase::application_startup::ApplicationStartupAuthority>,
    show_and_focus: impl FnOnce(&str),
) -> bool {
    let Some(label) = active_window_label(authority) else {
        return false;
    };
    show_and_focus(label);
    true
}

pub fn handle_run_event(app_handle: &tauri::AppHandle, event: tauri::RunEvent) {
    match event {
        tauri::RunEvent::WindowEvent {
            event: tauri::WindowEvent::CloseRequested { api, .. },
            label,
            ..
        } => {
            api.prevent_close();
            let authority = app_handle
                .try_state::<Arc<crate::usecase::application_startup::ApplicationStartupAuthority>>(
                );
            match active_window_label(
                authority
                    .as_ref()
                    .map(|authority| authority.inner().as_ref()),
            ) {
                Some(STARTUP_FAILURE_WINDOW_LABEL) => {
                    // S11 is the only safe failure surface and there is no
                    // normal tray/workbench fallback. Keep it reachable.
                    show_and_focus_window(app_handle, STARTUP_FAILURE_WINDOW_LABEL);
                }
                Some(NORMAL_WINDOW_LABEL) => {
                    close_window_to_configured_destination(app_handle, &label);
                }
                _ => {}
            }
        }
        tauri::RunEvent::ExitRequested { api, code, .. } => {
            let authority = app_handle
                .try_state::<Arc<crate::usecase::application_startup::ApplicationStartupAuthority>>(
                );
            handle_native_exit_requested(
                should_prevent_exit(),
                authority
                    .as_ref()
                    .map(|authority| authority.inner().as_ref()),
                || api.prevent_exit(),
                || {
                    if let Some(ingress) = app_handle.try_state::<Arc<
                        crate::adaptor::controller::application_lifecycle::ApplicationQuitIngress,
                    >>() {
                        ingress.request(native_exit_intent(code));
                        true
                    } else {
                        false
                    }
                },
            );
        }
        #[cfg(target_os = "macos")]
        tauri::RunEvent::Reopen { .. } => {
            show_and_focus_active_window(app_handle);
        }
        _ => {}
    }
}

#[cfg(test)]
mod native_exit_tests {
    use super::{
        active_window_label, handle_native_exit_requested, native_exit_intent, NativeExitRoute,
        NORMAL_WINDOW_LABEL, STARTUP_FAILURE_WINDOW_LABEL,
    };
    use crate::usecase::shutdown_coordinator::ApplicationQuitIntent;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn native_exit_preserves_signed_code() {
        for code in [i32::MIN, -1, 0, 1, i32::MAX] {
            assert_eq!(
                native_exit_intent(Some(code)),
                ApplicationQuitIntent::Exit { code }
            );
        }
        assert_eq!(
            native_exit_intent(None),
            ApplicationQuitIntent::Exit { code: 0 }
        );
    }

    #[derive(Default)]
    struct RecordingFailureExit {
        calls: AtomicUsize,
    }

    impl crate::usecase::application_startup::ProcessLocalExitPort for RecordingFailureExit {
        fn exit(&self, code: i32) {
            assert_eq!(code, 1);
            self.calls.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn native_quit_during_startup_failure_uses_only_the_process_local_one_shot() {
        let exit = Arc::new(RecordingFailureExit::default());
        let authority = crate::usecase::application_startup::ApplicationStartupAuthority::failed(
            crate::usecase::application_startup::StartupFailureKind::StorageUnavailable,
            exit.clone(),
        );
        let normal_quit_calls = AtomicUsize::new(0);
        let prevented_native_exits = AtomicUsize::new(0);

        for _ in 0..8 {
            assert_eq!(
                handle_native_exit_requested(
                    true,
                    Some(&authority),
                    || {
                        prevented_native_exits.fetch_add(1, Ordering::SeqCst);
                    },
                    || {
                        normal_quit_calls.fetch_add(1, Ordering::SeqCst);
                        true
                    },
                ),
                NativeExitRoute::StartupFailure
            );
            assert_eq!(exit.calls.load(Ordering::SeqCst), 1);
            assert_eq!(normal_quit_calls.load(Ordering::SeqCst), 0);
        }
        assert_eq!(prevented_native_exits.load(Ordering::SeqCst), 8);

        assert_eq!(
            handle_native_exit_requested(
                false,
                Some(&authority),
                || {
                    prevented_native_exits.fetch_add(1, Ordering::SeqCst);
                },
                || {
                    normal_quit_calls.fetch_add(1, Ordering::SeqCst);
                    true
                },
            ),
            NativeExitRoute::AlreadyGranted
        );
        assert_eq!(prevented_native_exits.load(Ordering::SeqCst), 8);
        assert_eq!(exit.calls.load(Ordering::SeqCst), 1);
        assert_eq!(normal_quit_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn ready_authority_without_normal_quit_ingress_fails_closed() {
        let authority = crate::usecase::application_startup::ApplicationStartupAuthority::ready();
        assert_eq!(
            handle_native_exit_requested(true, Some(&authority), || {}, || false),
            NativeExitRoute::Unavailable
        );
    }

    #[test]
    fn reopen_and_close_keep_the_authoritative_surface_reachable() {
        let ready = crate::usecase::application_startup::ApplicationStartupAuthority::ready();
        let failed = crate::usecase::application_startup::ApplicationStartupAuthority::failed_kind(
            crate::usecase::application_startup::StartupFailureKind::StorageUnavailable,
        );
        assert_eq!(active_window_label(Some(&ready)), Some(NORMAL_WINDOW_LABEL));
        assert_eq!(
            active_window_label(Some(&failed)),
            Some(STARTUP_FAILURE_WINDOW_LABEL)
        );
        assert_eq!(active_window_label(None), None);
        let shown = std::sync::Mutex::new(Vec::new());
        assert!(super::show_and_focus_active_surface(
            Some(&failed),
            |label| shown.lock().unwrap().push(label.to_string()),
        ));
        assert_eq!(
            shown.lock().unwrap().as_slice(),
            [STARTUP_FAILURE_WINDOW_LABEL]
        );
        assert!(!super::show_and_focus_active_surface(None, |_| {
            panic!("missing authority must not show a fail-open window")
        }));
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
        apply_window_view_close(
            startup_visibility_action(close_to_tray),
            || {
                let _ = window.hide();
            },
            || {
                let _ = window.minimize();
            },
        );
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

pub(crate) fn should_prevent_exit() -> bool {
    !super::tray::QUIT_REQUESTED.load(Ordering::SeqCst)
}

fn show_and_focus_window<R: tauri::Runtime>(app_handle: &tauri::AppHandle<R>, label: &str) {
    if let Some(window) = app_handle.get_webview_window(label) {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[cfg(target_os = "macos")]
fn show_and_focus_active_window<R: tauri::Runtime>(app_handle: &tauri::AppHandle<R>) {
    let authority = app_handle
        .try_state::<Arc<crate::usecase::application_startup::ApplicationStartupAuthority>>();
    show_and_focus_active_surface(
        authority
            .as_ref()
            .map(|authority| authority.inner().as_ref()),
        |label| show_and_focus_window(app_handle, label),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

    use crate::domain::app_config::repository::ConfigUpdate;
    use crate::domain::app_config::value_objects::{
        AppConfigDocument, AppSettings, ServerConfig, TelemetryConfig, TlsConfig, WorkflowConfig,
    };
    use crate::domain::app_config::AppConfigError;
    use crate::domain::notification::{DesktopNotifyMode, NotifyConfig};

    struct StubConfigRepository {
        config: AppConfigDocument,
    }

    #[test]
    fn close_quit_window_close_is_view_only() {
        for (close_to_tray, expected_hidden, expected_minimized) in [(true, 1, 0), (false, 0, 1)] {
            let hidden = AtomicUsize::new(0);
            let minimized = AtomicUsize::new(0);
            let application_quit_ingress = AtomicUsize::new(0);

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
            assert_eq!(
                application_quit_ingress.load(AtomicOrdering::SeqCst),
                0,
                "CloseRequested is a view-only visibility effect and cannot enter quit"
            );
        }
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
        let _guard = super::super::tray::QUIT_REQUESTED_TEST_LOCK.lock().unwrap();
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
