mod adaptor;
pub mod cli;
mod domain;
mod infrastructure;
mod other;
// Test-only helpers are intentionally kept as a root module.
#[cfg(test)]
mod test_support;
mod usecase;

use std::sync::Arc;
use std::time::Instant;

use adaptor::gateway::app_config::{load_or_create_config, AppConfig};
use domain::app_config::{
    AgentConfigRepository, ConfigRepository, ConfigSecretRepository, NotionConfigRepository,
};
use infrastructure::platform::window_lifecycle::{
    NORMAL_WINDOW_LABEL, STARTUP_FAILURE_WINDOW_LABEL,
};
use tauri::Manager;

type LocalApiShutdownTarget = Arc<parking_lot::RwLock<Option<Arc<dyn Fn() + Send + Sync>>>>;

#[allow(clippy::too_many_arguments)]
pub(crate) fn compose_agent_session_runtime(
    session_store: Arc<usecase::agent_session::session::SessionStore>,
    registry: Arc<usecase::agent_session::backend_registry::AgentBackendRegistry>,
    status_center: Arc<usecase::agent_session::status::AgentStatusCenter>,
    status_notifier: Arc<dyn usecase::agent_session::status::AgentStatusNotifier>,
    event_notifier: Arc<dyn usecase::agent_session::runtime::ports::AgentSessionEventNotifier>,
    spawner: Arc<dyn usecase::agent_session::runtime::ports::AgentTaskSpawner>,
    branch_diff_context: Option<Arc<dyn usecase::agent_session::context::BranchDiffContextPort>>,
    instruction_source: Arc<dyn usecase::agent_session::context::InstructionSourcePort>,
    data_dir: std::path::PathBuf,
    workspace_query: Arc<dyn usecase::workspace_tree::WorkspaceQueryService>,
) -> Arc<usecase::agent_session::runtime::AgentSessionRuntimeUsecase> {
    Arc::new(
        usecase::agent_session::runtime::AgentSessionRuntimeUsecase::new(
            session_store,
            registry,
            status_center,
            status_notifier,
            event_notifier,
            spawner,
            branch_diff_context,
            instruction_source,
            data_dir,
            workspace_query,
        ),
    )
}

fn application_context<R: tauri::Runtime>() -> tauri::Context<R> {
    tauri::generate_context!()
}

fn create_configured_window(
    app: &tauri::App,
    label: &str,
) -> Result<(), Box<dyn std::error::Error>> {
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

#[cfg(test)]
mod startup_composition_tests {
    use super::*;
    use adaptor::gateway::local_event_store::store::LocalEventStoreOpenError as E;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use usecase::application_startup::StartupFailureKind as K;

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

/// App-lifetime owner for recovery work that can become runnable after the
/// initial two-pass scan has quiesced. Store commits and process-local retry
/// requests only publish wakeups; this single supervisor performs fresh
/// bounded scans and owns capped retry while storage is unavailable.
async fn run_wakeable_recovery<Recover, Future, Error, Subscribe>(
    worker_name: &'static str,
    mut recover_pass: Recover,
    mut subscribe: Subscribe,
    wakeup: Arc<tokio::sync::Notify>,
    initial_retry_delay: std::time::Duration,
    maximum_retry_delay: std::time::Duration,
) where
    Recover: FnMut() -> Future,
    Future: std::future::Future<Output = Result<usize, Error>>,
    Error: std::fmt::Debug,
    Subscribe: FnMut() -> domain::local_event::LocalEventSubscription,
{
    use futures_util::{FutureExt as _, StreamExt as _};

    // Subscribe before the first inventory scan. A commit racing the final
    // empty pass is then buffered by the subscription, while a process-local
    // retry racing that same boundary leaves a permit in `Notify`.
    let mut signals = subscribe().into_stream();
    loop {
        run_startup_recovery(
            worker_name,
            &mut recover_pass,
            initial_retry_delay,
            maximum_retry_delay,
        )
        .await;
        let mut signal_stream_closed = tokio::select! {
            _ = wakeup.notified() => false,
            signal = signals.next() => signal.is_none(),
        };
        // One send-inventory scan is enough for every commit already visible
        // at this boundary. Coalesce the buffered global-store burst instead
        // of rescanning every pending page once per unrelated projection or
        // streaming commit.
        while !signal_stream_closed {
            match signals.next().now_or_never() {
                Some(Some(_)) => {}
                Some(None) => signal_stream_closed = true,
                None => break,
            }
        }
        if signal_stream_closed {
            // A live writable store keeps its broadcaster open. Avoid a busy
            // loop if a replacement/read-only source closes, then subscribe
            // before taking the next fresh inventory.
            tokio::time::sleep(maximum_retry_delay).await;
            signals = subscribe().into_stream();
        }
    }
}

#[cfg(test)]
mod recovery_supervisor_tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    #[tokio::test]
    async fn wakeable_recovery_does_not_lose_commit_or_notify_at_quiescence_boundary() {
        let (signals_tx, signals_rx) = tokio::sync::mpsc::unbounded_channel();
        let signals_rx = Arc::new(std::sync::Mutex::new(Some(signals_rx)));
        let subscribed = Arc::new(AtomicBool::new(false));
        let passes = Arc::new(AtomicUsize::new(0));
        let second_pass_entered = Arc::new(tokio::sync::Notify::new());
        let release_second_pass = Arc::new(tokio::sync::Notify::new());
        let fourth_pass_entered = Arc::new(tokio::sync::Notify::new());
        let release_fourth_pass = Arc::new(tokio::sync::Notify::new());
        let wakeup = Arc::new(tokio::sync::Notify::new());

        let supervisor = tokio::spawn(run_wakeable_recovery(
            "test recovery",
            {
                let subscribed = subscribed.clone();
                let passes = passes.clone();
                let second_pass_entered = second_pass_entered.clone();
                let release_second_pass = release_second_pass.clone();
                let fourth_pass_entered = fourth_pass_entered.clone();
                let release_fourth_pass = release_fourth_pass.clone();
                move || {
                    let subscribed = subscribed.clone();
                    let passes = passes.clone();
                    let second_pass_entered = second_pass_entered.clone();
                    let release_second_pass = release_second_pass.clone();
                    let fourth_pass_entered = fourth_pass_entered.clone();
                    let release_fourth_pass = release_fourth_pass.clone();
                    async move {
                        assert!(
                            subscribed.load(Ordering::SeqCst),
                            "the signal subscription must exist before the first scan"
                        );
                        let pass = passes.fetch_add(1, Ordering::SeqCst) + 1;
                        match pass {
                            2 => {
                                second_pass_entered.notify_one();
                                release_second_pass.notified().await;
                            }
                            4 => {
                                fourth_pass_entered.notify_one();
                                release_fourth_pass.notified().await;
                            }
                            _ => {}
                        }
                        Ok::<usize, ()>(0)
                    }
                }
            },
            {
                let subscribed = subscribed.clone();
                move || {
                    subscribed.store(true, Ordering::SeqCst);
                    let receiver = signals_rx
                        .lock()
                        .unwrap()
                        .take()
                        .expect("the live test subscription is created once");
                    let stream = futures_util::stream::unfold(receiver, |mut receiver| async {
                        receiver.recv().await.map(|signal| (signal, receiver))
                    });
                    domain::local_event::LocalEventSubscription::new(Box::pin(stream))
                }
            },
            wakeup.clone(),
            std::time::Duration::from_millis(1),
            std::time::Duration::from_millis(10),
        ));

        second_pass_entered.notified().await;
        for index in 0..128 {
            signals_tx
                .send(domain::local_event::LocalEventSignal::Committed {
                    commit_id: domain::local_event::CommitIdentity::parse(&format!(
                        "test-commit-{index}"
                    ))
                    .unwrap(),
                    max_global_sequence: domain::local_event::GlobalSequence::new(index + 1)
                        .unwrap(),
                })
                .unwrap();
        }
        release_second_pass.notify_one();

        fourth_pass_entered.notified().await;
        wakeup.notify_one();
        release_fourth_pass.notify_one();

        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while passes.load(Ordering::SeqCst) < 6 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("both boundary wakeups must trigger a fresh two-pass scan");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert_eq!(
            passes.load(Ordering::SeqCst),
            6,
            "a buffered global commit burst must coalesce into one fresh scan"
        );

        supervisor.abort();
        let _ = supervisor.await;
    }
}

fn spawn_startup_maintenance(
    app_data: adaptor::controller::app_data_composition::ProductionAppDataComposition,
    _shared_repo_paths: adaptor::gateway::repository::repo_paths::SharedRepoPaths,
) {
    tauri::async_runtime::spawn(async move {
        match tauri::async_runtime::spawn_blocking(move || app_data.cleanup_orphan_processes())
            .await
        {
            Ok(report) if report.scanned > 0 || report.failures > 0 => {
                log::info!(
                    "agent orphan cleanup scanned={} processed={} skipped={} failures={}",
                    report.scanned,
                    report.processed,
                    report.skipped,
                    report.failures
                );
            }
            Ok(_) => {}
            Err(error) => log::error!("agent orphan cleanup task failed: {error}"),
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let startup_started = Instant::now();
    other::telemetry::set_startup_origin(startup_started);

    // OTLP exporter and async commands share the Tokio runtime installed for Tauri.
    let _runtime = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");
    let _runtime_guard = _runtime.enter();
    tauri::async_runtime::set(_runtime.handle().clone());

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    let _ = fix_path_env::fix();

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--hidden"]),
        ))
        .setup(|app| {
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
            let pty_gateway = Arc::new(
                adaptor::gateway::pty_session::backend_impl::PtySessionRuntimeGateway::default(),
            );
            let pty_read_gateway: Arc<
                dyn usecase::pty_session::ports::PtySessionReadGateway + Send + Sync,
            > = pty_gateway.clone();
            let pty_session_read_usecase = Arc::new(
                usecase::pty_session::read_usecase::PtySessionReadUsecase::new(pty_read_gateway),
            );
            pty_gateway.start_idle_sweeper(app.handle().clone());
            app.manage(pty_gateway);
            let session_event_repository: Arc<
                dyn domain::local_event::LocalEventTransactionRepository,
            > = projected_local_event_repository.clone();
            let session_store = Arc::new(
                usecase::agent_session::session::SessionStore::new_canonical(
                    session_event_repository,
                    local_event_store.installation_id().to_string(),
                    Arc::new(
                        adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
                    ),
                ),
            );
            let workspace_session_creation_usecase = Arc::new(
                usecase::agent_session::workspace_session_creation::WorkspaceSessionCreationUsecase::new(
                    session_store.clone(),
                ),
            );
            app.manage(Arc::new(
                adaptor::controller::wiring::build_review_comment_usecase(),
            ));
            app.manage(session_store.clone());
            app.manage(workspace_session_creation_usecase);
            app.manage(Arc::new(
                adaptor::controller::wiring::build_agent_prompt_suggestion_usecase(
                    session_store.clone(),
                ),
            ));
            app.manage(infrastructure::file_watcher::FileWatcherManager::default());
            app.manage(Arc::new(
                usecase::agent_session::session::OpenTabRegistry::default(),
            ));
            app.manage::<adaptor::gateway::repository::repo_paths::SharedRepoPaths>(Arc::new(
                parking_lot::RwLock::new(Vec::new()),
            ));
            let session_feedback_usecase = Arc::new(
                usecase::agent_session::feedback::SessionFeedbackUsecase::new(
                    projected_local_event_repository.clone(),
                    local_event_store.installation_id().to_string(),
                ),
            );
            let abandoned_feedback_recovery = session_feedback_usecase.clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    match abandoned_feedback_recovery
                        .recover_abandoned_reservations()
                        .await
                    {
                        Ok(recovered) => {
                            if recovered > 0 {
                                log::warn!(
                                    "recovered {recovered} abandoned session feedback reservations"
                                );
                            }
                            break;
                        }
                        Err(error) => {
                            log::warn!(
                                "abandoned session feedback recovery will retry: {error:?}"
                            );
                            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                        }
                    }
                }
            });
            let agent_session_notice_usecase = Arc::new(
                adaptor::controller::agent_session_notice_wiring::build_agent_session_notice_usecase(),
            );
            app.manage(session_feedback_usecase.clone());
            app.manage(agent_session_notice_usecase);
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
            let agent_config_repository: Arc<dyn AgentConfigRepository> = app_config.clone();
            let config_secret_repository: Arc<dyn ConfigSecretRepository> = app_config.clone();
            let notion_config_repository: Arc<dyn NotionConfigRepository> = app_config.clone();
            let notion_api_gateway: Arc<dyn domain::notion::NotionApiGateway> =
                Arc::new(adaptor::gateway::notion::NotionApiGatewayImpl::new());
            app.manage(config_repository.clone());
            app.manage(agent_config_repository.clone());
            app.manage(config_secret_repository.clone());
            app.manage(Arc::new(
                adaptor::controller::wiring::build_agent_backend_registry(
                    agent_config_repository.clone(),
                ),
            ));

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
            let repository_usecase =
                Arc::new(adaptor::controller::wiring::build_repository_usecase());
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
                let branch_diff_context: Arc<
                    dyn usecase::agent_session::context::BranchDiffContextPort,
                > = Arc::new(
                    adaptor::gateway::code::branch_diff_context::CodeBranchDiffContextGateway::new(
                        code_usecase.clone(),
                    ),
                );
                app.manage(branch_diff_context);
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
                    pty_session_read_usecase,
                    git_host_usecase,
                });
            }

            let focus_tracker = Arc::new(parking_lot::Mutex::new(
                infrastructure::platform::focus_tracker::FocusTracker::new(),
            ));
            app.manage(focus_tracker.clone());
            infrastructure::platform::focus_tracker::install(app, focus_tracker.clone());

            {
                let notice_usecase = app
                    .state::<Arc<usecase::agent_session::notice::AgentSessionNoticeUsecase>>()
                    .inner()
                    .clone();
                adaptor::controller::agent_session_notice_wiring::register_agent_session_notice_publisher(
                    notice_usecase,
                    app.handle().clone(),
                );
            }

            {
                let session_store_state = app
                    .state::<Arc<usecase::agent_session::session::SessionStore>>()
                    .inner()
                    .clone();
                let notification_usecase = Arc::new(
                    usecase::notification::usecase::AgentSessionNotificationUsecase::new(
                        Arc::new(
                            adaptor::gateway::notification::NotificationSettingsConfigGateway::new(
                                config_repository.clone(),
                            ),
                        ),
                        Arc::new(
                            adaptor::gateway::notification::FocusNotificationInactivityGateway::new(
                                focus_tracker.clone(),
                            ),
                        ),
                        Arc::new(adaptor::gateway::notification::ReqwestWebhookSenderGateway),
                    ),
                );
                adaptor::controller::notification_wiring::register_agent_notification_listener(
                    session_store_state,
                    notification_usecase,
                );
            }

            // AgentStatusCenter を構築・登録
            let agent_status_center =
                Arc::new(usecase::agent_session::status::AgentStatusCenter::new());
            let agent_status_notifier: Arc<
                dyn usecase::agent_session::status::AgentStatusNotifier,
            > = Arc::new(
                adaptor::presenter::agent_status::TauriAgentStatusNotifier::new(
                    app.handle().clone(),
                ),
            );
            // SessionStore の状態変更通知を購読して、保持している SessionStatus を
            // 最新化＋再集約する。Closed への遷移は aggregate でフィルタされ、
            // Closed → Idle の復帰では再び集約対象に戻る。
            {
                let session_store_state = app
                    .state::<Arc<usecase::agent_session::session::SessionStore>>()
                    .inner()
                    .clone();
                adaptor::controller::agent_status_wiring::register_agent_status_listener(
                    session_store_state,
                    agent_status_center.clone(),
                    agent_status_notifier.clone(),
                );
            }
            app.manage(agent_status_notifier.clone());
            app.manage(agent_status_center.clone());
            {
                let runtime_session_store = app
                    .state::<Arc<usecase::agent_session::session::SessionStore>>()
                    .inner()
                    .clone();
                let runtime_registry = app
                    .state::<Arc<usecase::agent_session::backend_registry::AgentBackendRegistry>>()
                    .inner()
                    .clone();
                let runtime_notifier: Arc<
                    dyn usecase::agent_session::runtime::ports::AgentSessionEventNotifier,
                > = Arc::new(
                    adaptor::presenter::agent_session::TauriAgentSessionEventNotifier::new(
                        app.handle().clone(),
                    ),
                );
                let runtime_spawner: Arc<
                    dyn usecase::agent_session::runtime::ports::AgentTaskSpawner,
                > = Arc::new(adaptor::gateway::agent_session::TokioAgentTaskSpawner);
                let runtime_branch_diff_context = app
                    .state::<Arc<dyn usecase::agent_session::context::BranchDiffContextPort>>()
                    .inner()
                    .clone();
                let runtime_instruction_source: Arc<
                    dyn usecase::agent_session::context::InstructionSourcePort,
                > = Arc::new(adaptor::gateway::agent_session::FileSystemInstructionSourceGateway);
                // The fixed app-data path was resolved and classified once at
                // the startup boundary above. Reuse that authority instead of
                // introducing a later unclassified resolution/panic path.
                let runtime_data_dir = data_dir.clone();
                let workspace_query = app
                    .state::<Arc<dyn usecase::workspace_tree::WorkspaceQueryService>>()
                    .inner()
                    .clone();
                let runtime_usecase = compose_agent_session_runtime(
                    runtime_session_store.clone(),
                    runtime_registry,
                    agent_status_center.clone(),
                    agent_status_notifier.clone(),
                    runtime_notifier,
                    runtime_spawner,
                    Some(runtime_branch_diff_context),
                    runtime_instruction_source,
                    runtime_data_dir,
                    workspace_query,
                );
                adaptor::controller::event_log_recovery_wiring::register_event_log_recovery_listener(
                    runtime_session_store.clone(),
                    &runtime_usecase,
                );
                app.manage(runtime_usecase);
                let stored_lifecycle_registry = app
                    .state::<Arc<usecase::agent_session::backend_registry::AgentBackendRegistry>>()
                    .inner()
                    .clone();
                let stored_lifecycle_runtime = app
                    .state::<Arc<usecase::agent_session::runtime::AgentSessionRuntimeUsecase>>()
                    .inner()
                    .clone();
                let stored_lifecycle_notice = app
                    .state::<Arc<usecase::agent_session::notice::AgentSessionNoticeUsecase>>()
                    .inner()
                    .clone();
                let stored_lifecycle_open_tabs = app
                    .state::<Arc<usecase::agent_session::session::OpenTabRegistry>>()
                    .inner()
                    .clone();
                let workflow_node_restorer = Arc::new(
                    adaptor::controller::wiring::build_node_execution_lifecycle_usecase(
                        app.handle().clone(),
                        runtime_session_store.clone(),
                        stored_lifecycle_runtime.clone(),
                        stored_lifecycle_open_tabs,
                    ),
                );
                app.manage(workflow_node_restorer.clone());
                let stored_session_lifecycle = Arc::new(
                    adaptor::controller::wiring::build_stored_session_lifecycle_usecase(
                        runtime_session_store,
                        stored_lifecycle_registry,
                        stored_lifecycle_runtime,
                        workflow_node_restorer,
                        stored_lifecycle_notice,
                    ),
                );
                app.manage(stored_session_lifecycle.clone());
            }
            let agent_runtime = app
                .state::<Arc<usecase::agent_session::runtime::AgentSessionRuntimeUsecase>>()
                .inner()
                .clone();
            let session_feedback_load_usecase = Arc::new(
                usecase::agent_session::session_feedback_load::SessionFeedbackLoadUsecase::new(
                    agent_runtime.clone(),
                    session_feedback_usecase.clone(),
                ),
            );
            app.manage(session_feedback_load_usecase.clone());
            let operation_gate = Arc::new(
                adaptor::controller::agent_session_operation_wiring::RuntimeAgentSessionOperationGate::new(
                    agent_runtime.clone(),
                    session_store.clone(),
                    data_dir.clone(),
                ),
            );
            let operation_repository: Arc<
                dyn domain::local_event::LocalEventTransactionRepository,
            > = projected_local_event_repository.clone();
            let operation_authority: Arc<
                dyn usecase::agent_session::operation::OperationBindingAuthority,
            > = local_event_store.clone();
            let lifecycle_gate: Arc<
                dyn usecase::agent_session::operation::SessionLifecycleGate,
            > = operation_gate.clone();
            let stop_gate: Arc<dyn usecase::agent_session::operation::StopAdmissionGate> =
                operation_gate.clone();
            let send_operation_gate = Arc::new(
                adaptor::controller::agent_session_operation_wiring::RuntimeSendOperationGate::new(
                    agent_runtime.clone(),
                    session_store.clone(),
                    data_dir.clone(),
                ),
            );
            let send_gate: Arc<dyn usecase::agent_session::operation::SendAdmissionGate> =
                send_operation_gate.clone();
            let lifecycle_operation = Arc::new(
                usecase::agent_session::operation::SessionLifecycleOperationUsecase::new(
                    operation_repository.clone(),
                    operation_authority.clone(),
                    lifecycle_gate,
                    local_event_store.installation_id().to_string(),
                ),
            );
            app.manage(lifecycle_operation.clone());
            let workspace_node_resolver: Arc<
                dyn usecase::workflow::WorkspaceNodeActionResolver,
            > = app
                .state::<adaptor::controller::state::AppState>()
                .workflow_usecase
                .clone();
            app.manage(Arc::new(
                adaptor::controller::wiring::build_workspace_node_command_usecase(
                    workspace_node_resolver,
                    lifecycle_operation.clone(),
                    session_store.clone(),
                    data_dir.clone(),
                ),
            ));
            let stop_operation = Arc::new(
                usecase::agent_session::operation::StopOperationUsecase::new(
                    operation_repository.clone(),
                    operation_authority.clone(),
                    stop_gate,
                    local_event_store.installation_id().to_string(),
                ),
            );
            operation_gate.bind_stop_operation(Arc::downgrade(&stop_operation));
            adaptor::controller::agent_session_operation_wiring::bind_runtime_durable_stop_driver(
                &agent_runtime,
                stop_operation.clone(),
            );
            app.manage(stop_operation.clone());
            let send_operation = Arc::new(
                usecase::agent_session::operation::AgentSendOperationUsecase::new(
                    projected_local_event_repository.clone(),
                    local_event_store.clone(),
                    send_gate,
                    local_event_store.installation_id().to_string(),
                ),
            );
            operation_gate.bind_send_operation(Arc::downgrade(&send_operation));
            send_operation_gate.bind_status_sink(Arc::downgrade(&send_operation));
            adaptor::controller::agent_session_operation_wiring::bind_runtime_durable_workflow_send_driver(
                &agent_runtime,
                send_operation.clone(),
                session_store.clone(),
                data_dir.clone(),
            );
            adaptor::controller::agent_session_operation_wiring::bind_runtime_terminal_operation_participant_provider(
                &session_store,
                stop_operation.clone(),
                send_operation.clone(),
            );
            let pending_stop_recovery = stop_operation.clone();
            tauri::async_runtime::spawn(async move {
                run_startup_recovery(
                    "pending accepted Stop",
                    || {
                        let recovery = pending_stop_recovery.clone();
                        async move { recovery.recover_pending_stops_pass().await }
                    },
                    std::time::Duration::from_millis(50),
                    std::time::Duration::from_secs(1),
                )
                .await;
            });
            let pending_send_recovery = send_operation.clone();
            let pending_send_wakeup = send_operation.pending_recovery_wakeup();
            let pending_send_signal_store = local_event_store.clone();
            tauri::async_runtime::spawn(async move {
                run_wakeable_recovery(
                    "pending accepted send",
                    || {
                        let recovery = pending_send_recovery.clone();
                        async move { recovery.recover_pending_provider_effects_pass().await }
                    },
                    move || {
                        domain::local_event::LocalEventTransactionRepository::subscribe(
                            pending_send_signal_store.as_ref(),
                            domain::local_event::GlobalSequence::new(
                                domain::local_event::GlobalSequence::MIN,
                            )
                            .expect("minimum global sequence"),
                        )
                    },
                    pending_send_wakeup,
                    std::time::Duration::from_millis(50),
                    std::time::Duration::from_secs(1),
                )
                .await;
            });
            app.manage(send_operation.clone());
            let permission_response_gate: Arc<
                dyn usecase::agent_session::operation::PermissionResponseGate,
            > = Arc::new(
                adaptor::controller::agent_session_operation_wiring::RuntimePermissionResponseOperationGate::new(
                    agent_runtime.clone(),
                    session_store.clone(),
                ),
            );
            let permission_response_operation = Arc::new(
                usecase::agent_session::operation::PermissionResponseOperationUsecase::new(
                    operation_repository.clone(),
                    operation_authority.clone(),
                    permission_response_gate,
                    local_event_store.installation_id().to_string(),
                ),
            );
            let pending_permission_recovery = permission_response_operation.clone();
            tauri::async_runtime::spawn(async move {
                run_startup_recovery(
                    "pending permission response",
                    || {
                        let recovery = pending_permission_recovery.clone();
                        async move { recovery.recover_pending_permission_responses_pass().await }
                    },
                    std::time::Duration::from_millis(50),
                    std::time::Duration::from_secs(1),
                )
                .await;
            });
            app.manage(permission_response_operation.clone());
            let recovery_operation = Arc::new(
                usecase::agent_session::operation::RecoveryActionUsecase::new(
                    projected_local_event_repository.clone(),
                    local_event_store.clone(),
                    Arc::new(
                        adaptor::controller::agent_session_operation_wiring::ConservativeRecoveryExecutor::new(
                            stop_operation.clone(),
                            lifecycle_operation.clone(),
                            operation_gate.clone(),
                            adaptor::controller::agent_session_operation_wiring::ActiveSendRecoveryContext::new(
                                send_operation.clone(),
                                agent_runtime.clone(),
                                send_operation_gate.current_process_claims(),
                            ),
                            permission_response_operation.clone(),
                            local_event_store.clone(),
                        ),
                    ),
                    local_event_store.installation_id().to_string(),
                ),
            );
            app.manage(recovery_operation.clone());
            let caller_journal = Arc::new(
                usecase::agent_session::operation::CallerAttemptJournal::new(
                    projected_local_event_repository.clone(),
                    local_event_store.clone(),
                    local_event_store.installation_id().to_string(),
                ),
            );
            app.manage(caller_journal.clone());
            let open_tabs = app
                .state::<Arc<usecase::agent_session::session::OpenTabRegistry>>()
                .inner()
                .clone();
            let branch_diff_context = app
                .state::<Arc<dyn usecase::agent_session::context::BranchDiffContextPort>>()
                .inner()
                .clone();
            let workflow_runtime_usecase = Arc::new(
                adaptor::controller::wiring::build_workflow_runtime_usecase(
                    app.handle().clone(),
                    adaptor::gateway::workflow::TauriWorkflowRuntimeCommandGatewayDeps {
                        repository_usecase: repository_usecase.clone(),
                        app_config: config_repository.clone(),
                        session_store: session_store.clone(),
                        agent_runtime: agent_runtime.clone(),
                        open_tabs,
                        branch_diff_context: branch_diff_context.clone(),
                        data_dir: Some(data_dir.clone()),
                        local_event_repository: projected_local_event_repository.clone(),
                        local_event_installation_id: local_event_store
                            .installation_id()
                            .to_string(),
                    },
                )
                .map_err(|error| format!("workflow recovery admission failed: {error}"))?,
            );
            send_operation_gate
                .bind_workflow_runtime(Arc::downgrade(&workflow_runtime_usecase));
            let workflow_runtime_agent_notifier = Arc::new(
                adaptor::gateway::agent_session::WorkflowRuntimeAgentSessionNotifier::new(
                    workflow_runtime_usecase.clone(),
                    session_store.clone(),
                ),
            );
            agent_runtime
                .set_workflow_turn_complete_notifier(workflow_runtime_agent_notifier.clone());
            agent_runtime
                .set_workflow_stall_notifier(workflow_runtime_agent_notifier.clone());
            let pending_workflow_recovery = workflow_runtime_usecase.clone();
            let pending_turn_completion_recovery = workflow_runtime_agent_notifier.clone();
            tauri::async_runtime::spawn(async move {
                run_startup_recovery(
                    "pending workflow turn-completion/orphan",
                    || {
                        let workflow = pending_workflow_recovery.clone();
                        let turn_completion = pending_turn_completion_recovery.clone();
                        async move {
                            let report = turn_completion
                                .recover_pending_turn_completions()
                                .await?;
                            workflow
                                .recover_startup_excluding(&report.unresolved_execution_ids)
                                .await
                                .map_err(|error| error.to_string())?;
                            if report.transient_failures != 0 {
                                return Err(format!(
                                    "{} workflow turn-completion item(s) remain transiently unresolved",
                                    report.transient_failures
                                ));
                            }
                            Ok::<usize, String>(report.terminal_count)
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
                    agent_runtime.clone(),
                    workflow_runtime_usecase.clone(),
                    lifecycle_operation,
                    shutdown_local_api,
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
                spawn_startup_maintenance(app_data.clone(), shared_repo_paths.clone());
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
                    Some(adaptor::controller::api::AgentSessionApiDeps::new(
                        send_operation,
                        permission_response_operation,
                        stop_operation,
                        recovery_operation,
                        session_feedback_usecase,
                        session_feedback_load_usecase,
                        shutdown_coordinator.clone(),
                        process_actions.clone(),
                        local_event_store.clone(),
                        caller_journal.clone(),
                        app.handle().clone(),
                    )),
                );
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
