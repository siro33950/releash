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

    async fn resolve_workflow_execution_id(
        &self,
        node_execution_id: &str,
    ) -> Result<Option<String>, WorkflowError> {
        use tauri::Manager as _;
        let store = self
            .app
            .try_state::<std::sync::Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>>()
            .map(|store| store.inner().clone())
            .ok_or_else(|| {
                WorkflowError::external("workflow SQLite event authority is not managed")
            })?;
        super::fact_log::FactLogReadBackend::Live(store)
            .tree_id_for_node(node_execution_id)
            .map_err(WorkflowError::external)
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
            .reconcile_startup(&self.app)
            .await
            .map_err(workflow_runtime_error_to_workflow_error)
    }

    async fn approval_persisted(
        &self,
        execution_id: &str,
        node_name: &str,
        node_execution_id: Option<&str>,
    ) -> Result<bool, WorkflowError> {
        use tauri::Manager as _;
        let store = self
            .app
            .try_state::<std::sync::Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>>()
            .map(|store| store.inner().clone())
            .ok_or_else(|| {
                WorkflowError::external("workflow SQLite event authority is not managed")
            })?;
        let records = super::fact_log::read_tree_records(&store, execution_id)
            .map_err(|error| WorkflowError::external(error.to_string()))?;
        Ok(records.iter().any(|record| {
            matches!(
                record.fact,
                crate::domain::workflow::NodeFact::ApprovalGranted(_)
            ) && record.meta.node_name == node_name
                && node_execution_id
                    .is_none_or(|expected| expected == record.meta.node_execution_id)
        }))
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
}

#[async_trait::async_trait]
impl<R: tauri::Runtime> WorkflowRuntimeStateGateway for TauriWorkflowRuntimeCommandGateway<R> {
    async fn recover_startup(&self) -> Result<(), WorkflowError> {
        self.driver
            .reconcile_startup(&self.app)
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
        use crate::domain::local_event::workflow_shutdown::{
            self, WorkflowShutdownReservationStep,
        };
        let Ok(record) = self.shutdown_effect_record(effect_identity).await else {
            return WorkflowShutdownEffectReadback::Ambiguous;
        };
        let reservation = match workflow_shutdown::reservation_step(
            record.as_ref(),
            operation_id,
            effect_identity,
            execution_id,
            owner_revision,
        ) {
            WorkflowShutdownReservationStep::AlreadyCompleted => {
                return WorkflowShutdownEffectReadback::Completed
            }
            WorkflowShutdownReservationStep::ContinueOwn { reservation } => reservation,
            WorkflowShutdownReservationStep::Reserve {
                expected,
                reservation,
            } => {
                if !self
                    .commit_workflow_shutdown_record(WorkflowShutdownRecord {
                        operation_id,
                        effect_identity,
                        owner_revision,
                        execution_id,
                        state: crate::domain::local_event::ObligationStateRecord::EffectReserved,
                        expected,
                        revision: reservation,
                    })
                    .await
                {
                    return WorkflowShutdownEffectReadback::Ambiguous;
                }
                reservation
            }
            WorkflowShutdownReservationStep::Reject => {
                return WorkflowShutdownEffectReadback::Ambiguous
            }
        };
        // An execution with no owned command has nothing left to quiesce in
        // this process, so the effect is satisfied whether or not a command
        // was observed. Commands a dead process left behind are crash
        // recovery's responsibility, not this effect's.
        self.driver
            .shutdown_active_commands_for_execution(execution_id)
            .await;
        let Some(completed) = reservation.next() else {
            return WorkflowShutdownEffectReadback::Ambiguous;
        };
        if self
            .commit_workflow_shutdown_record(WorkflowShutdownRecord {
                operation_id,
                effect_identity,
                owner_revision,
                execution_id,
                state: crate::domain::local_event::ObligationStateRecord::Completed,
                expected: crate::domain::local_event::RevisionGuard::Expected(reservation),
                revision: completed,
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
        _owner_revision: i64,
        execution_id: &str,
    ) -> WorkflowShutdownEffectReadback {
        use crate::domain::local_event::workflow_shutdown::{
            self, WorkflowShutdownEffectResolution,
        };
        let Ok(record) = self.shutdown_effect_record(effect_identity).await else {
            return WorkflowShutdownEffectReadback::Ambiguous;
        };
        match workflow_shutdown::read_resolution(
            record.as_ref(),
            operation_id,
            effect_identity,
            execution_id,
        ) {
            WorkflowShutdownEffectResolution::Completed => {
                WorkflowShutdownEffectReadback::Completed
            }
            WorkflowShutdownEffectResolution::NotStarted => {
                WorkflowShutdownEffectReadback::ConfirmedNotStarted
            }
            WorkflowShutdownEffectResolution::Unresolved => {
                WorkflowShutdownEffectReadback::Ambiguous
            }
        }
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
