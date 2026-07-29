use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::domain::local_event::{SafeOperationFailure, SessionOperationFailureKind};
use crate::usecase::agent_session::operation::{
    SessionLifecycleCommandResult, SessionLifecycleOperationError, SessionLifecycleOperationState,
    SessionLifecycleRejection,
};
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
    runtime: Arc<crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase>,
    workflow_runtime: Arc<crate::usecase::workflow::WorkflowRuntimeUsecase>,
    lifecycle_operation:
        Arc<crate::usecase::agent_session::operation::SessionLifecycleOperationUsecase>,
    shutdown_local_api: Arc<dyn Fn() + Send + Sync>,
}

fn shutdown_effect_request_id(effect_identity: &str) -> String {
    use sha2::{Digest, Sha256};
    format!(
        "shutdown-{}",
        hex::encode(Sha256::digest(effect_identity.as_bytes()))
    )
}

fn shutdown_lifecycle_failure(
    effect_identity: &str,
    correlation_suffix: &str,
    kind: SessionOperationFailureKind,
    retryable: bool,
    label: &str,
) -> SafeOperationFailure {
    SafeOperationFailure::new(
        kind,
        retryable,
        label,
        format!(
            "{}-{correlation_suffix}",
            shutdown_effect_request_id(effect_identity)
        ),
    )
}

fn classify_session_lifecycle_shutdown_result(
    effect_identity: &str,
    result: Result<SessionLifecycleCommandResult, SessionLifecycleOperationError>,
) -> Result<(), SafeOperationFailure> {
    match result {
        Ok(SessionLifecycleCommandResult::Accepted {
            state: SessionLifecycleOperationState::Completed,
            ..
        }) => Ok(()),
        Ok(SessionLifecycleCommandResult::Accepted {
            state: SessionLifecycleOperationState::ReconciliationRequired { failure },
            ..
        })
        | Ok(SessionLifecycleCommandResult::Rejected(SessionLifecycleRejection::Failed {
            failure,
        }))
        | Err(SessionLifecycleOperationError::StorageUnavailable { failure }) => Err(failure),
        Ok(SessionLifecycleCommandResult::Accepted {
            state: SessionLifecycleOperationState::Accepted,
            ..
        }) => Err(shutdown_lifecycle_failure(
            effect_identity,
            "lifecycle-pending",
            SessionOperationFailureKind::OutcomeUnknown,
            true,
            "The session shutdown effect has not reached a durable terminal result.",
        )),
        Ok(SessionLifecycleCommandResult::OutcomeUnknown { .. }) => {
            Err(shutdown_lifecycle_failure(
                effect_identity,
                "lifecycle-outcome-unknown",
                SessionOperationFailureKind::OutcomeUnknown,
                true,
                "The session shutdown acceptance result is unknown.",
            ))
        }
        Ok(SessionLifecycleCommandResult::Rejected(
            SessionLifecycleRejection::Busy
            | SessionLifecycleRejection::PendingOperation
            | SessionLifecycleRejection::RevisionConflict { .. }
            | SessionLifecycleRejection::InvalidState,
        )) => Err(shutdown_lifecycle_failure(
            effect_identity,
            "lifecycle-rejected",
            SessionOperationFailureKind::TargetRevisionChanged,
            true,
            "The session shutdown target changed before the close operation was accepted.",
        )),
        Err(
            SessionLifecycleOperationError::InvalidRequest
            | SessionLifecycleOperationError::PayloadConflict,
        ) => Err(shutdown_lifecycle_failure(
            effect_identity,
            "lifecycle-invalid-effect-intent",
            SessionOperationFailureKind::InvalidEffectIntent,
            false,
            "The session shutdown effect intent is invalid or conflicts with its saved identity.",
        )),
        Err(SessionLifecycleOperationError::ShutdownInProgress) => Err(shutdown_lifecycle_failure(
            effect_identity,
            "lifecycle-shutdown-authority",
            SessionOperationFailureKind::ShutdownAuthorityMismatch,
            true,
            "The session shutdown target is not bound to the active shutdown authority.",
        )),
        Err(SessionLifecycleOperationError::NotFound) => Err(shutdown_lifecycle_failure(
            effect_identity,
            "lifecycle-target-changed",
            SessionOperationFailureKind::TargetRevisionChanged,
            true,
            "The session shutdown target changed before lifecycle execution.",
        )),
        Err(SessionLifecycleOperationError::QueryBusy) => Err(shutdown_lifecycle_failure(
            effect_identity,
            "lifecycle-query-busy",
            SessionOperationFailureKind::StorageUnavailable,
            true,
            "The session shutdown state could not be read.",
        )),
        Err(SessionLifecycleOperationError::DeadlineExceeded) => Err(shutdown_lifecycle_failure(
            effect_identity,
            "lifecycle-deadline",
            SessionOperationFailureKind::DeadlineExceeded,
            true,
            "The session shutdown lifecycle operation reached its fixed deadline.",
        )),
        Err(SessionLifecycleOperationError::Internal { correlation_id }) => {
            Err(SafeOperationFailure::new(
                SessionOperationFailureKind::Internal,
                false,
                "The session shutdown lifecycle operation failed internally.",
                correlation_id,
            ))
        }
    }
}

#[async_trait::async_trait]
impl ShutdownTargetExecutor for RuntimeShutdownExecutor {
    async fn targets(
        &self,
    ) -> Result<Vec<ShutdownTarget>, crate::domain::local_event::SafeOperationFailure> {
        let session_ids = self
            .runtime
            .application_shutdown_target_session_ids()
            .map_err(|_| {
                crate::domain::local_event::SafeOperationFailure::new(
                    crate::domain::local_event::SessionOperationFailureKind::StorageUnavailable,
                    true,
                    "The agent-session shutdown inventory could not be fixed.",
                    uuid::Uuid::new_v4().to_string(),
                )
            })?;
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
        let targets = session_ids
            .into_iter()
            .map(|target_id| ShutdownTarget {
                target_id,
                kind: "agent_session".to_string(),
            })
            .chain(workflow_ids.into_iter().map(|target_id| ShutdownTarget {
                target_id,
                kind: "workflow_execution".to_string(),
            }))
            .collect::<Vec<_>>();
        Ok(targets)
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
            "agent_session" => match self.runtime.get_session(&target.target_id).await {
                Err(error) => Err(error.to_string()),
                Ok(None) => Ok(()),
                Ok(Some(current)) => {
                    return classify_session_lifecycle_shutdown_result(
                            effect_identity,
                            self.lifecycle_operation
                                .request_shutdown_target(
                            crate::usecase::agent_session::operation::SessionLifecycleRequest {
                                principal: format!("shutdown:{operation_id}"),
                                request_id: shutdown_effect_request_id(effect_identity),
                                session_id: target.target_id.clone(),
                                expected_session_revision: current.session_revision as i64,
                                action: crate::usecase::agent_session::operation::SessionLifecycleAction::Close,
                            },
                        )
                                .await,
                        );
                }
            },
            "workflow_execution" => {
                match self.workflow_runtime
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
                }
            }
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
            "agent_session" => {
                let principal = format!("shutdown:{operation_id}");
                match self
                    .lifecycle_operation
                    .get_operation(&principal, &shutdown_effect_request_id(effect_identity))
                    .await
                {
                    Ok((_, crate::usecase::agent_session::operation::SessionLifecycleOperationState::Completed)) => {
                        return Ok(ShutdownEffectReadback::Completed)
                    }
                    Ok(_) => return Ok(ShutdownEffectReadback::Ambiguous),
                    Err(crate::usecase::agent_session::operation::SessionLifecycleOperationError::NotFound) => {
                        return Ok(ShutdownEffectReadback::ConfirmedNotStarted)
                    }
                    Err(_) => return Ok(ShutdownEffectReadback::Ambiguous),
                }
            }
            "workflow_execution" => {
                return Ok(match self
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
                })
            }
            _ => return Ok(ShutdownEffectReadback::Ambiguous),
        }
    }

    async fn shutdown_subordinates(
        &self,
    ) -> Result<(), crate::domain::local_event::SafeOperationFailure> {
        (self.shutdown_local_api)();
        Ok(())
    }
}

pub(crate) fn build_shutdown_coordinator(
    store: Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>,
    repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository>,
    runtime: Arc<crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase>,
    workflow_runtime: Arc<crate::usecase::workflow::WorkflowRuntimeUsecase>,
    lifecycle_operation: Arc<
        crate::usecase::agent_session::operation::SessionLifecycleOperationUsecase,
    >,
    shutdown_local_api: Arc<dyn Fn() + Send + Sync>,
) -> Arc<ShutdownCoordinator> {
    let installation_id = store.installation_id().to_string();
    let process_instance_id = store.process_instance_id().to_string();
    let authority: Arc<dyn crate::usecase::agent_session::operation::OperationBindingAuthority> =
        store.clone();
    let executor: Arc<dyn ShutdownTargetExecutor> = Arc::new(RuntimeShutdownExecutor {
        runtime,
        workflow_runtime,
        lifecycle_operation,
        shutdown_local_api,
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
                principal: crate::adaptor::controller::agent_session_operation_wiring::LOCAL_INSTALLATION_OPERATION_PRINCIPAL.to_string(),
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
mod tests {
    use super::{
        classify_session_lifecycle_shutdown_result, dispatch_startup_failure_process_exit,
        shutdown_effect_request_id, ApplicationProcessActionDispatcher,
        ApplicationProcessActionPort,
    };
    use crate::domain::local_event::{SafeOperationFailure, SessionOperationFailureKind};
    use crate::usecase::agent_session::operation::{
        SessionLifecycleAction, SessionLifecycleCommandResult, SessionLifecycleOperationError,
        SessionLifecycleOperationState, SessionLifecycleReceipt, SessionLifecycleRejection,
    };
    use crate::usecase::shutdown_coordinator::ApplicationProcessAction;
    use std::sync::{Arc, Mutex};

    const TEST_EFFECT_IDENTITY: &str = "shutdown-effect-lifecycle-result";

    fn lifecycle_result(
        state: SessionLifecycleOperationState,
    ) -> Result<SessionLifecycleCommandResult, SessionLifecycleOperationError> {
        Ok(SessionLifecycleCommandResult::Accepted {
            receipt: SessionLifecycleReceipt {
                operation_id: "lifecycle-operation".to_string(),
                session_id: "session-1".to_string(),
                action: SessionLifecycleAction::Close,
                first_accepted_revision: 7,
            },
            state,
        })
    }

    fn expected_shutdown_correlation(suffix: &str) -> String {
        format!(
            "{}-{suffix}",
            shutdown_effect_request_id(TEST_EFFECT_IDENTITY)
        )
    }

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

    #[test]
    fn runtime_shutdown_lifecycle_accepts_only_completed_state() {
        assert_eq!(
            classify_session_lifecycle_shutdown_result(
                TEST_EFFECT_IDENTITY,
                lifecycle_result(SessionLifecycleOperationState::Completed),
            ),
            Ok(())
        );

        let failure = classify_session_lifecycle_shutdown_result(
            TEST_EFFECT_IDENTITY,
            lifecycle_result(SessionLifecycleOperationState::Accepted),
        )
        .expect_err("nonterminal Accepted state must not complete the shutdown target");
        assert_eq!(failure.kind, SessionOperationFailureKind::OutcomeUnknown);
        assert!(failure.retryable);
        assert_eq!(
            failure.correlation_id,
            expected_shutdown_correlation("lifecycle-pending")
        );
    }

    #[test]
    fn runtime_shutdown_lifecycle_preserves_reconciliation_failure() {
        let expected = SafeOperationFailure::new(
            SessionOperationFailureKind::ProviderUnavailable,
            true,
            "The provider result requires reconciliation.",
            "provider-reconciliation",
        );
        let failure = classify_session_lifecycle_shutdown_result(
            TEST_EFFECT_IDENTITY,
            lifecycle_result(SessionLifecycleOperationState::ReconciliationRequired {
                failure: expected.clone(),
            }),
        )
        .expect_err("reconciliation-required state must not complete the shutdown target");
        assert_eq!(failure, expected);
    }

    #[test]
    fn runtime_shutdown_lifecycle_maps_outcome_unknown_to_stable_failure() {
        let result = || {
            classify_session_lifecycle_shutdown_result(
                TEST_EFFECT_IDENTITY,
                Ok(SessionLifecycleCommandResult::OutcomeUnknown {
                    request_id: "shutdown-request".to_string(),
                }),
            )
            .expect_err("unknown acceptance outcome must not complete the shutdown target")
        };
        let failure = result();
        assert_eq!(failure.kind, SessionOperationFailureKind::OutcomeUnknown);
        assert!(failure.retryable);
        assert_eq!(
            failure.correlation_id,
            expected_shutdown_correlation("lifecycle-outcome-unknown")
        );
        assert_eq!(result(), failure, "same effect identity must replay stably");
    }

    #[test]
    fn runtime_shutdown_lifecycle_propagates_every_rejection() {
        for rejection in [
            SessionLifecycleRejection::Busy,
            SessionLifecycleRejection::PendingOperation,
            SessionLifecycleRejection::RevisionConflict {
                current_revision: 8,
            },
            SessionLifecycleRejection::InvalidState,
        ] {
            let classify = || {
                classify_session_lifecycle_shutdown_result(
                    TEST_EFFECT_IDENTITY,
                    Ok(SessionLifecycleCommandResult::Rejected(rejection.clone())),
                )
                .expect_err("pre-acceptance rejection must not complete the shutdown target")
            };
            let failure = classify();
            assert_eq!(
                failure.kind,
                SessionOperationFailureKind::TargetRevisionChanged
            );
            assert!(failure.retryable);
            assert_eq!(
                failure.correlation_id,
                expected_shutdown_correlation("lifecycle-rejected")
            );
            assert_eq!(
                classify(),
                failure,
                "same rejected target must replay stably"
            );
        }

        let expected = SafeOperationFailure::new(
            SessionOperationFailureKind::CapacityExceeded,
            false,
            "The lifecycle operation was rejected.",
            "lifecycle-rejected-failure",
        );
        let failure = classify_session_lifecycle_shutdown_result(
            TEST_EFFECT_IDENTITY,
            Ok(SessionLifecycleCommandResult::Rejected(
                SessionLifecycleRejection::Failed {
                    failure: expected.clone(),
                },
            )),
        )
        .expect_err("typed lifecycle rejection must not complete the shutdown target");
        assert_eq!(failure, expected);
    }

    #[test]
    fn runtime_shutdown_lifecycle_maps_every_operation_error() {
        for (error, kind, retryable, suffix) in [
            (
                SessionLifecycleOperationError::InvalidRequest,
                SessionOperationFailureKind::InvalidEffectIntent,
                false,
                "lifecycle-invalid-effect-intent",
            ),
            (
                SessionLifecycleOperationError::PayloadConflict,
                SessionOperationFailureKind::InvalidEffectIntent,
                false,
                "lifecycle-invalid-effect-intent",
            ),
            (
                SessionLifecycleOperationError::ShutdownInProgress,
                SessionOperationFailureKind::ShutdownAuthorityMismatch,
                true,
                "lifecycle-shutdown-authority",
            ),
            (
                SessionLifecycleOperationError::NotFound,
                SessionOperationFailureKind::TargetRevisionChanged,
                true,
                "lifecycle-target-changed",
            ),
            (
                SessionLifecycleOperationError::QueryBusy,
                SessionOperationFailureKind::StorageUnavailable,
                true,
                "lifecycle-query-busy",
            ),
            (
                SessionLifecycleOperationError::DeadlineExceeded,
                SessionOperationFailureKind::DeadlineExceeded,
                true,
                "lifecycle-deadline",
            ),
        ] {
            let classify = || {
                classify_session_lifecycle_shutdown_result(TEST_EFFECT_IDENTITY, Err(error.clone()))
                    .expect_err("lifecycle operation error must not complete the shutdown target")
            };
            let failure = classify();
            assert_eq!(failure.kind, kind);
            assert_eq!(failure.retryable, retryable);
            assert_eq!(
                failure.correlation_id,
                expected_shutdown_correlation(suffix)
            );
            assert_eq!(
                classify(),
                failure,
                "same lifecycle operation error must replay stably"
            );
        }

        let expected_storage = SafeOperationFailure::new(
            SessionOperationFailureKind::StorageUnavailable,
            true,
            "The lifecycle store is unavailable.",
            "lifecycle-storage",
        );
        let storage = classify_session_lifecycle_shutdown_result(
            TEST_EFFECT_IDENTITY,
            Err(SessionLifecycleOperationError::StorageUnavailable {
                failure: expected_storage.clone(),
            }),
        )
        .expect_err("storage failure must not complete the shutdown target");
        assert_eq!(storage, expected_storage);

        let internal = classify_session_lifecycle_shutdown_result(
            TEST_EFFECT_IDENTITY,
            Err(SessionLifecycleOperationError::Internal {
                correlation_id: "lifecycle-internal".to_string(),
            }),
        )
        .expect_err("internal failure must not complete the shutdown target");
        assert_eq!(internal.kind, SessionOperationFailureKind::Internal);
        assert!(!internal.retryable);
        assert_eq!(internal.correlation_id, "lifecycle-internal");
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
