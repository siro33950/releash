use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::usecase::shutdown_coordinator::{
    ApplicationProcessAction, ApplicationQuitIntent, ApplicationQuitOutcome,
    ApplicationQuitRequest, ShutdownCoordinator, ShutdownEffectReadback, ShutdownTarget,
    ShutdownTargetExecutor,
};

pub(crate) struct TauriProcessLocalExitPort {
    app: tauri::AppHandle,
}

impl TauriProcessLocalExitPort {
    pub(crate) fn new(app: tauri::AppHandle) -> Self {
        Self { app }
    }
}

fn dispatch_startup_failure_process_exit(exit: impl FnOnce()) {
    // `AppHandle::exit` itself emits `RunEvent::ExitRequested`. Grant the
    // native exit before dispatch so the lifecycle hook does not prevent the
    // one process-local effect and then join its own already-fired flight.
    crate::infrastructure::platform::tray::mark_quit_requested();
    exit();
}

impl crate::usecase::application_startup::ProcessLocalExitPort for TauriProcessLocalExitPort {
    fn exit(&self, code: i32) {
        dispatch_startup_failure_process_exit(|| self.app.exit(code));
    }
}

#[derive(Clone)]
pub(crate) struct ApplicationQuitIngress {
    handler: Arc<dyn Fn(ApplicationQuitIntent) + Send + Sync>,
}

impl ApplicationQuitIngress {
    pub(crate) fn new(handler: impl Fn(ApplicationQuitIntent) + Send + Sync + 'static) -> Self {
        Self {
            handler: Arc::new(handler),
        }
    }

    pub(crate) fn request(&self, intent: ApplicationQuitIntent) {
        (self.handler)(intent);
    }
}

/// Platform boundary for the single process destination granted by shutdown.
pub(crate) trait ApplicationProcessActionPort: Send + Sync {
    fn execute(&self, action: ApplicationProcessAction);
}

struct TauriApplicationProcessActionPort {
    app: tauri::AppHandle,
}

impl ApplicationProcessActionPort for TauriApplicationProcessActionPort {
    fn execute(&self, action: ApplicationProcessAction) {
        match action {
            ApplicationProcessAction::Exit { code } => self.app.exit(code),
            // `request_restart` selects Tauri's relaunch path. The signed code
            // remains part of the durable/public action even though Tauri uses
            // its own reserved restart exit code for the process handoff.
            ApplicationProcessAction::Restart { .. } => self.app.request_restart(),
        }
    }
}

/// Process-local exactly-once fence shared by Tauri, native, and WebSocket
/// ingress. A new boot receives a new dispatcher, allowing durable recovery to
/// perform the same saved destination once after a pre-handoff crash.
#[derive(Default)]
pub(crate) struct ApplicationProcessActionDispatcher {
    dispatched: AtomicBool,
}

impl ApplicationProcessActionDispatcher {
    pub(crate) fn dispatch(
        &self,
        port: &dyn ApplicationProcessActionPort,
        action: ApplicationProcessAction,
    ) -> bool {
        if self
            .dispatched
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return false;
        }
        crate::infrastructure::platform::tray::mark_quit_requested();
        port.execute(action);
        true
    }

    pub(crate) fn dispatch_tauri(
        &self,
        app: tauri::AppHandle,
        action: ApplicationProcessAction,
    ) -> bool {
        self.dispatch(&TauriApplicationProcessActionPort { app }, action)
    }
}

struct RuntimeShutdownExecutor {
    workflow_runtime: Arc<crate::usecase::workflow::WorkflowRuntimeUsecase>,
    terminal_surface:
        Arc<crate::usecase::terminal_surface::application::TerminalSurfaceApplication>,
    shutdown_provider_exit_observer: Arc<dyn Fn() + Send + Sync>,
    shutdown_local_api: Arc<dyn Fn() + Send + Sync>,
}

fn workflow_shutdown_targets(execution_ids: Vec<String>) -> Vec<ShutdownTarget> {
    execution_ids
        .into_iter()
        .map(|target_id| ShutdownTarget {
            target_id,
            kind: "workflow_execution".to_string(),
        })
        .collect()
}

pub(crate) struct RuntimeShutdownDependencies {
    workflow_runtime: Arc<crate::usecase::workflow::WorkflowRuntimeUsecase>,
    terminal_surface:
        Arc<crate::usecase::terminal_surface::application::TerminalSurfaceApplication>,
    shutdown_provider_exit_observer: Arc<dyn Fn() + Send + Sync>,
    shutdown_local_api: Arc<dyn Fn() + Send + Sync>,
}

impl RuntimeShutdownDependencies {
    pub(crate) fn new(
        workflow_runtime: Arc<crate::usecase::workflow::WorkflowRuntimeUsecase>,
        terminal_surface: Arc<
            crate::usecase::terminal_surface::application::TerminalSurfaceApplication,
        >,
        shutdown_provider_exit_observer: Arc<dyn Fn() + Send + Sync>,
        shutdown_local_api: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        Self {
            workflow_runtime,
            terminal_surface,
            shutdown_provider_exit_observer,
            shutdown_local_api,
        }
    }
}

fn shutdown_provider_observer_terminal_surface_and_local_api(
    shutdown_provider_exit_observer: &(dyn Fn() + Send + Sync),
    shutdown_terminal_surface: &(dyn Fn() -> Result<(), String> + Send + Sync),
    shutdown_local_api: &(dyn Fn() + Send + Sync),
) -> Result<(), String> {
    shutdown_provider_exit_observer();
    shutdown_terminal_surface()?;
    shutdown_local_api();
    Ok(())
}

#[async_trait::async_trait]
impl ShutdownTargetExecutor for RuntimeShutdownExecutor {
    async fn targets(
        &self,
    ) -> Result<Vec<ShutdownTarget>, crate::domain::local_event::SafeOperationFailure> {
        let workflow_ids = self
            .workflow_runtime
            .application_shutdown_target_execution_ids()
            .await
            .map_err(|_| {
                crate::domain::local_event::SafeOperationFailure::new(
                    crate::domain::local_event::SessionOperationFailureKind::StorageUnavailable,
                    true,
                    "The workflow shutdown inventory could not be fixed.",
                    uuid::Uuid::new_v4().to_string(),
                )
            })?;
        Ok(workflow_shutdown_targets(workflow_ids))
    }

    async fn execute_target(
        &self,
        operation_id: &str,
        effect_identity: &str,
        owner_revision: crate::domain::local_event::Revision,
        target: &ShutdownTarget,
    ) -> Result<(), crate::domain::local_event::SafeOperationFailure> {
        log::debug!(
            "shutdown effect operation={operation_id} identity={effect_identity} revision={}",
            owner_revision.value()
        );
        let result = match target.kind.as_str() {
            "workflow_execution" => match self
                .workflow_runtime
                    .shutdown_execution_commands_for_effect(
                        operation_id,
                        effect_identity,
                        owner_revision.value(),
                        &target.target_id,
                    )
                    .await
                {
                    crate::usecase::workflow::ports::WorkflowShutdownEffectReadback::Completed => Ok(()),
                    crate::usecase::workflow::ports::WorkflowShutdownEffectReadback::ConfirmedNotStarted => {
                        Err("workflow shutdown effect was not started".to_string())
                    }
                    crate::usecase::workflow::ports::WorkflowShutdownEffectReadback::Ambiguous => {
                        Err("workflow shutdown effect requires reconciliation".to_string())
                    }
                },
            _ => Err("unknown shutdown target kind".to_string()),
        };
        result.map_err(|_| {
            crate::domain::local_event::SafeOperationFailure::new(
                crate::domain::local_event::SessionOperationFailureKind::ExternalEffectFailed,
                true,
                "A shutdown target could not be quiesced.",
                uuid::Uuid::new_v4().to_string(),
            )
        })
    }

    async fn read_target_effect(
        &self,
        operation_id: &str,
        effect_identity: &str,
        owner_revision: crate::domain::local_event::Revision,
        target: &ShutdownTarget,
    ) -> Result<ShutdownEffectReadback, crate::domain::local_event::SafeOperationFailure> {
        match target.kind.as_str() {
            "workflow_execution" => Ok(match self
                    .workflow_runtime
                    .read_shutdown_execution_effect(
                        operation_id,
                        effect_identity,
                        owner_revision.value(),
                        &target.target_id,
                    )
                    .await
                {
                    crate::usecase::workflow::ports::WorkflowShutdownEffectReadback::Completed => {
                        ShutdownEffectReadback::Completed
                    }
                    crate::usecase::workflow::ports::WorkflowShutdownEffectReadback::ConfirmedNotStarted => {
                        ShutdownEffectReadback::ConfirmedNotStarted
                    }
                    crate::usecase::workflow::ports::WorkflowShutdownEffectReadback::Ambiguous => {
                        ShutdownEffectReadback::Ambiguous
                    }
                }),
            _ => Ok(ShutdownEffectReadback::Ambiguous),
        }
    }

    async fn shutdown_subordinates(
        &self,
    ) -> Result<(), crate::domain::local_event::SafeOperationFailure> {
        shutdown_provider_observer_terminal_surface_and_local_api(
            self.shutdown_provider_exit_observer.as_ref(),
            &|| {
                self.terminal_surface
                    .shutdown()
                    .map_err(|error| error.to_string())
            },
            self.shutdown_local_api.as_ref(),
        )
        .map_err(|_| {
            crate::domain::local_event::SafeOperationFailure::new(
                crate::domain::local_event::SessionOperationFailureKind::StorageUnavailable,
                true,
                "Terminal Surface processes could not be stopped and persisted before shutdown.",
                uuid::Uuid::new_v4().to_string(),
            )
        })
    }
}

pub(crate) fn build_shutdown_coordinator(
    store: Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>,
    repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository>,
    dependencies: RuntimeShutdownDependencies,
) -> Arc<ShutdownCoordinator> {
    let installation_id = store.installation_id().to_string();
    let process_instance_id = store.process_instance_id().to_string();
    let authority: Arc<
        dyn crate::usecase::application_lifecycle::operation::OperationBindingAuthority,
    > = store.clone();
    let executor: Arc<dyn ShutdownTargetExecutor> = Arc::new(RuntimeShutdownExecutor {
        workflow_runtime: dependencies.workflow_runtime,
        terminal_surface: dependencies.terminal_surface,
        shutdown_provider_exit_observer: dependencies.shutdown_provider_exit_observer,
        shutdown_local_api: dependencies.shutdown_local_api,
    });
    Arc::new(ShutdownCoordinator::new(
        repository,
        authority,
        executor,
        installation_id,
        process_instance_id,
    ))
}

pub(crate) fn request_application_quit(
    app: tauri::AppHandle,
    coordinator: Arc<ShutdownCoordinator>,
    process_actions: Arc<ApplicationProcessActionDispatcher>,
    intent: ApplicationQuitIntent,
) {
    tauri::async_runtime::spawn(async move {
        let request_id = format!("quit-{}", uuid::Uuid::new_v4());
        match coordinator
            .request(ApplicationQuitRequest {
                principal: crate::usecase::application_lifecycle::operation::LOCAL_INSTALLATION_OPERATION_PRINCIPAL.to_string(),
                request_id,
                intent,
            })
            .await
        {
            Ok(ApplicationQuitOutcome::Accepted { receipt, state })
                if state.grants_exit_permit() =>
            {
                process_actions.dispatch_tauri(app, receipt.intent.into());
            }
            Ok(ApplicationQuitOutcome::Accepted { .. })
            | Ok(ApplicationQuitOutcome::OutcomeUnknown { .. })
            | Ok(ApplicationQuitOutcome::RejectedBeforeCommit { .. })
            | Err(_) => {
                crate::infrastructure::platform::tray::QUIT_REQUESTED
                    .store(false, Ordering::SeqCst);
                log::error!("application shutdown aborted before durable activation");
            }
        }
    });
}

#[cfg(test)]
#[path = "application_lifecycle_test.rs"]
mod application_lifecycle_tests;

#[cfg(test)]
mod tests {
    use super::{
        dispatch_startup_failure_process_exit, ApplicationProcessActionDispatcher,
        ApplicationProcessActionPort,
    };
    use crate::usecase::shutdown_coordinator::ApplicationProcessAction;
    use std::sync::{Arc, Mutex};

    #[test]
    fn startup_failure_process_exit_grants_the_native_exit_before_dispatch() {
        let _guard = crate::infrastructure::platform::tray::QUIT_REQUESTED_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        crate::infrastructure::platform::tray::QUIT_REQUESTED
            .store(false, std::sync::atomic::Ordering::SeqCst);
        let calls = std::sync::atomic::AtomicUsize::new(0);

        dispatch_startup_failure_process_exit(|| {
            assert!(
                !crate::infrastructure::platform::window_lifecycle::should_prevent_exit(),
                "the native ExitRequested callback must observe an exit permit"
            );
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        });

        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        crate::infrastructure::platform::tray::QUIT_REQUESTED
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }

    #[derive(Default)]
    struct RecordingProcessPort {
        actions: Mutex<Vec<ApplicationProcessAction>>,
    }

    impl ApplicationProcessActionPort for RecordingProcessPort {
        fn execute(&self, action: ApplicationProcessAction) {
            self.actions
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(action);
        }
    }

    #[test]
    fn f11_concrete_process_port_routes_exit_and_restart_to_distinct_destinations() {
        let _guard = crate::infrastructure::platform::tray::QUIT_REQUESTED_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        crate::infrastructure::platform::tray::QUIT_REQUESTED
            .store(false, std::sync::atomic::Ordering::SeqCst);

        let exit_port = RecordingProcessPort::default();
        let exit_dispatcher = ApplicationProcessActionDispatcher::default();
        assert!(exit_dispatcher.dispatch(&exit_port, ApplicationProcessAction::Exit { code: -7 },));
        assert_eq!(
            exit_port
                .actions
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .as_slice(),
            &[ApplicationProcessAction::Exit { code: -7 }]
        );

        let restart_port = RecordingProcessPort::default();
        let restart_dispatcher = ApplicationProcessActionDispatcher::default();
        assert!(restart_dispatcher.dispatch(
            &restart_port,
            ApplicationProcessAction::Restart { code: 42 },
        ));
        assert_eq!(
            restart_port
                .actions
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .as_slice(),
            &[ApplicationProcessAction::Restart { code: 42 }]
        );
    }

    #[test]
    fn f11_first_process_destination_is_one_shot_across_concurrent_surface_replays() {
        let _guard = crate::infrastructure::platform::tray::QUIT_REQUESTED_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        crate::infrastructure::platform::tray::QUIT_REQUESTED
            .store(false, std::sync::atomic::Ordering::SeqCst);

        let dispatcher = Arc::new(ApplicationProcessActionDispatcher::default());
        let port = Arc::new(RecordingProcessPort::default());
        assert!(dispatcher.dispatch(
            port.as_ref(),
            ApplicationProcessAction::Restart { code: 23 },
        ));

        let replays = (0..16)
            .map(|index| {
                let dispatcher = dispatcher.clone();
                let port = port.clone();
                std::thread::spawn(move || {
                    let changed_surface_intent = if index % 2 == 0 {
                        ApplicationProcessAction::Exit { code: index }
                    } else {
                        ApplicationProcessAction::Restart { code: index }
                    };
                    dispatcher.dispatch(port.as_ref(), changed_surface_intent)
                })
            })
            .collect::<Vec<_>>();
        assert!(replays.into_iter().all(|join| !join.join().unwrap()));
        assert_eq!(
            port.actions
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .as_slice(),
            &[ApplicationProcessAction::Restart { code: 23 }],
            "same/different identity replays and response loss cannot change or repeat the first process action"
        );
    }
}
