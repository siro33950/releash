use std::path::PathBuf;
use std::sync::Arc;

use crate::domain::app_config::ConfigRepository;
#[cfg(test)]
use crate::domain::workflow::WorkflowRuntimeSnapshot;
use crate::domain::workflow::{WorkflowDefinition, WorkflowError};
use crate::usecase::repository_usecase::RepositoryUsecase;
use crate::usecase::workflow::command::{
    AbortExecutionCommand, ResolvedStartExecutionCommand, ResumeExecutionCommand,
    StopExecutionCommand,
};
use crate::usecase::workflow::control_plane::{
    WorkflowControlPlaneCommit, WorkflowControlPlaneGateway,
};
use crate::usecase::workflow::ports::{
    WorkflowAbortExecutionGateway, WorkflowResumeExecutionGateway, WorkflowRuntimeShutdownGateway,
    WorkflowRuntimeStateGateway, WorkflowShutdownEffectReadback, WorkflowStartExecutionGateway,
    WorkflowStopExecutionGateway,
};

use super::runtime_resolver::{
    AppConfigManagedWorktreeResolver, DefaultWorkflowDefinitionResolver,
};
use crate::adaptor::gateway::workflow::workflow_host::WorkflowRuntimeHost;
use crate::usecase::workflow::runtime_error::WorkflowRuntimeError;

#[derive(Clone)]
pub(crate) struct TauriWorkflowRuntimeCommandGateway<R: tauri::Runtime = tauri::Wry> {
    app: tauri::AppHandle<R>,
    driver: Arc<WorkflowRuntimeHost>,
    local_event_repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository>,
    local_event_installation_id: String,
}

pub(crate) struct TauriWorkflowRuntimeCommandGatewayDeps {
    pub(crate) repository_usecase: Arc<RepositoryUsecase>,
    pub(crate) app_config: Arc<dyn ConfigRepository>,
    pub(crate) data_dir: Option<PathBuf>,
    pub(crate) local_event_repository:
        Arc<dyn crate::domain::local_event::LocalEventTransactionRepository>,
    pub(crate) local_event_installation_id: String,
    pub(crate) agent_session_launch: Arc<crate::usecase::agent_session::AgentSessionLaunchUsecase>,
    pub(crate) agent_session_initial_instruction:
        Arc<crate::usecase::agent_session::AgentSessionInitialInstructionUsecase>,
    pub(crate) agent_session_interrupt:
        Arc<crate::usecase::agent_session::AgentSessionInterruptUsecase>,
    pub(crate) provider_availability:
        Arc<dyn crate::domain::agent_session::ProviderAvailabilityReader>,
}

struct WorkflowShutdownRecord<'a> {
    operation_id: &'a str,
    effect_identity: &'a str,
    owner_revision: i64,
    execution_id: &'a str,
    state: crate::domain::local_event::ObligationStateRecord,
    expected: crate::domain::local_event::RevisionGuard,
    revision: crate::domain::local_event::Revision,
}

impl<R: tauri::Runtime> TauriWorkflowRuntimeCommandGateway<R> {
    fn shutdown_obligation_id(effect_identity: &str) -> String {
        use sha2::Digest;
        let digest = sha2::Sha256::digest(effect_identity.as_bytes());
        format!("workflow-shutdown-{}", &hex::encode(digest)[..32])
    }

    async fn shutdown_effect_record(
        &self,
        effect_identity: &str,
    ) -> Result<Option<crate::domain::local_event::ObligationView>, ()> {
        let result = self
            .local_event_repository
            .query(
                crate::domain::local_event::LocalEventQuery::ObligationByIdentity {
                    obligation_id: Self::shutdown_obligation_id(effect_identity),
                },
            )
            .await
            .map_err(|_| ())?;
        match result {
            crate::domain::local_event::LocalEventQueryResult::ObligationByIdentity(value) => {
                Ok(value)
            }
            _ => Err(()),
        }
    }

    fn shutdown_record_for_readback(
        lookup: Result<Option<crate::domain::local_event::ObligationView>, ()>,
    ) -> Result<crate::domain::local_event::ObligationView, WorkflowShutdownEffectReadback> {
        match lookup {
            Ok(Some(record)) => Ok(record),
            Ok(None) => Err(WorkflowShutdownEffectReadback::ConfirmedNotStarted),
            Err(()) => Err(WorkflowShutdownEffectReadback::Ambiguous),
        }
    }

    fn workflow_shutdown_record_matches(
        record: &crate::domain::local_event::ObligationView,
        operation_id: &str,
        effect_identity: &str,
        owner_revision: i64,
        execution_id: &str,
    ) -> Option<crate::domain::local_event::ObligationStateRecord> {
        let crate::domain::local_event::ObligationRecord::WorkflowShutdown {
            operation_id: stored_operation_id,
            effect_identity: stored_effect_identity,
            owner_revision: stored_owner_revision,
            execution_id: stored_execution_id,
            state,
        } = &record.record
        else {
            return None;
        };
        (stored_operation_id == operation_id
            && stored_effect_identity == effect_identity
            && *stored_owner_revision == owner_revision
            && stored_execution_id == execution_id)
            .then_some(*state)
    }

    async fn commit_workflow_shutdown_record(&self, record: WorkflowShutdownRecord<'_>) -> bool {
        use sha2::Digest;
        let WorkflowShutdownRecord {
            operation_id,
            effect_identity,
            owner_revision,
            execution_id,
            state,
            expected,
            revision,
        } = record;
        let repository = &self.local_event_repository;
        let installation_id = &self.local_event_installation_id;
        let obligation_id = Self::shutdown_obligation_id(effect_identity);
        let record = crate::domain::local_event::ObligationRecord::WorkflowShutdown {
            operation_id: operation_id.to_string(),
            effect_identity: effect_identity.to_string(),
            owner_revision,
            execution_id: execution_id.to_string(),
            state,
        };
        let digest = sha2::Sha256::digest(
            format!("workflow-shutdown\0{effect_identity}\0{}", revision.value()).as_bytes(),
        );
        let Ok(commit_id) = crate::domain::local_event::CommitIdentity::parse(&hex::encode(digest))
        else {
            return false;
        };
        let pending =
            (state != crate::domain::local_event::ObligationStateRecord::Completed).then(|| {
                crate::domain::local_event::PendingIndexEntry {
                    ordered_key: format!("workflow-shutdown-{effect_identity}"),
                    owner: execution_id.to_string(),
                    partition: crate::domain::local_event::PendingPartition::Owner,
                    shutdown_plan: None,
                }
            });
        let state_code = match state {
            crate::domain::local_event::ObligationStateRecord::EffectReserved => "effect_reserved",
            crate::domain::local_event::ObligationStateRecord::Completed => "completed",
            _ => return false,
        };
        let payload_hash: [u8; 32] = sha2::Sha256::digest(
			format!(
				"workflow-shutdown-record/v1\0{operation_id}\0{effect_identity}\0{owner_revision}\0{execution_id}\0{state_code}"
			)
			.as_bytes(),
		)
		.into();
        let batch = crate::domain::local_event::LocalAtomicBatch {
            commit_id,
            idempotency: crate::domain::local_event::IdempotencyBinding {
                installation_id: installation_id.clone(),
                operation_kind: crate::domain::local_event::OperationKind::ApplicationQuit.into(),
                idempotency_key: format!("{obligation_id}.{}", revision.value()),
                payload_hash,
            },
            expected_heads: Vec::new(),
            events: Vec::new(),
            state_mutations: vec![crate::domain::local_event::LocalStateMutation::Obligation(
                crate::domain::local_event::ObligationMutation {
                    obligation_id,
                    record,
                    pending,
                    expected,
                    revision,
                },
            )],
        };
        repository.commit_batch(batch).await.is_ok()
    }

    pub(crate) fn new_with_default_driver(
        app: tauri::AppHandle<R>,
        deps: TauriWorkflowRuntimeCommandGatewayDeps,
    ) -> Result<Self, WorkflowRuntimeError> {
        let TauriWorkflowRuntimeCommandGatewayDeps {
            repository_usecase,
            app_config,
            data_dir,
            local_event_repository,
            local_event_installation_id,
            agent_session_launch,
            agent_session_initial_instruction,
            agent_session_interrupt,
            provider_availability,
        } = deps;
        let driver = Arc::new(WorkflowRuntimeHost::new_canonical(
            Arc::new(DefaultWorkflowDefinitionResolver),
            Arc::new(AppConfigManagedWorktreeResolver::new(
                repository_usecase,
                app_config,
            )),
            data_dir,
            local_event_repository.clone(),
            local_event_installation_id.clone(),
            agent_session_launch,
            agent_session_initial_instruction,
            agent_session_interrupt,
            provider_availability,
        ));
        Ok(Self {
            app,
            driver,
            local_event_repository,
            local_event_installation_id,
        })
    }

    #[cfg(debug_assertions)]
    pub(crate) fn new_with_driver(
        app: tauri::AppHandle<R>,
        driver: Arc<WorkflowRuntimeHost>,
        local_event_repository: Arc<
            dyn crate::domain::local_event::LocalEventTransactionRepository,
        >,
        local_event_installation_id: String,
    ) -> Self {
        Self {
            app,
            driver,
            local_event_repository,
            local_event_installation_id,
        }
    }
}

#[async_trait::async_trait]
impl<R: tauri::Runtime> WorkflowStartExecutionGateway for TauriWorkflowRuntimeCommandGateway<R> {
    async fn resolve_start_execution_worktree(
        &self,
        worktree_path: String,
    ) -> Result<String, WorkflowError> {
        self.driver
            .resolve_start_execution_worktree(worktree_path)
            .await
            .map_err(workflow_runtime_error_to_workflow_error)
    }

    async fn resolve_start_execution_workflow(
        &self,
        workflow_name: &str,
    ) -> Result<WorkflowDefinition, WorkflowError> {
        let workflow = self
            .driver
            .resolve_start_execution_workflow(workflow_name)
            .await
            .map_err(workflow_runtime_error_to_workflow_error)?;
        super::mapper::schema_workflow_to_domain(workflow)
    }

    async fn start_resolved_execution(
        &self,
        command: ResolvedStartExecutionCommand,
    ) -> Result<String, WorkflowError> {
        let workflow = super::mapper::domain_workflow_to_schema(&command.workflow)?;
        self.driver
            .start_resolved_workflow(
                &self.app,
                workflow,
                command.worktree_path,
                command.request,
                command.created_from,
            )
            .await
            .map_err(workflow_runtime_error_to_workflow_error)
    }
}

fn workflow_runtime_error_to_workflow_error(error: WorkflowRuntimeError) -> WorkflowError {
    match error {
        WorkflowRuntimeError::InvalidWorkflow(message)
        | WorkflowRuntimeError::ValidationError(message) => WorkflowError::validation(message),
        error @ WorkflowRuntimeError::ExecutionNotFound(_)
        | error @ WorkflowRuntimeError::SessionNotFound(_) => {
            WorkflowError::NotFound(error.to_string())
        }
        error @ WorkflowRuntimeError::AlreadyActive(_) => {
            WorkflowError::InvalidState(error.to_string())
        }
        WorkflowRuntimeError::InvalidState(message) => WorkflowError::InvalidState(message),
        WorkflowRuntimeError::Conflict(message) => WorkflowError::Conflict(message),
        WorkflowRuntimeError::UnauthorizedWorktree(message) => WorkflowError::validation(message),
        WorkflowRuntimeError::UnauthorizedApprovalTarget(message) => {
            WorkflowError::UnauthorizedApprovalTarget(message)
        }
        WorkflowRuntimeError::SessionStore(message)
        | WorkflowRuntimeError::AgentSession(message) => WorkflowError::external(message),
    }
}

#[async_trait::async_trait]
impl<R: tauri::Runtime> WorkflowAbortExecutionGateway for TauriWorkflowRuntimeCommandGateway<R> {
    async fn abort_execution(&self, command: AbortExecutionCommand) -> Result<(), WorkflowError> {
        self.driver
            .abort_workflow_execution(
                &self.app,
                &command.execution_id,
                command.expected_node_name.as_deref(),
            )
            .await
            .map_err(workflow_runtime_error_to_workflow_error)
    }
}

#[async_trait::async_trait]
impl<R: tauri::Runtime> WorkflowStopExecutionGateway for TauriWorkflowRuntimeCommandGateway<R> {
    async fn stop_execution(&self, command: StopExecutionCommand) -> Result<(), WorkflowError> {
        self.driver
            .stop_workflow_execution(&self.app, &command.execution_id)
            .await
            .map_err(workflow_runtime_error_to_workflow_error)
    }
}

#[async_trait::async_trait]
impl<R: tauri::Runtime> WorkflowResumeExecutionGateway for TauriWorkflowRuntimeCommandGateway<R> {
    async fn resume_execution(&self, command: ResumeExecutionCommand) -> Result<(), WorkflowError> {
        self.driver
            .resume_workflow_execution(&self.app, &command.execution_id)
            .await
            .map_err(workflow_runtime_error_to_workflow_error)
    }
}

#[async_trait::async_trait]
impl<R: tauri::Runtime> WorkflowControlPlaneGateway for TauriWorkflowRuntimeCommandGateway<R> {
    fn current_timestamp(&self) -> f64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0.0, |duration| duration.as_secs_f64())
    }

    fn new_node_execution_id(&self) -> String {
        uuid::Uuid::new_v4().to_string()
    }

    async fn load_active_execution(
        &self,
        execution_id: &str,
    ) -> Result<
        Option<crate::domain::workflow::entities::workflow_execution::WorkflowExecution>,
        WorkflowError,
    > {
        Ok(self.driver.load_control_plane_execution(execution_id).await)
    }

    async fn recover_active_executions(&self) -> Result<(), WorkflowError> {
        self.driver
            .recover_orphan_executions(&self.app)
            .await
            .map_err(workflow_runtime_error_to_workflow_error)
    }

    async fn load_persisted_events(
        &self,
        execution_id: &str,
    ) -> Result<Vec<crate::domain::workflow::WorkflowEvent>, WorkflowError> {
        super::event_log_writer::read_events_for_app(&self.app, execution_id)
            .map_err(WorkflowError::external)
    }

    fn configured_secret_values(&self) -> Vec<String> {
        super::secret_source::collect_configured_secret_values(&self.app)
    }

    fn approval_auto_approve_enabled(&self) -> bool {
        super::workflow_host::approval_runtime::workflow_approval_auto_approve_enabled(&self.app)
    }

    async fn commit_control_plane(
        &self,
        commit: WorkflowControlPlaneCommit,
    ) -> Result<crate::usecase::workflow::runtime_snapshot::RuntimeCommitSnapshot, WorkflowError>
    {
        self.driver
            .commit_workflow_control_plane(&self.app, commit)
            .await
            .map_err(workflow_runtime_error_to_workflow_error)
    }

    async fn finish_control_plane_commit(
        &self,
        worktree_path: &str,
        snapshot: &crate::usecase::workflow::runtime_snapshot::RuntimeCommitSnapshot,
        outcome: Option<crate::usecase::workflow::runtime_driver::NodeOutcome>,
    ) -> Result<(), WorkflowError> {
        self.driver
            .finish_workflow_control_plane_commit(&self.app, worktree_path, snapshot, outcome)
            .await
            .map_err(workflow_runtime_error_to_workflow_error)
    }

    async fn finish_retried_fanout_commit(
        &self,
        worktree_path: &str,
        snapshot: &crate::usecase::workflow::runtime_snapshot::RuntimeCommitSnapshot,
        node_execution_id: &str,
    ) -> Result<(), WorkflowError> {
        self.driver
            .finish_retried_fanout_control_plane_commit(
                &self.app,
                worktree_path,
                snapshot,
                node_execution_id,
            )
            .await
            .map_err(workflow_runtime_error_to_workflow_error)
    }
}

#[async_trait::async_trait]
impl<R: tauri::Runtime> WorkflowRuntimeStateGateway for TauriWorkflowRuntimeCommandGateway<R> {
    async fn recover_startup(&self) -> Result<(), WorkflowError> {
        self.driver
            .recover_orphan_executions(&self.app)
            .await
            .map_err(workflow_runtime_error_to_workflow_error)
    }

    #[cfg(test)]
    async fn get_state_by_execution_id(
        &self,
        execution_id: &str,
    ) -> Result<Option<WorkflowRuntimeSnapshot>, WorkflowError> {
        Ok(self
            .driver
            .get_state_by_execution_id(execution_id)
            .await
            .map(
            crate::adaptor::gateway::workflow::state::runtime_commit_snapshot_to_domain_snapshot,
        ))
    }
}

#[async_trait::async_trait]
impl<R: tauri::Runtime> WorkflowRuntimeShutdownGateway for TauriWorkflowRuntimeCommandGateway<R> {
    async fn shutdown_active_commands(&self) {
        self.driver.shutdown_all_active_commands().await;
    }

    async fn shutdown_execution_commands(&self, execution_id: &str) {
        self.driver
            .shutdown_active_commands_for_execution(execution_id)
            .await;
    }

    async fn application_shutdown_target_execution_ids(&self) -> Result<Vec<String>, String> {
        self.driver
            .application_shutdown_target_execution_ids()
            .await
    }

    async fn execute_shutdown_effect(
        &self,
        operation_id: &str,
        effect_identity: &str,
        owner_revision: i64,
        execution_id: &str,
    ) -> WorkflowShutdownEffectReadback {
        match self.shutdown_effect_record(effect_identity).await {
            Ok(Some(record)) => {
                return match Self::workflow_shutdown_record_matches(
                    &record,
                    operation_id,
                    effect_identity,
                    owner_revision,
                    execution_id,
                ) {
                    Some(crate::domain::local_event::ObligationStateRecord::Completed) => {
                        WorkflowShutdownEffectReadback::Completed
                    }
                    Some(crate::domain::local_event::ObligationStateRecord::EffectReserved) => {
                        WorkflowShutdownEffectReadback::Ambiguous
                    }
                    _ => WorkflowShutdownEffectReadback::Ambiguous,
                };
            }
            Ok(None) => {}
            Err(()) => return WorkflowShutdownEffectReadback::Ambiguous,
        }
        let zero = crate::domain::local_event::Revision::new(0).expect("zero revision");
        if !self
            .commit_workflow_shutdown_record(WorkflowShutdownRecord {
                operation_id,
                effect_identity,
                owner_revision,
                execution_id,
                state: crate::domain::local_event::ObligationStateRecord::EffectReserved,
                expected: crate::domain::local_event::RevisionGuard::Absent,
                revision: zero,
            })
            .await
        {
            return WorkflowShutdownEffectReadback::Ambiguous;
        }
        let observed_owned_command = self
            .driver
            .shutdown_active_commands_for_execution(execution_id)
            .await;
        if !observed_owned_command {
            // The durable reservation proves the effect may have started, but
            // absence of an owned command cannot prove this effect completed;
            // an unrelated terminal transition must not be adopted as its
            // result.
            return WorkflowShutdownEffectReadback::Ambiguous;
        }
        let one = crate::domain::local_event::Revision::new(1).expect("one revision");
        if self
            .commit_workflow_shutdown_record(WorkflowShutdownRecord {
                operation_id,
                effect_identity,
                owner_revision,
                execution_id,
                state: crate::domain::local_event::ObligationStateRecord::Completed,
                expected: crate::domain::local_event::RevisionGuard::Expected(zero),
                revision: one,
            })
            .await
        {
            WorkflowShutdownEffectReadback::Completed
        } else {
            WorkflowShutdownEffectReadback::Ambiguous
        }
    }

    async fn read_shutdown_effect(
        &self,
        operation_id: &str,
        effect_identity: &str,
        owner_revision: i64,
        execution_id: &str,
    ) -> WorkflowShutdownEffectReadback {
        let record = match Self::shutdown_record_for_readback(
            self.shutdown_effect_record(effect_identity).await,
        ) {
            Ok(record) => record,
            Err(readback) => return readback,
        };
        match Self::workflow_shutdown_record_matches(
            &record,
            operation_id,
            effect_identity,
            owner_revision,
            execution_id,
        ) {
            Some(crate::domain::local_event::ObligationStateRecord::Completed) => {
                WorkflowShutdownEffectReadback::Completed
            }
            Some(crate::domain::local_event::ObligationStateRecord::EffectReserved) => {
                WorkflowShutdownEffectReadback::Ambiguous
            }
            _ => WorkflowShutdownEffectReadback::Ambiguous,
        }
    }
}

#[cfg(test)]
mod shutdown_effect_contract_tests {
    use super::TauriWorkflowRuntimeCommandGateway;

    #[test]
    fn workflow_shutdown_read_failure_is_ambiguous_not_confirmed_not_started() {
        use crate::usecase::workflow::ports::WorkflowShutdownEffectReadback;

        assert_eq!(
            TauriWorkflowRuntimeCommandGateway::<tauri::Wry>::shutdown_record_for_readback(Err(())),
            Err(WorkflowShutdownEffectReadback::Ambiguous)
        );
        assert_eq!(
            TauriWorkflowRuntimeCommandGateway::<tauri::Wry>::shutdown_record_for_readback(Ok(
                None
            )),
            Err(WorkflowShutdownEffectReadback::ConfirmedNotStarted)
        );
    }

    #[test]
    fn workflow_shutdown_readback_is_bound_to_exact_effect_and_owner_revision() {
        let record_value = crate::domain::local_event::ObligationRecord::WorkflowShutdown {
            operation_id: "quit-operation".to_string(),
            effect_identity: "quit-operation:0:workflow-1".to_string(),
            owner_revision: 7,
            execution_id: "workflow-1".to_string(),
            state: crate::domain::local_event::ObligationStateRecord::Completed,
        };
        let record = crate::domain::local_event::ObligationView {
            obligation_id: "workflow-shutdown-record".to_string(),
            record: record_value,
            record_sha256: [0; 32],
            pending: None,
            revision: crate::domain::local_event::Revision::new(1).unwrap(),
        };
        assert_eq!(
            TauriWorkflowRuntimeCommandGateway::<tauri::Wry>::workflow_shutdown_record_matches(
                &record,
                "quit-operation",
                "quit-operation:0:workflow-1",
                7,
                "workflow-1",
            ),
            Some(crate::domain::local_event::ObligationStateRecord::Completed)
        );
        assert!(
            TauriWorkflowRuntimeCommandGateway::<tauri::Wry>::workflow_shutdown_record_matches(
                &record,
                "unrelated-terminal-operation",
                "quit-operation:0:workflow-1",
                7,
                "workflow-1",
            )
            .is_none()
        );
        assert!(
            TauriWorkflowRuntimeCommandGateway::<tauri::Wry>::workflow_shutdown_record_matches(
                &record,
                "quit-operation",
                "quit-operation:0:workflow-1",
                8,
                "workflow-1",
            )
            .is_none()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_name_resolution_diagnostics_remain_validation_errors() {
        let error =
            workflow_runtime_error_to_workflow_error(WorkflowRuntimeError::InvalidWorkflow(
                "workflow_diagnostics: WFS006: duplicate workflow name".to_string(),
            ));

        assert!(matches!(
            error,
            WorkflowError::Validation(message)
                if message.contains("WFS006") && message.contains("duplicate workflow name")
        ));
    }

    #[test]
    fn runtime_command_error_mapping_preserves_domain_variants() {
        assert!(matches!(
            workflow_runtime_error_to_workflow_error(WorkflowRuntimeError::ExecutionNotFound(
                "missing".to_string()
            )),
            WorkflowError::NotFound(message)
                if message == "No workflow execution found for session 'missing'"
        ));
        assert!(matches!(
            workflow_runtime_error_to_workflow_error(WorkflowRuntimeError::InvalidState(
                "terminal".to_string()
            )),
            WorkflowError::InvalidState(message) if message == "terminal"
        ));
        assert!(matches!(
            workflow_runtime_error_to_workflow_error(
                WorkflowRuntimeError::UnauthorizedApprovalTarget("wrong target".to_string())
            ),
            WorkflowError::UnauthorizedApprovalTarget(message) if message == "wrong target"
        ));
        assert!(matches!(
            workflow_runtime_error_to_workflow_error(WorkflowRuntimeError::ValidationError(
                "bad output".to_string()
            )),
            WorkflowError::Validation(message) if message == "bad output"
        ));
        assert!(matches!(
            workflow_runtime_error_to_workflow_error(WorkflowRuntimeError::SessionStore(
                "io".to_string()
            )),
            WorkflowError::External(message) if message == "io"
        ));
    }
}
