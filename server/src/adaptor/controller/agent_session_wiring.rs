use std::path::PathBuf;
use std::sync::Arc;

use crate::adaptor::gateway::agent_session::{
    LocalAgentSessionHistoryGateway, LocalAgentSessionHistoryQueryService,
    LocalAgentSessionQueryService, LocalAgentSessionRepository, LocalProviderAgentLaunchGateway,
};
use crate::adaptor::gateway::provider_lifecycle::{
    LocalProviderHookHealthFailureQuery, LocalProviderHookHealthRepository,
    LocalProviderLifecycleCredentialGateway, LocalProviderLifecycleEventRepository,
};
use crate::domain::agent_session::{ProviderAvailabilityReader, ProviderExecutableProbeGateway};
use crate::domain::app_config::AppConfigRepository;
use crate::usecase::agent_session::{
    AgentSessionExitUsecase, AgentSessionHistoryReadUsecase, AgentSessionInitialInstructionUsecase,
    AgentSessionLaunchUsecase, AgentSessionLifecycleUsecase, AgentSessionQueryService,
    AgentSessionReadUsecase, AgentSessionRenameUsecase, AgentSessionUsecase,
    ExecutionTreeCacheReleaseError, ProviderAgentRuntime, ProviderAvailabilityUsecase,
    ProviderAvailabilityUsecaseError, ProviderSessionTitleIngestionUsecase,
    StartedExecutionTreeRegistrationError,
};
use crate::usecase::provider_lifecycle::{
    ProviderExecutionTreeStopCommand, ProviderExecutionTreeStopTransaction,
    ProviderHookHealthReadUsecase, ProviderHookHealthUsecase, ProviderLifecycleIngressUsecase,
    ProviderLifecycleIngressUsecaseError, ProviderLifecycleUsecase,
};
use crate::usecase::state_subscription::StateSubscriptionUsecase;
use crate::usecase::terminal_surface::application::TerminalSurfaceApplication;

pub struct AgentSessionCompositionInput {
    pub launch_retention:
        tokio::sync::mpsc::UnboundedSender<crate::usecase::agent_session::LaunchRetention>,
    pub retrying: Arc<crate::usecase::retry::Retrying>,
    pub state_publisher: Option<crate::usecase::state_subscription::StateSubscriptionUsecase>,
    pub store: Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>,
    pub data_dir: PathBuf,
    pub provider_executable_config: Arc<dyn AppConfigRepository>,
    pub git: Arc<dyn crate::domain::git::GitService>,
    pub config_mutation: Arc<parking_lot::Mutex<()>>,
    pub provider_executable_probe: Arc<dyn ProviderExecutableProbeGateway>,
    pub claude_config_dir: PathBuf,
    pub codex_home: PathBuf,
    pub cli_binary: String,
    pub hook_token: Arc<str>,
    pub terminal: Arc<TerminalSurfaceApplication>,
    pub subscriptions: StateSubscriptionUsecase,
}

pub struct AgentSessionComposition {
    pub(crate) provider_lifecycle: Arc<ProviderLifecycleUsecase>,
    pub(crate) sessions: Arc<AgentSessionUsecase>,
    pub(crate) usage: Arc<crate::usecase::provider_usage::ProviderUsageUsecase>,
    pub(crate) history_read: Arc<AgentSessionHistoryReadUsecase>,
    pub(crate) provider_session_title_ingestion: Arc<ProviderSessionTitleIngestionUsecase>,
    pub(crate) rename: Arc<AgentSessionRenameUsecase>,
    pub(crate) hook_health_read: Arc<ProviderHookHealthReadUsecase>,
    pub(crate) lifecycle_ingress: Arc<ProviderLifecycleIngressUsecase>,
    pub launch: Arc<AgentSessionLaunchUsecase>,
    pub initial_instruction: Arc<AgentSessionInitialInstructionUsecase>,
    pub lifecycle: Arc<AgentSessionLifecycleUsecase>,
    pub(crate) exit: Arc<AgentSessionExitUsecase>,
    pub(crate) read: Arc<AgentSessionReadUsecase>,
    pub(crate) provider_availability: Arc<ProviderAvailabilityUsecase>,
    pub availability_reader: Arc<dyn ProviderAvailabilityReader>,
    pub(crate) execution_tree_stops: Arc<DeferredProviderExecutionTreeStopTransaction>,
    pub(crate) execution_tree_registrations: Arc<DeferredStartedExecutionTreeRegistrar>,
}

pub(crate) struct DeferredStartedExecutionTreeRegistrar {
    target: std::sync::RwLock<
        Option<std::sync::Weak<crate::usecase::workflow::WorkflowRuntimeUsecase>>,
    >,
}

impl DeferredStartedExecutionTreeRegistrar {
    fn new() -> Self {
        Self {
            target: std::sync::RwLock::new(None),
        }
    }

    pub(crate) fn bind(&self, target: Arc<crate::usecase::workflow::WorkflowRuntimeUsecase>) {
        *self
            .target
            .write()
            .expect("execution tree registrar poisoned") = Some(Arc::downgrade(&target));
    }
}

impl crate::usecase::agent_session::WorktreeMutationAdmission
    for DeferredStartedExecutionTreeRegistrar
{
    fn begin_worktree_mutation(
        &self,
        path: &str,
    ) -> Result<
        crate::usecase::worktree_operation::WorktreeMutationGuard,
        crate::domain::workflow::WorkflowError,
    > {
        let target = self
            .target
            .read()
            .map_err(|_| {
                crate::domain::workflow::WorkflowError::external(
                    "execution tree registrar lock poisoned",
                )
            })?
            .as_ref()
            .and_then(std::sync::Weak::upgrade)
            .ok_or_else(|| {
                crate::domain::workflow::WorkflowError::external(
                    "execution tree registrar unavailable",
                )
            })?;
        target.begin_worktree_mutation(path)
    }
}

#[async_trait::async_trait]
impl crate::usecase::agent_session::ExecutionTreeCache for DeferredStartedExecutionTreeRegistrar {
    async fn release_deleted_execution_tree(
        &self,
        tree_id: &str,
    ) -> Result<(), ExecutionTreeCacheReleaseError> {
        let target = self
            .target
            .read()
            .map_err(|_| ExecutionTreeCacheReleaseError::Corrupt)?
            .as_ref()
            .and_then(std::sync::Weak::upgrade)
            .ok_or(ExecutionTreeCacheReleaseError::Unavailable)?;
        target.release_deleted_execution_tree(tree_id).await
    }
}

#[async_trait::async_trait]
impl crate::usecase::agent_session::StartedExecutionTreeRegistrar
    for DeferredStartedExecutionTreeRegistrar
{
    async fn register_started_execution_tree(
        &self,
        tree_id: &str,
    ) -> Result<(), StartedExecutionTreeRegistrationError> {
        let target = self
            .target
            .read()
            .map_err(|_| StartedExecutionTreeRegistrationError::Corrupt)?
            .as_ref()
            .and_then(std::sync::Weak::upgrade)
            .ok_or(StartedExecutionTreeRegistrationError::Unavailable)?;
        target.register_started_execution_tree(tree_id).await
    }
}

#[async_trait::async_trait]
impl crate::usecase::agent_session::AgentSessionExecutionTreeLifecycle
    for DeferredStartedExecutionTreeRegistrar
{
    async fn lock_execution_tree(
        &self,
        tree_id: &str,
    ) -> Result<tokio::sync::OwnedMutexGuard<()>, crate::domain::workflow::WorkflowError> {
        let target = self
            .target
            .read()
            .map_err(|_| {
                crate::domain::workflow::WorkflowError::external(
                    "execution tree registrar poisoned",
                )
            })?
            .as_ref()
            .and_then(std::sync::Weak::upgrade)
            .ok_or_else(|| {
                crate::domain::workflow::WorkflowError::external(
                    "execution tree runtime unavailable",
                )
            })?;
        target.lock_execution_tree(tree_id).await
    }

    async fn archive_execution_tree(
        &self,
        tree_id: &str,
    ) -> Result<(), crate::domain::workflow::WorkflowError> {
        let target = self
            .target
            .read()
            .map_err(|_| {
                crate::domain::workflow::WorkflowError::external(
                    "execution tree registrar poisoned",
                )
            })?
            .as_ref()
            .and_then(std::sync::Weak::upgrade)
            .ok_or_else(|| {
                crate::domain::workflow::WorkflowError::external(
                    "execution tree runtime unavailable",
                )
            })?;
        target.archive_execution_tree(tree_id, "manual").await
    }

    async fn restore_execution_tree(
        &self,
        tree_id: &str,
    ) -> Result<(), crate::domain::workflow::WorkflowError> {
        let target = self
            .target
            .read()
            .map_err(|_| {
                crate::domain::workflow::WorkflowError::external(
                    "execution tree registrar poisoned",
                )
            })?
            .as_ref()
            .and_then(std::sync::Weak::upgrade)
            .ok_or_else(|| {
                crate::domain::workflow::WorkflowError::external(
                    "execution tree runtime unavailable",
                )
            })?;
        target.restore_execution_tree_locked(tree_id).await
    }
}

pub(crate) struct DeferredProviderExecutionTreeStopTransaction {
    target: std::sync::RwLock<Option<Arc<dyn ProviderExecutionTreeStopTransaction>>>,
}

impl DeferredProviderExecutionTreeStopTransaction {
    fn new() -> Self {
        Self {
            target: std::sync::RwLock::new(None),
        }
    }

    pub(crate) fn bind(&self, target: Arc<dyn ProviderExecutionTreeStopTransaction>) {
        *self
            .target
            .write()
            .expect("execution tree Stop router poisoned") = Some(target);
    }
}

#[async_trait::async_trait]
impl ProviderExecutionTreeStopTransaction for DeferredProviderExecutionTreeStopTransaction {
    fn begin_worktree_mutation(
        &self,
        path: &str,
    ) -> Result<
        crate::usecase::worktree_operation::WorktreeMutationGuard,
        ProviderLifecycleIngressUsecaseError,
    > {
        let target = self
            .target
            .read()
            .map_err(|_| ProviderLifecycleIngressUsecaseError::Corrupt)?
            .clone()
            .ok_or(ProviderLifecycleIngressUsecaseError::StorageUnavailable)?;
        target.begin_worktree_mutation(path)
    }

    async fn commit_provider_stop(
        &self,
        command: ProviderExecutionTreeStopCommand,
        lifecycle_events: Vec<crate::domain::provider_lifecycle::ScopedProviderLifecycleEvent>,
    ) -> Result<(), ProviderLifecycleIngressUsecaseError> {
        let target = self
            .target
            .read()
            .map_err(|_| ProviderLifecycleIngressUsecaseError::Corrupt)?
            .clone()
            .ok_or(ProviderLifecycleIngressUsecaseError::StorageUnavailable)?;
        target.commit_provider_stop(command, lifecycle_events).await
    }
}

pub fn compose_agent_sessions(
    input: AgentSessionCompositionInput,
) -> Result<AgentSessionComposition, ProviderAvailabilityUsecaseError> {
    todo!()
}
