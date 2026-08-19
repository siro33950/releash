mod adaptor;
#[cfg(debug_assertions)]
pub mod agent_session_tui_acceptance;
pub mod cli;
mod domain;
mod infrastructure;
mod other;
#[cfg(debug_assertions)]
pub mod provider_lifecycle_acceptance;
#[cfg(debug_assertions)]
pub mod workflow_control_plane_acceptance;
pub mod terminal_surface {
    pub use crate::adaptor::controller::terminal_surface_runtime::{
        TerminalSurfaceEventFault, TerminalSurfaceEventFaultController, TerminalSurfaceRuntime,
        TerminalSurfaceWireAttachment,
    };
    pub use crate::adaptor::protocol::terminal::{
        GetOrSpawnTerminalV1, TerminalProcessLaunchV1, TerminalSurfaceOwnerV1,
        TerminalSurfaceStreamItemV1, TerminalSurfaceV1,
    };
}
// Test-only helpers are intentionally kept as a root module.
#[cfg(test)]
mod test_support;
mod usecase;

use std::sync::Arc;
use std::time::Instant;

use adaptor::gateway::app_config::{load_or_create_config, AppConfig};
use domain::app_config::{ConfigRepository, ConfigSecretRepository, NotionConfigRepository};
use infrastructure::platform::window_lifecycle::{
    NORMAL_WINDOW_LABEL, STARTUP_FAILURE_WINDOW_LABEL,
};
use tauri::Manager;

type LocalApiShutdownTarget = Arc<parking_lot::RwLock<Option<Arc<dyn Fn() + Send + Sync>>>>;

fn application_context<R: tauri::Runtime>() -> tauri::Context<R> {
    tauri::generate_context!()
}

fn create_configured_window(
    app: &tauri::App,
    label: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "performance-wdio")]
    if label == NORMAL_WINDOW_LABEL && app.get_webview_window(label).is_some() {
        return Ok(());
    }
    let mut config = app
        .config()
        .app
        .windows
        .first()
        .cloned()
        .ok_or_else(|| std::io::Error::other("application window configuration is missing"))?;
    config.label = label.to_string();
    config.create = true;
    config.closable = label != STARTUP_FAILURE_WINDOW_LABEL;
    tauri::WebviewWindowBuilder::from_config(app.handle(), &config)?.build()?;
    Ok(())
}

struct StartupFailureSurface {
    authority: Arc<usecase::application_startup::ApplicationStartupAuthority>,
    window_label: &'static str,
}

impl StartupFailureSurface {
    fn new(
        kind: usecase::application_startup::StartupFailureKind,
        failure_exit: Arc<dyn usecase::application_startup::ProcessLocalExitPort>,
    ) -> Self {
        Self {
            authority: Arc::new(
                usecase::application_startup::ApplicationStartupAuthority::failed(
                    kind,
                    failure_exit,
                ),
            ),
            window_label: STARTUP_FAILURE_WINDOW_LABEL,
        }
    }
}

struct ReadyStartupStore {
    data_dir: std::path::PathBuf,
    app_data: adaptor::controller::app_data_composition::ProductionAppDataComposition,
    local_event_store: Arc<adaptor::gateway::local_event_store::LocalEventStore>,
}

enum StartupStoreAdmission {
    Ready(ReadyStartupStore),
    Failed(StartupFailureSurface),
}

fn compose_startup_store_admission<PathError>(
    resolve_app_data_dir: impl FnOnce() -> Result<std::path::PathBuf, PathError>,
    open_local_event_store: impl FnOnce(
        &adaptor::controller::app_data_composition::ProductionAppDataComposition,
    ) -> Result<
        Arc<adaptor::gateway::local_event_store::LocalEventStore>,
        adaptor::gateway::local_event_store::store::LocalEventStoreOpenError,
    >,
    failure_exit: Arc<dyn usecase::application_startup::ProcessLocalExitPort>,
) -> StartupStoreAdmission {
    let data_dir = match resolve_app_data_dir() {
        Ok(data_dir) => data_dir,
        Err(_) => {
            return StartupStoreAdmission::Failed(StartupFailureSurface::new(
                usecase::application_startup::StartupFailureKind::StorageUnavailable,
                failure_exit,
            ));
        }
    };
    let app_data = adaptor::controller::app_data_composition::ProductionAppDataComposition::new(
        data_dir.clone(),
    );
    let local_event_store = match open_local_event_store(&app_data) {
        Ok(local_event_store) => local_event_store,
        Err(error) => {
            return StartupStoreAdmission::Failed(StartupFailureSurface::new(
                classify_startup_failure(error),
                failure_exit,
            ));
        }
    };

    StartupStoreAdmission::Ready(ReadyStartupStore {
        data_dir,
        app_data,
        local_event_store,
    })
}

fn install_failed_startup_surface(
    app: &tauri::App,
    surface: StartupFailureSurface,
) -> Result<(), Box<dyn std::error::Error>> {
    let StartupFailureSurface {
        authority,
        window_label,
    } = surface;
    if !app.manage(authority) {
        return Err(
            std::io::Error::other("application startup authority was already installed").into(),
        );
    }
    create_configured_window(app, window_label)
}

fn normal_startup_effect<T>(
    authority: &usecase::application_startup::ApplicationStartupAuthority,
    effect: impl FnOnce() -> T,
) -> Option<T> {
    authority.normal_admission_ready().then(effect)
}

fn classify_startup_failure(
    error: adaptor::gateway::local_event_store::store::LocalEventStoreOpenError,
) -> usecase::application_startup::StartupFailureKind {
    use adaptor::gateway::local_event_store::store::LocalEventStoreOpenError as E;
    use usecase::application_startup::StartupFailureKind as K;

    match error {
        E::WriterLockHeld => K::StoreInUse,
        E::StorageUnavailable => K::StorageUnavailable,
        E::UnsupportedRuntime => K::UnsupportedRuntime,
        E::UnsupportedStoreVersion => K::UnsupportedStoreVersion,
        E::InitializationStateInvalid => K::InitializationStateInvalid,
        E::StoreValidationFailed => K::StoreValidationFailed,
        E::SchemaEvolutionFailed => K::SchemaEvolutionFailed,
    }
}

fn select_provider_agent_executables(
    claude_executable: String,
    codex_executable: String,
    fixture_executable: Option<String>,
) -> (String, String) {
    match fixture_executable.filter(|path| !path.trim().is_empty()) {
        Some(path) => (path.clone(), path),
        None => (claude_executable, codex_executable),
    }
}

#[cfg(feature = "performance-wdio")]
fn performance_provider_fixture_executable() -> Option<String> {
    std::env::var("RELEASH_PERFORMANCE_PROVIDER_FIXTURE_EXECUTABLE").ok()
}

#[cfg(not(feature = "performance-wdio"))]
fn performance_provider_fixture_executable() -> Option<String> {
    None
}

#[cfg(test)]
mod startup_composition_tests {
    use super::*;
    use adaptor::gateway::local_event_store::store::LocalEventStoreOpenError as E;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use usecase::application_startup::StartupFailureKind as K;

    #[test]
    fn test_performance_fixture_replaces_both_provider_executables_without_affecting_defaults() {
        assert_eq!(
            select_provider_agent_executables(
                "claude".to_string(),
                "codex".to_string(),
                Some("/tmp/provider-fixture".to_string()),
            ),
            (
                "/tmp/provider-fixture".to_string(),
                "/tmp/provider-fixture".to_string(),
            )
        );
        assert_eq!(
            select_provider_agent_executables("claude".to_string(), "codex".to_string(), None,),
            ("claude".to_string(), "codex".to_string())
        );
    }

    #[derive(Default)]
    struct RecordingProcessLocalExitPort {
        calls: AtomicUsize,
    }

    impl usecase::application_startup::ProcessLocalExitPort for RecordingProcessLocalExitPort {
        fn exit(&self, code: i32) {
            assert_eq!(code, 1);
            self.calls.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[tauri::command]
    fn record_path_failure_normal_effect(
        effects: tauri::State<'_, Arc<AtomicUsize>>,
    ) -> &'static str {
        effects.fetch_add(1, Ordering::SeqCst);
        "normal-effect-ran"
    }

    fn invoke_request(command: &str, body: serde_json::Value) -> tauri::webview::InvokeRequest {
        tauri::webview::InvokeRequest {
            cmd: command.to_string(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            url: if cfg!(any(windows, target_os = "android")) {
                "http://tauri.localhost"
            } else {
                "tauri://localhost"
            }
            .parse()
            .unwrap(),
            body: tauri::ipc::InvokeBody::Json(body),
            headers: Default::default(),
            invoke_key: tauri::test::INVOKE_KEY.to_string(),
        }
    }

    fn startup_failure_composition_handler(
    ) -> impl Fn(tauri::ipc::Invoke<tauri::test::MockRuntime>) -> bool + Send + Sync + 'static {
        tauri::generate_handler![
            adaptor::controller::command::application_lifecycle::get_application_startup_outcome,
            adaptor::controller::command::application_lifecycle::quit_after_startup_failure,
            record_path_failure_normal_effect
        ]
    }

    #[test]
    fn b071_app_data_path_failure_composes_the_two_command_safe_surface_without_effects() {
        let path_resolution_effects = AtomicUsize::new(0);
        let store_open_effects = AtomicUsize::new(0);
        let exit = Arc::new(RecordingProcessLocalExitPort::default());
        let failure_exit: Arc<dyn usecase::application_startup::ProcessLocalExitPort> =
            exit.clone();

        let admission = compose_startup_store_admission(
            || -> Result<std::path::PathBuf, &'static str> {
                path_resolution_effects.fetch_add(1, Ordering::SeqCst);
                Err("path unavailable")
            },
            |_app_data| -> Result<Arc<adaptor::gateway::local_event_store::LocalEventStore>, E> {
                store_open_effects.fetch_add(1, Ordering::SeqCst);
                Err(E::StoreValidationFailed)
            },
            failure_exit,
        );
        let StartupStoreAdmission::Failed(surface) = admission else {
            panic!("app-data path failure must fail startup admission");
        };

        assert_eq!(path_resolution_effects.load(Ordering::SeqCst), 1);
        assert_eq!(
            store_open_effects.load(Ordering::SeqCst),
            0,
            "path failure must not open or initialize the fixed store"
        );
        assert_eq!(surface.window_label, STARTUP_FAILURE_WINDOW_LABEL);

        let authority = surface.authority.clone();
        let usecase::application_startup::ApplicationStartupOutcome::Failed(failure) =
            authority.outcome()
        else {
            panic!("path failure must install a failed startup authority");
        };
        assert_eq!(failure.kind, K::StorageUnavailable);
        assert_eq!(
            failure.safe_description,
            K::StorageUnavailable.safe_description()
        );
        assert!(failure.retry_on_next_launch);
        assert!(uuid::Uuid::parse_str(&failure.correlation_id).is_ok());

        let listener_effects = AtomicUsize::new(0);
        let local_api_bind_effects = AtomicUsize::new(0);
        let websocket_listen_effects = AtomicUsize::new(0);
        assert_eq!(
            normal_startup_effect(authority.as_ref(), || {
                listener_effects.fetch_add(1, Ordering::SeqCst);
                local_api_bind_effects.fetch_add(1, Ordering::SeqCst);
                websocket_listen_effects.fetch_add(1, Ordering::SeqCst);
            }),
            None
        );
        assert_eq!(listener_effects.load(Ordering::SeqCst), 0);
        assert_eq!(local_api_bind_effects.load(Ordering::SeqCst), 0);
        assert_eq!(websocket_listen_effects.load(Ordering::SeqCst), 0);

        let production_resolver_call = ["app.path()", ".app_data_dir()"].concat();
        assert_eq!(
            include_str!("lib.rs")
                .matches(&production_resolver_call)
                .count(),
            1,
            "production startup must resolve app-data once at the classified boundary and reuse it"
        );

        let normal_command_effects = Arc::new(AtomicUsize::new(0));
        let handler = startup_failure_composition_handler();
        let app = tauri::test::mock_builder()
            .plugin(tauri_plugin_fs::init())
            .manage(authority)
            .manage(normal_command_effects.clone())
            .invoke_handler(move |invoke| {
                match adaptor::controller::command::gate_invoke_before_domain_routing(invoke) {
                    Ok(invoke) => handler(invoke),
                    Err(handled) => handled,
                }
            })
            .build(application_context())
            .expect("build path-failure production-composition test app");
        let normal =
            tauri::WebviewWindowBuilder::new(&app, NORMAL_WINDOW_LABEL, Default::default())
                .build()
                .expect("create normal-capability control window");
        let failure_window =
            tauri::WebviewWindowBuilder::new(&app, surface.window_label, Default::default())
                .build()
                .expect("create startup-failure surface");

        let outcome = tauri::test::get_ipc_response(
            &failure_window,
            invoke_request("get_application_startup_outcome", serde_json::json!({})),
        )
        .expect("startup outcome is the first safe command")
        .deserialize::<serde_json::Value>()
        .expect("decode startup outcome");
        assert_eq!(
            outcome,
            serde_json::json!({
                "type": "failed",
                "kind": "storage_unavailable",
                "safeDescription": K::StorageUnavailable.safe_description(),
                "correlationId": failure.correlation_id,
                "retryOnNextLaunch": true,
                "actions": ["quit"]
            })
        );

        let normal_command_error = tauri::test::get_ipc_response(
            &failure_window,
            invoke_request("record_path_failure_normal_effect", serde_json::json!({})),
        )
        .expect_err("failed startup must reject a normal custom command before its handler");
        assert_eq!(
            normal_command_error,
            serde_json::json!({ "type": "application_unavailable" })
        );
        assert_eq!(normal_command_effects.load(Ordering::SeqCst), 0);

        let fixture = tempfile::tempdir().expect("plugin capability fixture");
        let plugin_body =
            serde_json::json!({ "path": fixture.path().to_string_lossy().into_owned() });
        let normal_plugin_result = tauri::test::get_ipc_response(
            &normal,
            invoke_request("plugin:fs|exists", plugin_body.clone()),
        )
        .expect("normal workbench fs command must remain available")
        .deserialize::<bool>()
        .expect("decode fs exists result");
        assert!(normal_plugin_result);
        let failure_plugin_error = tauri::test::get_ipc_response(
            &failure_window,
            invoke_request("plugin:fs|exists", plugin_body),
        )
        .expect_err("startup failure window must not reach the fs plugin handler");
        assert!(
            failure_plugin_error.to_string().contains("not allowed"),
            "failure plugin IPC must be rejected by the production ACL: {failure_plugin_error}"
        );

        for attempt in 0..3 {
            let quit = tauri::test::get_ipc_response(
                &failure_window,
                invoke_request("quit_after_startup_failure", serde_json::json!({})),
            )
            .expect("process-local Quit is the second safe command")
            .deserialize::<serde_json::Value>()
            .expect("decode startup failure Quit outcome");
            assert_eq!(
                quit,
                serde_json::json!({
                    "type": "accepted",
                    "correlationId": failure.correlation_id
                })
            );
            assert_eq!(
                exit.calls.load(Ordering::SeqCst),
                1,
                "attempt {attempt} must join the exit dispatched by the first Quit"
            );
        }
    }

    #[test]
    fn b071_store_open_failures_map_to_the_closed_safe_startup_vocabulary() {
        for (error, expected) in [
            (E::WriterLockHeld, K::StoreInUse),
            (E::StorageUnavailable, K::StorageUnavailable),
            (E::UnsupportedRuntime, K::UnsupportedRuntime),
            (E::UnsupportedStoreVersion, K::UnsupportedStoreVersion),
            (E::InitializationStateInvalid, K::InitializationStateInvalid),
            (E::StoreValidationFailed, K::StoreValidationFailed),
            (E::SchemaEvolutionFailed, K::SchemaEvolutionFailed),
        ] {
            assert_eq!(classify_startup_failure(error), expected);
        }
    }

    #[test]
    fn b071_pre_admission_window_grants_no_plugin_ipc_capability() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(config["app"]["windows"][0]["label"], NORMAL_WINDOW_LABEL);
        assert_eq!(config["app"]["windows"][0]["create"], false);

        fn capability_files(root: &std::path::Path, output: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(root).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    capability_files(&path, output);
                } else if path.extension().and_then(|value| value.to_str()) == Some("json") {
                    output.push(path);
                }
            }
        }

        let mut automatically_loaded = Vec::new();
        capability_files(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("capabilities"),
            &mut automatically_loaded,
        );
        assert!(!automatically_loaded.is_empty());
        let mut startup_failure_capability_seen = false;
        let mut normal_capability = None;
        for path in automatically_loaded {
            let capability: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            let windows = capability["windows"]
                .as_array()
                .expect("capability windows");
            let permissions = capability["permissions"]
                .as_array()
                .expect("capability permissions");
            if !permissions.is_empty() {
                assert_eq!(
                    capability["windows"],
                    serde_json::json!([NORMAL_WINDOW_LABEL]),
                    "a plugin-capable capability can target only the post-admission window: {}",
                    path.display()
                );
            }
            if windows
                .iter()
                .any(|window| window == STARTUP_FAILURE_WINDOW_LABEL)
            {
                startup_failure_capability_seen = true;
                assert_eq!(
                    capability["permissions"],
                    serde_json::json!([]),
                    "startup-failure window received plugin IPC permissions from {}",
                    path.display()
                );
            }
            if windows.iter().any(|window| window == NORMAL_WINDOW_LABEL) {
                normal_capability = Some(capability);
            }
        }
        assert!(startup_failure_capability_seen);

        let normal = normal_capability.expect("normal workbench capability");
        assert_eq!(normal["windows"], serde_json::json!(["main"]));
        let permissions = normal["permissions"]
            .as_array()
            .expect("normal workbench capability permissions");
        for plugin in [
            "fs:default",
            "updater:default",
            "process:allow-restart",
            "autostart:allow-enable",
        ] {
            assert!(
                permissions.iter().any(|permission| permission == plugin),
                "Ready-only capability lost {plugin}"
            );
        }
    }

    #[test]
    fn b071_failed_startup_never_binds_or_listens_on_the_local_api() {
        let failed = usecase::application_startup::ApplicationStartupAuthority::failed_kind(
            K::StoreValidationFailed,
        );
        let plugin_capability_effects = AtomicUsize::new(0);
        let listener_effects = AtomicUsize::new(0);

        assert_eq!(
            normal_startup_effect(&failed, || {
                plugin_capability_effects.fetch_add(1, Ordering::SeqCst);
                listener_effects.fetch_add(1, Ordering::SeqCst);
                "normal-surface"
            }),
            None
        );
        assert_eq!(plugin_capability_effects.load(Ordering::SeqCst), 0);
        assert_eq!(listener_effects.load(Ordering::SeqCst), 0);

        let ready = usecase::application_startup::ApplicationStartupAuthority::ready();
        assert_eq!(
            normal_startup_effect(&ready, || {
                plugin_capability_effects.fetch_add(1, Ordering::SeqCst);
                listener_effects.fetch_add(1, Ordering::SeqCst);
                "normal-surface"
            }),
            Some("normal-surface")
        );
        assert_eq!(plugin_capability_effects.load(Ordering::SeqCst), 1);
        assert_eq!(listener_effects.load(Ordering::SeqCst), 1);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartupRecoveryWorkerExit {
    Quiescent,
}

/// Drives bounded recovery passes after the fixed store is Ready. Every pass
/// starts with a fresh pending-index snapshot; two empty passes define
/// quiescence, so work inserted while the first empty page was being observed
/// is not lost. Transient failures retain the worker with capped backoff.
async fn run_startup_recovery<Recover, Future, Error>(
    worker_name: &'static str,
    mut recover_pass: Recover,
    initial_retry_delay: std::time::Duration,
    maximum_retry_delay: std::time::Duration,
) -> StartupRecoveryWorkerExit
where
    Recover: FnMut() -> Future,
    Future: std::future::Future<Output = Result<usize, Error>>,
    Error: std::fmt::Debug,
{
    let mut retry_delay = initial_retry_delay;
    let mut consecutive_empty_passes = 0u8;
    loop {
        match recover_pass().await {
            Ok(0) => {
                consecutive_empty_passes = consecutive_empty_passes.saturating_add(1);
                if consecutive_empty_passes >= 2 {
                    return StartupRecoveryWorkerExit::Quiescent;
                }
                tokio::task::yield_now().await;
            }
            Ok(_) => {
                consecutive_empty_passes = 0;
                retry_delay = initial_retry_delay;
                tokio::time::sleep(initial_retry_delay).await;
            }
            Err(error) => {
                consecutive_empty_passes = 0;
                log::warn!("{worker_name} startup recovery will retry: {error:?}");
                tokio::time::sleep(retry_delay).await;
                retry_delay = retry_delay.saturating_mul(2).min(maximum_retry_delay);
            }
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let startup_started = Instant::now();
    other::telemetry::set_startup_origin(startup_started);

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    let provider_initial_search_path =
        infrastructure::process::search_path::capture_login_shell_path(
            infrastructure::process::search_path::LOGIN_SHELL_PATH_TIMEOUT,
        );
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    if let Ok(search_path) = &provider_initial_search_path {
        std::env::set_var("PATH", search_path);
    }

    // OTLP exporter and async commands share the Tokio runtime installed for Tauri.
    let _runtime = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");
    let _runtime_guard = _runtime.enter();
    tauri::async_runtime::set(_runtime.handle().clone());

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--hidden"]),
        ));
    #[cfg(feature = "performance-wdio")]
    let builder = builder
        .plugin(tauri_plugin_wdio::init())
        .plugin(tauri_plugin_wdio_webdriver::init());
    let builder = builder.setup(move |app| {
            let failure_exit: Arc<
                dyn usecase::application_startup::ProcessLocalExitPort,
            > = Arc::new(
                adaptor::controller::application_lifecycle::TauriProcessLocalExitPort::new(
                    app.handle().clone(),
                ),
            );
            let ReadyStartupStore {
                data_dir,
                app_data,
                local_event_store,
            } = match compose_startup_store_admission(
                || app.path().app_data_dir(),
                |app_data| app_data.open_local_event_store(),
                failure_exit,
            ) {
                StartupStoreAdmission::Ready(store) => store,
                StartupStoreAdmission::Failed(surface) => {
                    log::error!("application startup admission failed");
                    install_failed_startup_surface(app, surface)?;
                    return Ok(());
                }
            };
            // Publish Ready only after every normal state/effect ingress below
            // has been constructed. Until setup completes, a missing authority
            // is itself fail-closed at the top-level command router.
            let startup_authority = Arc::new(
                usecase::application_startup::ApplicationStartupAuthority::ready(),
            );
            app.manage(local_event_store.clone());
            let projected_local_event_repository: Arc<
                dyn domain::local_event::LocalEventTransactionRepository,
            > = local_event_store.clone();
            let terminal_surface_runtime =
                terminal_surface::TerminalSurfaceRuntime::new(app.handle().clone());
            let terminal_surface = terminal_surface_runtime.application();
            app.manage(Arc::new(
                adaptor::controller::wiring::build_review_comment_usecase(),
            ));
            app.manage(infrastructure::file_watcher::FileWatcherManager::default());
            app.manage::<adaptor::gateway::repository::repo_paths::SharedRepoPaths>(Arc::new(
                parking_lot::RwLock::new(Vec::new()),
            ));
            app.manage(Arc::new(
                adaptor::gateway::workspace_state::WorkspaceStateStore::new(data_dir.clone()),
            ));
            // spec issues-1054 Implementation Freedom (L104): 別 Releash binary 由来の
            // RELEASH_DATA_DIR inherit (例: prod 版 Releash の Terminal Panel から起動
            // した shell から dev binary を起動した場合) を「ユーザー明示指定」と誤認しないよう、
            // 起動初期に env を自プロセス alias data_dir で正す。
            crate::infrastructure::platform::path_aliases::
                ensure_release_data_dir_env_for_resolved_path(&data_dir);
            infrastructure::platform::cli_install::ensure_cli_symlink_installed();
            let config_path = data_dir.join("releash.toml");
            let config = load_or_create_config(&config_path)
                .map_err(|e| format!("設定ファイルの読み込みに失敗: {e}"))?;
            if let Some(telemetry_guard) = infrastructure::telemetry::init_telemetry(&config) {
                app.manage(telemetry_guard);
            }

            let app_config = Arc::new(AppConfig::new(config, config_path));
            let config_repository: Arc<dyn ConfigRepository> = app_config.clone();
            let config_secret_repository: Arc<dyn ConfigSecretRepository> = app_config.clone();
            let notion_config_repository: Arc<dyn NotionConfigRepository> = app_config.clone();
            let notion_api_gateway: Arc<dyn domain::notion::NotionApiGateway> =
                Arc::new(adaptor::gateway::notion::NotionApiGatewayImpl::new());
            app.manage(config_repository.clone());
            app.manage(config_secret_repository.clone());
            let provider_executable_config: Arc<
                dyn domain::agent_session::ProviderExecutableConfigRepository,
            > = if let Some(fixture) = performance_provider_fixture_executable() {
                let (claude, codex) = select_provider_agent_executables(
                    "claude".to_string(),
                    "codex".to_string(),
                    Some(fixture),
                );
                Arc::new(
                    adaptor::gateway::agent_session::InMemoryProviderExecutableConfigRepository::new(
                        Some(claude),
                        Some(codex),
                    )
                    .map_err(|error| {
                        format!("Provider performance fixture設定の初期化に失敗: {error:?}")
                    })?,
                )
            } else {
                app_config.clone()
            };
            let provider_history_home = dirs::home_dir()
                .unwrap_or_else(|| data_dir.join("provider-history-unavailable"));
            let agent_sessions =
                adaptor::controller::agent_session_wiring::compose_agent_sessions(
                    adaptor::controller::agent_session_wiring::AgentSessionCompositionInput {
                        repository: projected_local_event_repository.clone(),
                        installation_id: local_event_store.installation_id().to_string(),
                        store: local_event_store.clone(),
                        data_dir: data_dir.clone(),
                        provider_executable_config,
                        provider_executable_probe: Arc::new(
                            #[cfg(any(target_os = "macos", target_os = "linux"))]
                            adaptor::gateway::agent_session::LocalProviderExecutableProbeGateway::with_initial_search_path(
                                provider_initial_search_path.clone(),
                            ),
                            #[cfg(not(any(target_os = "macos", target_os = "linux")))]
                            adaptor::gateway::agent_session::LocalProviderExecutableProbeGateway::new(),
                        ),
                        claude_config_dir: std::env::var_os("CLAUDE_CONFIG_DIR")
                            .map(std::path::PathBuf::from)
                            .unwrap_or_else(|| provider_history_home.join(".claude")),
                        codex_home: std::env::var_os("CODEX_HOME")
                            .map(std::path::PathBuf::from)
                            .unwrap_or_else(|| provider_history_home.join(".codex")),
                        cli_binary: infrastructure::platform::path_aliases::alias_name_for_profile(
                            infrastructure::platform::path_aliases::BuildProfile::current(),
                        )
                        .to_string(),
                        terminal: terminal_surface.clone(),
                        change_notifier: Arc::new(
                            adaptor::presenter::agent_session_changed::TauriAgentSessionChangeNotifier::new(
                                app.handle().clone(),
                            ),
                        ),
                    },
                )
                .map_err(|error| format!("Provider availability初期化失敗: {error:?}"))?;
            let agent_session_launch = agent_sessions.launch.clone();
            let agent_session_initial_instruction = agent_sessions.initial_instruction.clone();
            let agent_session_interrupt = agent_sessions.interrupt.clone();
            let agent_session_exit = agent_sessions.exit.clone();
            let provider_availability = agent_sessions.availability_reader.clone();
            let provider_lifecycle_ingress = agent_sessions.lifecycle_ingress.clone();
            let provider_workflow_stops = agent_sessions.workflow_stops.clone();
            terminal_surface_runtime
                .bind_agent_session_activity(agent_sessions.activity.clone());
            let provider_agent_terminal_events = terminal_surface.subscribe_events();
            let shutdown_provider_exit_observer: Arc<dyn Fn() + Send + Sync> = Arc::new({
                let cancellation = provider_agent_terminal_events.cancellation.clone();
                move || cancellation.cancel()
            });
            tauri::async_runtime::spawn(
                adaptor::controller::agent_session_exit_observer::run_agent_session_exit_observer(
                    provider_agent_terminal_events,
                    agent_session_exit.clone(),
                ),
            );
            app.manage(agent_sessions.provider_lifecycle);
            app.manage(agent_sessions.sessions);
            app.manage(agent_sessions.history_read);
            app.manage(agent_sessions.hook_health);
            app.manage(agent_sessions.hook_health_read);
            app.manage(agent_sessions.lifecycle_ingress);
            app.manage(agent_sessions.lifecycle);
            app.manage(agent_sessions.read);
            app.manage(agent_session_exit);
            app.manage(agent_sessions.provider_availability);
            app.manage(agent_session_launch.clone());
            app.manage(agent_session_initial_instruction.clone());
            app.manage(agent_session_interrupt.clone());

            // Initialize shared repo_paths from config
            let shared_repo_paths = app
                .state::<adaptor::gateway::repository::repo_paths::SharedRepoPaths>()
                .inner()
                .clone();
            {
                if let Ok(cfg) = config_repository.load() {
                    let paths: Vec<String> = cfg
                        .app
                        .last_repo_paths
                        .iter()
                        .filter(|p| !p.is_empty())
                        .cloned()
                        .collect();
                    *shared_repo_paths.write() = paths;
                }
            }
            // repository ドメインの DI 配線（起動時に AppState を組み立てて manage）。
            // git ベースの usecase / query service はステートレス、repo_paths は
            // SharedRepoPaths + AppConfig を共有する。repository usecase は 1 度だけ
            // 組み立て、AppState・単体 State（workflow コマンド注入用）・watcher・
            // workflow リゾルバへ Arc 共有する（各エントリは注入で受け取る）。
            let repository_usecase = Arc::new(
                adaptor::controller::wiring::build_repository_usecase_with_worktree_terminals(
                    terminal_surface.clone(),
                ),
            );
            app.manage(repository_usecase.clone());
            {
                use adaptor::controller::state::AppState;
                use adaptor::gateway::repository::repo_paths::RepoPathsGateway;
                use usecase::repo_paths_usecase::RepoPathsUsecase;

                let repo_paths_gateway =
                    RepoPathsGateway::new(shared_repo_paths.clone(), config_repository.clone());
                // 変更通知（repo-paths-changed）の送信 infra を NotifyGateway として注入。
                let repo_paths_notifier = Arc::new(
                    adaptor::gateway::repository::notify::RepoPathsNotifyGateway::new(
                        app.handle().clone(),
                    ),
                );
                let repo_paths_usecase = Arc::new(RepoPathsUsecase::new(
                    Arc::new(repo_paths_gateway),
                    repo_paths_notifier,
                ));

                // code ドメインの DI 配線（gateway 実装はステートレス）。
                let code_usecase = Arc::new(
                    adaptor::controller::wiring::build_code_usecase_with_app(app.handle().clone()),
                );
                let git_host_usecase =
                    Arc::new(adaptor::controller::wiring::build_git_host_usecase());
                let repository_scanner = Arc::new(
                    adaptor::gateway::repository::scanner::DefaultRepositoryScanner::new(
                        repository_usecase.clone(),
                        code_usecase.clone(),
                    ),
                );
                let repository_state_repository = Arc::new(
                    adaptor::gateway::repository::state::RepositoryStateRepositoryGateway::new(
                        repository_usecase.clone(),
                    ),
                );
                let repository_state =
                    Arc::new(usecase::repository_state::RepositoryStateService::new(
                        repository_state_repository,
                        repository_scanner,
                        Arc::new(
                            adaptor::gateway::repository::state::TauriRepositoryStateNotifier::new(
                                app.handle().clone(),
                            ),
                        ),
                        Arc::new(
                            adaptor::gateway::repository::state::NotifyRepositoryStateWatcher::new(
                                repository_usecase.clone(),
                            ),
                        ),
                        Arc::new(
                            adaptor::gateway::repository::state::TokioRepositoryStateWorkerRuntime,
                        ),
                        Arc::new(adaptor::gateway::repository::state::FsWorktreePathNormalizer),
                    ));
                app.manage(repository_state.clone());
                let review_usecase = Arc::new(usecase::review_usecase::ReviewUsecase::new(
                    repository_state.clone(),
                    code_usecase.clone(),
                ));
                let (workflow_usecase, workspace_query_service) =
                    adaptor::controller::wiring::build_workflow_services_with_repository_worktrees(
                        data_dir.clone(),
                        repository_usecase.clone(),
                        config_repository.clone(),
                        config_secret_repository.clone(),
                        app.handle().clone(),
                        local_event_store.clone(),
                    );
                let workflow_usecase = Arc::new(workflow_usecase);
                app.manage(workspace_query_service);
                let notion_usecase = Arc::new(usecase::notion::usecase::NotionUsecase::new(
                    notion_config_repository.clone(),
                    notion_api_gateway.clone(),
                ));

                app.manage(AppState {
                    repository_usecase: repository_usecase.clone(),
                    repository_state,
                    repo_paths_usecase,
                    code_usecase,
                    review_usecase,
                    notion_usecase,
                    workflow_usecase,
                    terminal_surface: terminal_surface.clone(),
                    git_host_usecase,
                });
            }

			if app.get_webview_window(NORMAL_WINDOW_LABEL).is_some() {
				other::telemetry::record_startup_from_origin(
					other::telemetry::Startup::FirstWindowReady,
				);
			} else {
				log::warn!("Main window not found; first-window telemetry is unavailable");
			}

            let caller_journal = Arc::new(
                usecase::application_lifecycle::operation::CallerAttemptJournal::new(
                    projected_local_event_repository.clone(),
                    local_event_store.clone(),
                    local_event_store.installation_id().to_string(),
                ),
            );
            app.manage(caller_journal.clone());
            let workflow_runtime_usecase = Arc::new(
                adaptor::controller::wiring::build_workflow_runtime_usecase(
                    app.handle().clone(),
                    adaptor::gateway::workflow::TauriWorkflowRuntimeCommandGatewayDeps {
                        repository_usecase: repository_usecase.clone(),
                        app_config: config_repository.clone(),
                        data_dir: Some(data_dir.clone()),
                        local_event_repository: projected_local_event_repository.clone(),
                        local_event_installation_id: local_event_store
                            .installation_id()
                            .to_string(),
                        agent_session_launch: agent_session_launch.clone(),
                        agent_session_initial_instruction: agent_session_initial_instruction
                            .clone(),
                        agent_session_interrupt: agent_session_interrupt.clone(),
                        provider_availability: provider_availability.clone(),
                    },
                )
                .map_err(|error| format!("workflow recovery admission failed: {error}"))?,
            );
            let workspace_node_resolver: Arc<
                dyn usecase::workflow::WorkspaceNodeActionResolver,
            > = app
                .state::<adaptor::controller::state::AppState>()
                .workflow_usecase
                .clone();
            app.manage(Arc::new(
                adaptor::controller::wiring::build_workspace_node_command_usecase(
                    workspace_node_resolver,
                    workflow_runtime_usecase.clone(),
                ),
            ));
            provider_workflow_stops.bind(workflow_runtime_usecase.clone());
            let pending_workflow_recovery = workflow_runtime_usecase.clone();
            tauri::async_runtime::spawn(async move {
                run_startup_recovery(
                    "pending workflow restart reconciliation",
                    || {
                        let workflow = pending_workflow_recovery.clone();
                        async move {
                            workflow
                                .recover_startup()
                                .await
                                .map_err(|error| error.to_string())?;
                            Ok::<usize, String>(0)
                        }
                    },
                    std::time::Duration::from_millis(50),
                    std::time::Duration::from_secs(1),
                )
                .await;
            });
            app.manage(workflow_runtime_usecase.clone());

            let workflow_query_usecase = app
                .state::<adaptor::controller::state::AppState>()
                .workflow_usecase
                .clone();
            let local_api_shutdown_target: LocalApiShutdownTarget =
                Arc::new(parking_lot::RwLock::new(None));
            let shutdown_local_api: Arc<dyn Fn() + Send + Sync> = Arc::new({
                let target = local_api_shutdown_target.clone();
                move || {
                    if let Some(shutdown) = target.read().clone() {
                        shutdown();
                    }
                }
            });
            let shutdown_coordinator =
                adaptor::controller::application_lifecycle::build_shutdown_coordinator(
                    local_event_store.clone(),
                    projected_local_event_repository.clone(),
                    adaptor::controller::application_lifecycle::RuntimeShutdownDependencies::new(
                        workflow_runtime_usecase.clone(),
                        terminal_surface.clone(),
                        shutdown_provider_exit_observer,
                        shutdown_local_api,
                    ),
                );
            let process_actions = Arc::new(
                adaptor::controller::application_lifecycle::ApplicationProcessActionDispatcher::default(),
            );
            app.manage(process_actions.clone());

            // CLI / Agent / 外部編集由来の review comment 変更を UI へ通知する。
            infrastructure::comment::watcher::spawn_review_comments_watcher(
                app.handle().clone(),
                data_dir.clone(),
            );

            infrastructure::platform::menu::setup_menu(app)?;
            app.manage(shutdown_coordinator.clone());
            let quit_app = app.handle().clone();
            let quit_process_actions = process_actions.clone();
            let quit_shutdown_coordinator = shutdown_coordinator.clone();
            let quit_ingress = Arc::new(
                adaptor::controller::application_lifecycle::ApplicationQuitIngress::new(
                    move |intent| {
                        adaptor::controller::application_lifecycle::request_application_quit(
                            quit_app.clone(),
                            quit_shutdown_coordinator.clone(),
                            quit_process_actions.clone(),
                            intent,
                        );
                    },
                ),
            );
            app.manage(quit_ingress.clone());
            infrastructure::platform::tray::setup_tray(app, move |_app| {
                quit_ingress.request(
                    usecase::shutdown_coordinator::ApplicationQuitIntent::Exit { code: 0 },
                );
            })?;
            if !app.manage(startup_authority.clone()) {
                return Err("application startup authority was already installed".into());
            }
            normal_startup_effect(startup_authority.as_ref(), || {
                adaptor::controller::wiring::spawn_startup_app_data_gc(
                    app_data.clone(),
                    shared_repo_paths.clone(),
                    projected_local_event_repository.clone(),
                );
                let local_api_binding =
                    infrastructure::local_api::LocalApiServerBinding::bind(data_dir.clone())
                        .map_err(|error| format!("local API の起動に失敗しました: {error}"))?;
                let local_api_router = adaptor::controller::api::build_router(
                    Arc::new(workflow_query_usecase.read_usecase()),
                    workflow_runtime_usecase.clone(),
                    local_api_binding.bearer_token(),
                    local_api_binding.terminal_bearer_token(),
                    Some(adaptor::controller::api::TerminalApiDeps::new(
                        terminal_surface.clone(),
                    )),
                    Some(provider_lifecycle_ingress.clone()),
                );
                app.manage(adaptor::controller::state::TerminalStreamEndpoint {
                    port: local_api_binding.port(),
                    token: local_api_binding.terminal_bearer_token(),
                });
                let local_api =
                    local_api_binding.start(local_api_router, &tokio::runtime::Handle::current());
                *local_api_shutdown_target.write() = Some(Arc::new({
                    let local_api = local_api.clone();
                    move || local_api.shutdown()
                }));
                app.manage(local_api);

                create_configured_window(app, NORMAL_WINDOW_LABEL)?;
                if let Some(window) = app.get_webview_window(NORMAL_WINDOW_LABEL) {
                    infrastructure::platform::native_drop::install(&window);
                }
                infrastructure::platform::window_lifecycle::apply_startup_visibility(
                    app.handle(),
                    config_repository.as_ref(),
                );
                other::telemetry::record_startup_from_origin(
                    other::telemetry::Startup::AppStartup,
                );
                Ok::<(), Box<dyn std::error::Error>>(())
            })
            .ok_or_else(|| std::io::Error::other("normal startup admission was not Ready"))??;
            Ok(())
        });
    let builder =
        adaptor::controller::command::code::review_blob::register_review_blob_protocol(builder);

    let builder = adaptor::controller::command::register_all(builder);
    builder
        .build(application_context())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            infrastructure::platform::window_lifecycle::handle_run_event(app_handle, event);
        });
}
