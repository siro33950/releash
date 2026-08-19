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
use crate::domain::agent_session::{
    ProviderAvailabilityReader, ProviderExecutableConfigRepository, ProviderExecutableProbeGateway,
};
use crate::usecase::agent_session::{
    AgentSessionActivityUsecase, AgentSessionChangeNotifier, AgentSessionExitUsecase,
    AgentSessionHistoryReadUsecase, AgentSessionInitialInstructionUsecase,
    AgentSessionInterruptUsecase, AgentSessionLaunchUsecase, AgentSessionLifecycleUsecase,
    AgentSessionQueryService, AgentSessionReadUsecase, AgentSessionUsecase,
    ProviderAvailabilityUsecase, ProviderAvailabilityUsecaseError,
};
use crate::usecase::provider_lifecycle::{
    ProviderHookHealthReadUsecase, ProviderHookHealthUsecase, ProviderLifecycleIngressUsecase,
    ProviderLifecycleIngressUsecaseError, ProviderLifecycleUsecase, ProviderWorkflowStopCommand,
    ProviderWorkflowStopTransaction,
};
use crate::usecase::terminal_surface::application::TerminalSurfaceApplication;

pub(crate) struct AgentSessionCompositionInput {
    pub(crate) store: Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>,
    pub(crate) data_dir: PathBuf,
    pub(crate) provider_executable_config: Arc<dyn ProviderExecutableConfigRepository>,
    pub(crate) provider_executable_probe: Arc<dyn ProviderExecutableProbeGateway>,
    pub(crate) claude_config_dir: PathBuf,
    pub(crate) codex_home: PathBuf,
    pub(crate) cli_binary: String,
    pub(crate) terminal: Arc<TerminalSurfaceApplication>,
    pub(crate) change_notifier: Arc<dyn AgentSessionChangeNotifier>,
}

pub(crate) struct AgentSessionComposition {
    pub(crate) provider_lifecycle: Arc<ProviderLifecycleUsecase>,
    pub(crate) sessions: Arc<AgentSessionUsecase>,
    pub(crate) history_read: Arc<AgentSessionHistoryReadUsecase>,
    pub(crate) hook_health: Arc<ProviderHookHealthUsecase>,
    pub(crate) hook_health_read: Arc<ProviderHookHealthReadUsecase>,
    pub(crate) lifecycle_ingress: Arc<ProviderLifecycleIngressUsecase>,
    pub(crate) launch: Arc<AgentSessionLaunchUsecase>,
    pub(crate) initial_instruction: Arc<AgentSessionInitialInstructionUsecase>,
    pub(crate) interrupt: Arc<AgentSessionInterruptUsecase>,
    pub(crate) lifecycle: Arc<AgentSessionLifecycleUsecase>,
    pub(crate) exit: Arc<AgentSessionExitUsecase>,
    pub(crate) activity: Arc<AgentSessionActivityUsecase>,
    pub(crate) read: Arc<AgentSessionReadUsecase>,
    pub(crate) provider_availability: Arc<ProviderAvailabilityUsecase>,
    pub(crate) availability_reader: Arc<dyn ProviderAvailabilityReader>,
    pub(crate) workflow_stops: Arc<DeferredProviderWorkflowStopTransaction>,
}

pub(crate) struct DeferredProviderWorkflowStopTransaction {
    target: std::sync::RwLock<Option<Arc<dyn ProviderWorkflowStopTransaction>>>,
}

impl DeferredProviderWorkflowStopTransaction {
    fn new() -> Self {
        Self {
            target: std::sync::RwLock::new(None),
        }
    }

    pub(crate) fn bind(&self, target: Arc<dyn ProviderWorkflowStopTransaction>) {
        *self.target.write().expect("workflow Stop router poisoned") = Some(target);
    }
}

#[async_trait::async_trait]
impl ProviderWorkflowStopTransaction for DeferredProviderWorkflowStopTransaction {
    async fn commit_provider_stop(
        &self,
        command: ProviderWorkflowStopCommand,
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

pub(crate) fn compose_agent_sessions(
    input: AgentSessionCompositionInput,
) -> Result<AgentSessionComposition, ProviderAvailabilityUsecaseError> {
    let repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository> =
        input.store.clone();
    let installation_id = input.store.installation_id().to_string();
    let provider_lifecycle = Arc::new(ProviderLifecycleUsecase::new(
        Arc::new(LocalProviderLifecycleCredentialGateway),
        Arc::new(LocalProviderLifecycleEventRepository::new(
            repository.clone(),
            installation_id.clone(),
        )),
    ));
    let session_repository = Arc::new(LocalAgentSessionRepository::new(input.store.clone()));
    let sessions = Arc::new(AgentSessionUsecase::new(session_repository.clone()));
    let hook_health = Arc::new(ProviderHookHealthUsecase::new(Arc::new(
        LocalProviderHookHealthRepository::new(repository, installation_id),
    )));
    let hook_health_read = Arc::new(ProviderHookHealthReadUsecase::new(
        hook_health.clone(),
        Arc::new(LocalProviderHookHealthFailureQuery::new(
            input.data_dir.clone(),
        )),
    ));
    let workflow_stops = Arc::new(DeferredProviderWorkflowStopTransaction::new());
    let lifecycle_ingress = Arc::new(ProviderLifecycleIngressUsecase::new(
        provider_lifecycle.clone(),
        sessions.clone(),
        hook_health.clone(),
        session_repository.clone(),
        workflow_stops.clone(),
    ));
    let launch_gateway = Arc::new(LocalProviderAgentLaunchGateway::new(
        input.data_dir,
        input.cli_binary,
    ));
    let provider_availability = Arc::new(ProviderAvailabilityUsecase::initialize(
        input.provider_executable_config,
        input.provider_executable_probe,
    )?);
    let availability_reader: Arc<dyn ProviderAvailabilityReader> = provider_availability.clone();
    let history_gateway = Arc::new(LocalAgentSessionHistoryGateway::new(
        input.claude_config_dir,
        input.codex_home,
    ));
    let history_read = Arc::new(AgentSessionHistoryReadUsecase::new(Arc::new(
        LocalAgentSessionHistoryQueryService::new(history_gateway.clone(), session_repository),
    )));
    let launch = Arc::new(AgentSessionLaunchUsecase::new(
        sessions.clone(),
        provider_lifecycle.clone(),
        availability_reader.clone(),
        launch_gateway.clone(),
        input.terminal.clone(),
        history_gateway,
        hook_health.clone(),
    ));
    let lifecycle = Arc::new(AgentSessionLifecycleUsecase::new(
        sessions.clone(),
        provider_lifecycle.clone(),
        launch_gateway,
        availability_reader.clone(),
        input.terminal.clone(),
        hook_health.clone(),
        input.change_notifier.clone(),
    ));
    let query: Arc<dyn AgentSessionQueryService> =
        Arc::new(LocalAgentSessionQueryService::new(input.store.clone()));
    let read = Arc::new(AgentSessionReadUsecase::new(
        query,
        lifecycle.clone(),
        input.terminal.clone(),
    ));
    let initial_instruction = Arc::new(AgentSessionInitialInstructionUsecase::new(
        sessions.clone(),
        input.terminal.clone(),
    ));
    let interrupt = Arc::new(AgentSessionInterruptUsecase::new(
        sessions.clone(),
        input.terminal.clone(),
    ));
    let activity = Arc::new(AgentSessionActivityUsecase::new(
        input.terminal.clone(),
        input.change_notifier,
        tokio::runtime::Handle::current(),
    ));
    let exit = Arc::new(AgentSessionExitUsecase::new(
        input.terminal,
        lifecycle.clone(),
    ));

    Ok(AgentSessionComposition {
        provider_lifecycle,
        sessions,
        history_read,
        hook_health,
        hook_health_read,
        lifecycle_ingress,
        launch,
        initial_instruction,
        interrupt,
        lifecycle,
        exit,
        activity,
        read,
        provider_availability,
        availability_reader,
        workflow_stops,
    })
}
