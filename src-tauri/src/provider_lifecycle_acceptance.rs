use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use crate::adaptor::gateway::local_event_store::provider_lifecycle_codec::PROVIDER_LIFECYCLE_EVENT_TYPE;
use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
use crate::adaptor::gateway::provider_lifecycle::{
    LocalProviderLifecycleCredentialGateway, LocalProviderLifecycleEventRepository,
    ProviderLaunchContext, ProviderLaunchSpec,
};
use crate::adaptor::protocol::provider_lifecycle::{
    ProviderLifecycleProvider, ProviderLifecycleReceiveResponse,
    ProviderLifecycleUnavailableReasonRequest, ProviderLifecycleUnavailableRequest,
};
use crate::domain::local_event::{
    LoadStreamRequest, LoadedDomainEvent, LocalDomainEvent, LocalEventTransactionRepository,
    StreamId,
};
use crate::domain::provider_lifecycle::{
    ProviderKind, ProviderLifecycleEvent, ProviderLifecycleScope, ProviderLifecycleSlotId,
    ProviderLifecycleUnavailableReason,
};
use crate::domain::workflow::{WorkflowDefinition, WorkflowError};
use crate::infrastructure::local_api::{LocalApiHttpClient, LocalApiServer, LocalApiServerBinding};
use crate::usecase::provider_lifecycle::ProviderLifecycleUsecase;
use crate::usecase::workflow::command::{
    AbortExecutionCommand, ResolvedStartExecutionCommand, ResumeExecutionCommand,
    StopExecutionCommand,
};
use crate::usecase::workflow::control_plane::{
    WorkflowControlPlaneCommit, WorkflowControlPlaneGateway,
};
use crate::usecase::workflow::ports::{
    WorkflowAbortExecutionGateway, WorkflowResumeExecutionGateway, WorkflowRuntimeShutdownGateway,
    WorkflowRuntimeStateGateway, WorkflowStartExecutionGateway, WorkflowStopExecutionGateway,
};
use crate::usecase::workflow::WorkflowRuntimeUsecase;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcceptanceProvider {
    Claude,
    Codex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcceptanceUnavailableReason {
    SessionStartDeadlineExceeded,
    CodexHookDeliveryUnconfirmed,
    ProviderHookConfigurationRejected,
    LocalApiUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcceptanceIngressResult {
    Applied,
    Duplicate,
    Rejected { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptanceScope {
    pub agent_session_id: String,
}

impl AcceptanceScope {
    pub fn new(agent_session_id: impl Into<String>) -> Self {
        Self {
            agent_session_id: agent_session_id.into(),
        }
    }

    fn domain(&self) -> Result<ProviderLifecycleScope, String> {
        ProviderLifecycleScope::new(&self.agent_session_id).map_err(|error| error.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptanceLaunchFile {
    pub relative_path: PathBuf,
    pub contents: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptanceLaunch {
    pub slot_id: String,
    pub binding_id: String,
    pub capability: String,
    pub provider: AcceptanceProvider,
    pub scope: AcceptanceScope,
    pub arguments: Vec<String>,
    pub environment: Vec<(String, String)>,
    pub files: Vec<AcceptanceLaunchFile>,
    pub requires_hook_trust: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcceptanceFactKind {
    BindingArmed {
        binding_id: String,
        provider: AcceptanceProvider,
        scope: AcceptanceScope,
    },
    SessionAssociated {
        binding_id: String,
        provider_session_id: String,
        transcript_ref: Option<String>,
    },
    TranscriptAssociated {
        binding_id: String,
        transcript_ref: String,
    },
    StopObserved {
        binding_id: String,
    },
    StopFailed {
        binding_id: String,
        reason: String,
    },
    LifecycleUnavailable {
        binding_id: String,
        provider: AcceptanceProvider,
        scope: AcceptanceScope,
        reason: String,
    },
    BindingExpired {
        binding_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptanceFact {
    pub occurred_at_ms: i64,
    pub kind: AcceptanceFactKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AcceptanceEventCounts {
    pub provider_lifecycle: usize,
    pub other: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AcceptanceLedgerEventCounts {
    pub provider_lifecycle: usize,
    pub other: usize,
}

pub struct ProviderLifecycleAcceptanceHost {
    data_dir: PathBuf,
    store: Arc<LocalEventStore>,
    usecase: Arc<ProviderLifecycleUsecase>,
    server: Arc<LocalApiServer>,
    workflow_runtime_command_count: Arc<AtomicUsize>,
}

struct AcceptanceWorkflowRuntimeGateway {
    command_count: Arc<AtomicUsize>,
}

impl AcceptanceWorkflowRuntimeGateway {
    fn record_command(&self) {
        self.command_count.fetch_add(1, Ordering::SeqCst);
    }
}

fn unavailable_workflow_runtime() -> WorkflowError {
    WorkflowError::external("workflow runtime is not available in Provider lifecycle acceptance")
}

#[async_trait::async_trait]
impl WorkflowStartExecutionGateway for AcceptanceWorkflowRuntimeGateway {
    async fn resolve_start_execution_worktree(
        &self,
        _worktree_path: String,
    ) -> Result<String, WorkflowError> {
        Err(unavailable_workflow_runtime())
    }

    async fn resolve_start_execution_workflow(
        &self,
        _workflow_name: &str,
    ) -> Result<WorkflowDefinition, WorkflowError> {
        Err(unavailable_workflow_runtime())
    }

    async fn start_resolved_execution(
        &self,
        _command: ResolvedStartExecutionCommand,
    ) -> Result<String, WorkflowError> {
        self.record_command();
        Err(unavailable_workflow_runtime())
    }
}

#[async_trait::async_trait]
impl WorkflowAbortExecutionGateway for AcceptanceWorkflowRuntimeGateway {
    async fn abort_execution(&self, _command: AbortExecutionCommand) -> Result<(), WorkflowError> {
        self.record_command();
        Err(unavailable_workflow_runtime())
    }
}

#[async_trait::async_trait]
impl WorkflowStopExecutionGateway for AcceptanceWorkflowRuntimeGateway {
    async fn stop_execution(&self, _command: StopExecutionCommand) -> Result<(), WorkflowError> {
        self.record_command();
        Err(unavailable_workflow_runtime())
    }
}

#[async_trait::async_trait]
impl WorkflowResumeExecutionGateway for AcceptanceWorkflowRuntimeGateway {
    async fn resume_execution(
        &self,
        _command: ResumeExecutionCommand,
    ) -> Result<(), WorkflowError> {
        self.record_command();
        Err(unavailable_workflow_runtime())
    }
}

#[async_trait::async_trait]
impl WorkflowControlPlaneGateway for AcceptanceWorkflowRuntimeGateway {
    fn current_timestamp(&self) -> f64 {
        100.0
    }

    fn new_node_execution_id(&self) -> String {
        "node-execution-test".to_string()
    }

    async fn resolve_workflow_execution_id(
        &self,
        _node_execution_id: &str,
    ) -> Result<Option<String>, WorkflowError> {
        self.record_command();
        Err(unavailable_workflow_runtime())
    }

    async fn load_active_execution(
        &self,
        _execution_id: &str,
    ) -> Result<
        Option<crate::domain::workflow::entities::workflow_execution::WorkflowExecution>,
        WorkflowError,
    > {
        self.record_command();
        Err(unavailable_workflow_runtime())
    }

    async fn recover_active_executions(&self) -> Result<(), WorkflowError> {
        self.record_command();
        Err(unavailable_workflow_runtime())
    }

    async fn approval_persisted(
        &self,
        _execution_id: &str,
        _node_name: &str,
        _node_execution_id: Option<&str>,
    ) -> Result<bool, WorkflowError> {
        self.record_command();
        Err(unavailable_workflow_runtime())
    }

    fn configured_secret_values(&self) -> Vec<String> {
        Vec::new()
    }

    fn approval_auto_approve_enabled(&self) -> bool {
        false
    }

    async fn commit_control_plane(
        &self,
        _commit: WorkflowControlPlaneCommit,
    ) -> Result<crate::usecase::workflow::runtime_snapshot::RuntimeCommitSnapshot, WorkflowError>
    {
        self.record_command();
        Err(unavailable_workflow_runtime())
    }

    async fn finish_control_plane_commit(
        &self,
        _worktree_path: &str,
        _snapshot: &crate::usecase::workflow::runtime_snapshot::RuntimeCommitSnapshot,
        _outcome: Option<crate::usecase::workflow::runtime_driver::NodeOutcome>,
    ) -> Result<(), WorkflowError> {
        Ok(())
    }
}

#[async_trait::async_trait]
impl WorkflowRuntimeStateGateway for AcceptanceWorkflowRuntimeGateway {
    async fn recover_startup(&self) -> Result<(), WorkflowError> {
        Ok(())
    }

    #[cfg(test)]
    async fn get_state_by_execution_id(
        &self,
        _execution_id: &str,
    ) -> Result<Option<crate::domain::workflow::WorkflowRuntimeSnapshot>, WorkflowError> {
        Ok(None)
    }
}

#[async_trait::async_trait]
impl WorkflowRuntimeShutdownGateway for AcceptanceWorkflowRuntimeGateway {
    async fn shutdown_active_commands(&self) {}

    async fn application_shutdown_target_execution_ids(&self) -> Result<Vec<String>, String> {
        Ok(Vec::new())
    }
}

impl ProviderLifecycleAcceptanceHost {
    pub fn start(data_dir: &Path) -> Result<Self, String> {
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(data_dir.to_path_buf()))
                .map_err(|error| error.to_string())?;
        let events = Arc::new(LocalProviderLifecycleEventRepository::new(
            store.clone() as Arc<dyn LocalEventTransactionRepository>,
            store.installation_id().to_string(),
        ));
        let usecase = Arc::new(ProviderLifecycleUsecase::new(
            Arc::new(LocalProviderLifecycleCredentialGateway),
            events,
        ));
        let binding = LocalApiServerBinding::bind(data_dir.to_path_buf())
            .map_err(|error| error.to_string())?;
        let workflow = crate::adaptor::controller::wiring::build_canonical_workflow_read_usecase(
            data_dir, None,
        )
        .map_err(|error| error.to_string())?;
        let workflow_runtime_command_count = Arc::new(AtomicUsize::new(0));
        let runtime = Arc::new(WorkflowRuntimeUsecase::new(Arc::new(
            AcceptanceWorkflowRuntimeGateway {
                command_count: workflow_runtime_command_count.clone(),
            },
        )));
        let router = crate::adaptor::controller::api::build_router(
            Arc::new(workflow),
            runtime,
            binding.bearer_token(),
            binding.terminal_bearer_token(),
            None,
            Some(usecase.clone()),
        );
        let server = binding.start(router, &tokio::runtime::Handle::current());
        Ok(Self {
            data_dir: data_dir.to_path_buf(),
            store,
            usecase,
            server,
            workflow_runtime_command_count,
        })
    }

    pub async fn prepare_launch(
        &self,
        provider: AcceptanceProvider,
        scope: AcceptanceScope,
        claude_plugin_directory: Option<&Path>,
    ) -> Result<AcceptanceLaunch, String> {
        self.prepare_launch_in_slot(
            uuid::Uuid::new_v4().simple().to_string(),
            provider,
            scope,
            claude_plugin_directory,
        )
        .await
    }

    pub async fn prepare_launch_in_slot(
        &self,
        slot_id: impl Into<String>,
        provider: AcceptanceProvider,
        scope: AcceptanceScope,
        claude_plugin_directory: Option<&Path>,
    ) -> Result<AcceptanceLaunch, String> {
        let domain_provider = domain_provider(provider);
        let slot_id =
            ProviderLifecycleSlotId::new(slot_id.into()).map_err(|error| error.to_string())?;
        let armed = self
            .usecase
            .arm(slot_id, domain_provider, scope.domain()?)
            .await
            .map_err(|error| error.to_string())?;
        let context = ProviderLaunchContext::new(
            armed.slot_id().clone(),
            armed.binding_id(),
            armed.capability(),
            armed.scope().clone(),
        )
        .map_err(|error| error.to_string())?;
        let hook_cli_alias = crate::infrastructure::platform::path_aliases::alias_name_for_profile(
            crate::infrastructure::platform::path_aliases::BuildProfile::current(),
        );
        let spec = ProviderLaunchSpec::for_provider(
            armed.provider(),
            context,
            hook_cli_alias,
            claude_plugin_directory,
        )
        .map_err(|error| error.to_string())?;
        Ok(AcceptanceLaunch {
            slot_id: armed.slot_id().as_str().to_string(),
            binding_id: armed.binding_id().to_string(),
            capability: armed.capability().to_string(),
            provider,
            scope,
            arguments: spec.arguments().to_vec(),
            environment: spec.environment().to_vec(),
            files: spec
                .files()
                .iter()
                .map(|file| AcceptanceLaunchFile {
                    relative_path: file.relative_path().to_path_buf(),
                    contents: file.contents().to_vec(),
                })
                .collect(),
            requires_hook_trust: spec.requires_hook_trust(),
        })
    }

    pub async fn facts(&self, agent_session_id: &str) -> Result<Vec<AcceptanceFact>, String> {
        let page = self
            .store
            .load_stream(LoadStreamRequest {
                stream_id: StreamId::provider_lifecycle(agent_session_id)
                    .map_err(|error| error.to_string())?,
                after: None,
                limit: 1_024,
            })
            .await
            .map_err(|error| error.to_string())?;
        Ok(page
            .events
            .into_iter()
            .filter_map(|event| {
                let LoadedDomainEvent::Known(inner) = event.event else {
                    return None;
                };
                let LocalDomainEvent::ProviderLifecycle(provider_event) = *inner else {
                    return None;
                };
                Some(AcceptanceFact {
                    occurred_at_ms: event.occurred_at_ms,
                    kind: acceptance_fact(provider_event),
                })
            })
            .collect())
    }

    pub async fn report_unavailable(
        &self,
        launch: &AcceptanceLaunch,
        reason: AcceptanceUnavailableReason,
    ) -> Result<AcceptanceIngressResult, String> {
        let data_dir = self.data_dir.clone();
        let request = ProviderLifecycleUnavailableRequest {
            slot_id: launch.slot_id.clone(),
            binding_id: launch.binding_id.clone(),
            capability: launch.capability.clone(),
            provider: protocol_provider(launch.provider),
            agent_session_id: launch.scope.agent_session_id.clone(),
            reason: protocol_unavailable_reason(reason),
        };
        let response = tokio::task::spawn_blocking(move || {
            let client = LocalApiHttpClient::discover(&data_dir)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "local API discovery is unavailable".to_string())?;
            client
                .post_json::<_, ProviderLifecycleReceiveResponse>(
                    &["v1", "provider-lifecycle", "unavailable"],
                    &request,
                )
                .map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| error.to_string())??;
        Ok(match response {
            ProviderLifecycleReceiveResponse::Applied => AcceptanceIngressResult::Applied,
            ProviderLifecycleReceiveResponse::Duplicate => AcceptanceIngressResult::Duplicate,
            ProviderLifecycleReceiveResponse::Rejected { reason } => {
                AcceptanceIngressResult::Rejected { reason }
            }
        })
    }

    pub async fn event_counts(
        &self,
        agent_session_id: &str,
    ) -> Result<AcceptanceEventCounts, String> {
        let page = self
            .store
            .load_stream(LoadStreamRequest {
                stream_id: StreamId::provider_lifecycle(agent_session_id)
                    .map_err(|error| error.to_string())?,
                after: None,
                limit: 1_024,
            })
            .await
            .map_err(|error| error.to_string())?;
        let mut counts = AcceptanceEventCounts {
            provider_lifecycle: 0,
            other: 0,
        };
        for event in page.events {
            match event.event {
                LoadedDomainEvent::Known(inner)
                    if matches!(*inner, LocalDomainEvent::ProviderLifecycle(_)) =>
                {
                    counts.provider_lifecycle += 1;
                }
                _ => counts.other += 1,
            }
        }
        Ok(counts)
    }

    pub fn ledger_event_counts(&self) -> Result<AcceptanceLedgerEventCounts, String> {
        self.store
            .submit_indexed_query_blocking(|connection| {
                let (all, provider_lifecycle) = connection
                    .query_row(
                        "SELECT COUNT(*),
                                COALESCE(SUM(CASE WHEN event_type = ?1 THEN 1 ELSE 0 END), 0)
                         FROM events",
                        [PROVIDER_LIFECYCLE_EVENT_TYPE],
                        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
                    )
                    .map_err(|_| {
                        crate::domain::local_event::LocalEventQueryError::InvalidRequest
                    })?;
                let all = usize::try_from(all).map_err(|_| {
                    crate::domain::local_event::LocalEventQueryError::InvalidRequest
                })?;
                let provider_lifecycle = usize::try_from(provider_lifecycle).map_err(|_| {
                    crate::domain::local_event::LocalEventQueryError::InvalidRequest
                })?;
                Ok(AcceptanceLedgerEventCounts {
                    provider_lifecycle,
                    other: all.saturating_sub(provider_lifecycle),
                })
            })
            .map_err(|error| error.to_string())
    }

    pub fn workflow_runtime_command_count(&self) -> usize {
        self.workflow_runtime_command_count.load(Ordering::SeqCst)
    }
}

impl Drop for ProviderLifecycleAcceptanceHost {
    fn drop(&mut self) {
        self.server.shutdown();
    }
}

fn domain_provider(provider: AcceptanceProvider) -> ProviderKind {
    match provider {
        AcceptanceProvider::Claude => ProviderKind::Claude,
        AcceptanceProvider::Codex => ProviderKind::Codex,
    }
}

fn acceptance_provider(provider: ProviderKind) -> AcceptanceProvider {
    match provider {
        ProviderKind::Claude => AcceptanceProvider::Claude,
        ProviderKind::Codex => AcceptanceProvider::Codex,
    }
}

fn protocol_provider(provider: AcceptanceProvider) -> ProviderLifecycleProvider {
    match provider {
        AcceptanceProvider::Claude => ProviderLifecycleProvider::Claude,
        AcceptanceProvider::Codex => ProviderLifecycleProvider::Codex,
    }
}

fn protocol_unavailable_reason(
    reason: AcceptanceUnavailableReason,
) -> ProviderLifecycleUnavailableReasonRequest {
    match reason {
        AcceptanceUnavailableReason::SessionStartDeadlineExceeded => {
            ProviderLifecycleUnavailableReasonRequest::SessionStartDeadlineExceeded
        }
        AcceptanceUnavailableReason::CodexHookDeliveryUnconfirmed => {
            ProviderLifecycleUnavailableReasonRequest::CodexHookDeliveryUnconfirmed
        }
        AcceptanceUnavailableReason::ProviderHookConfigurationRejected => {
            ProviderLifecycleUnavailableReasonRequest::ProviderHookConfigurationRejected
        }
        AcceptanceUnavailableReason::LocalApiUnavailable => {
            ProviderLifecycleUnavailableReasonRequest::LocalApiUnavailable
        }
    }
}

fn unavailable_reason(reason: ProviderLifecycleUnavailableReason) -> &'static str {
    match reason {
        ProviderLifecycleUnavailableReason::SessionStartDeadlineExceeded => {
            "session_start_deadline_exceeded"
        }
        ProviderLifecycleUnavailableReason::CodexHookDeliveryUnconfirmed => {
            "codex_hook_delivery_unconfirmed"
        }
        ProviderLifecycleUnavailableReason::ProviderHookConfigurationRejected => {
            "provider_hook_configuration_rejected"
        }
        ProviderLifecycleUnavailableReason::LocalApiUnavailable => "local_api_unavailable",
    }
}

fn acceptance_scope(scope: ProviderLifecycleScope) -> AcceptanceScope {
    AcceptanceScope {
        agent_session_id: scope.agent_session_id().to_string(),
    }
}

fn acceptance_fact(event: ProviderLifecycleEvent) -> AcceptanceFactKind {
    match event {
        ProviderLifecycleEvent::BindingArmed {
            slot_id: _,
            binding_id,
            provider,
            scope,
        } => AcceptanceFactKind::BindingArmed {
            binding_id,
            provider: acceptance_provider(provider),
            scope: acceptance_scope(scope),
        },
        ProviderLifecycleEvent::SessionAssociated {
            binding_id,
            provider_session_id,
            transcript_ref,
        } => AcceptanceFactKind::SessionAssociated {
            binding_id,
            provider_session_id,
            transcript_ref,
        },
        ProviderLifecycleEvent::TranscriptAssociated {
            binding_id,
            transcript_ref,
        } => AcceptanceFactKind::TranscriptAssociated {
            binding_id,
            transcript_ref,
        },
        ProviderLifecycleEvent::StopObserved { binding_id } => {
            AcceptanceFactKind::StopObserved { binding_id }
        }
        ProviderLifecycleEvent::StopFailed { binding_id, reason } => {
            AcceptanceFactKind::StopFailed { binding_id, reason }
        }
        ProviderLifecycleEvent::LifecycleUnavailable {
            binding_id,
            provider,
            scope,
            reason,
        } => AcceptanceFactKind::LifecycleUnavailable {
            binding_id,
            provider: acceptance_provider(provider),
            scope: acceptance_scope(scope),
            reason: unavailable_reason(reason).to_string(),
        },
        ProviderLifecycleEvent::BindingExpired { binding_id } => {
            AcceptanceFactKind::BindingExpired { binding_id }
        }
    }
}
