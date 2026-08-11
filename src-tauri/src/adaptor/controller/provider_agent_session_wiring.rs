use std::path::PathBuf;
use std::sync::Arc;

use crate::adaptor::gateway::agent_session::{
    LocalProviderAgentLaunchGateway, LocalProviderAgentSessionHistoryGateway,
    LocalProviderAgentSessionHistoryQueryService, LocalProviderAgentSessionQueryService,
    LocalProviderAgentSessionRepository,
};
use crate::adaptor::gateway::provider_lifecycle::{
    LocalProviderHookHealthFailureQuery, LocalProviderHookHealthRepository,
    LocalProviderLifecycleCredentialGateway, LocalProviderLifecycleEventRepository,
};
use crate::domain::agent_session::{
    ProviderAvailabilityReader, ProviderExecutableConfigRepository, ProviderExecutableProbeGateway,
};
use crate::domain::local_event::LocalEventTransactionRepository;
use crate::usecase::agent_session::{
    ProviderAgentInitialInstructionUsecase, ProviderAgentSessionActivityUsecase,
    ProviderAgentSessionChangeNotifier, ProviderAgentSessionExitUsecase,
    ProviderAgentSessionHistoryReadUsecase, ProviderAgentSessionLaunchUsecase,
    ProviderAgentSessionLifecycleUsecase, ProviderAgentSessionQueryService,
    ProviderAgentSessionReadUsecase, ProviderAgentSessionUsecase, ProviderAvailabilityUsecase,
    ProviderAvailabilityUsecaseError,
};
use crate::usecase::provider_lifecycle::{
    ProviderHookHealthReadUsecase, ProviderHookHealthUsecase, ProviderLifecycleIngressUsecase,
    ProviderLifecycleIngressUsecaseError, ProviderLifecycleUsecase, ProviderWorkflowStopCommand,
    ProviderWorkflowStopTransaction,
};
use crate::usecase::terminal_surface::application::TerminalSurfaceApplication;

pub(crate) struct ProviderAgentSessionCompositionInput {
    pub(crate) repository: Arc<dyn LocalEventTransactionRepository>,
    pub(crate) installation_id: String,
    pub(crate) data_dir: PathBuf,
    pub(crate) provider_executable_config: Arc<dyn ProviderExecutableConfigRepository>,
    pub(crate) provider_executable_probe: Arc<dyn ProviderExecutableProbeGateway>,
    pub(crate) claude_config_dir: PathBuf,
    pub(crate) codex_home: PathBuf,
    pub(crate) cli_binary: String,
    pub(crate) terminal: Arc<TerminalSurfaceApplication>,
    pub(crate) change_notifier: Arc<dyn ProviderAgentSessionChangeNotifier>,
}

pub(crate) struct ProviderAgentSessionComposition {
    pub(crate) provider_lifecycle: Arc<ProviderLifecycleUsecase>,
    pub(crate) sessions: Arc<ProviderAgentSessionUsecase>,
    pub(crate) history_read: Arc<ProviderAgentSessionHistoryReadUsecase>,
    pub(crate) hook_health: Arc<ProviderHookHealthUsecase>,
    pub(crate) hook_health_read: Arc<ProviderHookHealthReadUsecase>,
    pub(crate) lifecycle_ingress: Arc<ProviderLifecycleIngressUsecase>,
    pub(crate) launch: Arc<ProviderAgentSessionLaunchUsecase>,
    pub(crate) initial_instruction: Arc<ProviderAgentInitialInstructionUsecase>,
    pub(crate) lifecycle: Arc<ProviderAgentSessionLifecycleUsecase>,
    pub(crate) exit: Arc<ProviderAgentSessionExitUsecase>,
    pub(crate) activity: Arc<ProviderAgentSessionActivityUsecase>,
    pub(crate) read: Arc<ProviderAgentSessionReadUsecase>,
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

pub(crate) fn compose_provider_agent_sessions(
    input: ProviderAgentSessionCompositionInput,
) -> Result<ProviderAgentSessionComposition, ProviderAvailabilityUsecaseError> {
    let provider_lifecycle = Arc::new(ProviderLifecycleUsecase::new(
        Arc::new(LocalProviderLifecycleCredentialGateway),
        Arc::new(LocalProviderLifecycleEventRepository::new(
            input.repository.clone(),
            input.installation_id.clone(),
        )),
    ));
    let session_repository = Arc::new(LocalProviderAgentSessionRepository::new(
        input.repository.clone(),
        input.installation_id.clone(),
    ));
    let sessions = Arc::new(ProviderAgentSessionUsecase::new(session_repository.clone()));
    let hook_health = Arc::new(ProviderHookHealthUsecase::new(Arc::new(
        LocalProviderHookHealthRepository::new(input.repository.clone(), input.installation_id),
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
    let history_gateway = Arc::new(LocalProviderAgentSessionHistoryGateway::new(
        input.claude_config_dir,
        input.codex_home,
    ));
    let history_read = Arc::new(ProviderAgentSessionHistoryReadUsecase::new(Arc::new(
        LocalProviderAgentSessionHistoryQueryService::new(
            history_gateway.clone(),
            session_repository,
        ),
    )));
    let launch = Arc::new(ProviderAgentSessionLaunchUsecase::new(
        sessions.clone(),
        provider_lifecycle.clone(),
        availability_reader.clone(),
        launch_gateway.clone(),
        input.terminal.clone(),
        history_gateway,
        hook_health.clone(),
    ));
    let lifecycle = Arc::new(ProviderAgentSessionLifecycleUsecase::new(
        sessions.clone(),
        provider_lifecycle.clone(),
        launch_gateway,
        availability_reader.clone(),
        input.terminal.clone(),
        hook_health.clone(),
        input.change_notifier.clone(),
    ));
    let query: Arc<dyn ProviderAgentSessionQueryService> =
        Arc::new(LocalProviderAgentSessionQueryService::new(input.repository));
    let read = Arc::new(ProviderAgentSessionReadUsecase::new(
        query,
        lifecycle.clone(),
        input.terminal.clone(),
    ));
    let initial_instruction = Arc::new(ProviderAgentInitialInstructionUsecase::new(
        sessions.clone(),
        input.terminal.clone(),
    ));
    let activity = Arc::new(ProviderAgentSessionActivityUsecase::new(
        input.terminal.clone(),
        input.change_notifier,
        tokio::runtime::Handle::current(),
    ));
    let exit = Arc::new(ProviderAgentSessionExitUsecase::new(
        input.terminal,
        lifecycle.clone(),
    ));

    Ok(ProviderAgentSessionComposition {
        provider_lifecycle,
        sessions,
        history_read,
        hook_health,
        hook_health_read,
        lifecycle_ingress,
        launch,
        initial_instruction,
        lifecycle,
        exit,
        activity,
        read,
        provider_availability,
        availability_reader,
        workflow_stops,
    })
}
