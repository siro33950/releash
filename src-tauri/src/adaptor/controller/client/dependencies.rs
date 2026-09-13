use std::sync::Arc;

pub(crate) struct ClientDependencies {
    pub(crate) caller_attempt_journal: Option<
        std::sync::Arc<crate::usecase::application_lifecycle::operation::CallerAttemptJournal>,
    >,
    pub(crate) shutdown_coordinator:
        Option<std::sync::Arc<crate::usecase::shutdown_coordinator::ShutdownCoordinator>>,
    pub(crate) application_startup_authority:
        Option<std::sync::Arc<crate::usecase::application_startup::ApplicationStartupAuthority>>,
    pub(crate) application_process_action_dispatcher: Option<
        std::sync::Arc<
            crate::adaptor::controller::application_lifecycle::ApplicationProcessActionDispatcher,
        >,
    >,
    pub(crate) workspace_node_command_usecase:
        Option<std::sync::Arc<crate::usecase::workflow::WorkspaceNodeCommandUsecase>>,
    pub(crate) app_state: Option<crate::adaptor::controller::state::AppState>,
    pub(crate) workspace_state_store:
        Option<std::sync::Arc<crate::adaptor::gateway::workspace_state::WorkspaceStateStore>>,
    pub(crate) agent_session_lifecycle_usecase:
        Option<std::sync::Arc<crate::usecase::agent_session::AgentSessionLifecycleUsecase>>,
    pub(crate) agent_session_launch_usecase:
        Option<std::sync::Arc<crate::usecase::agent_session::AgentSessionLaunchUsecase>>,
    pub(crate) agent_session_read_usecase:
        Option<std::sync::Arc<crate::usecase::agent_session::AgentSessionReadUsecase>>,
    pub(crate) provider_availability_usecase:
        Option<std::sync::Arc<crate::usecase::agent_session::ProviderAvailabilityUsecase>>,
    pub(crate) agent_session_history_read_usecase:
        Option<std::sync::Arc<crate::usecase::agent_session::AgentSessionHistoryReadUsecase>>,
    pub(crate) provider_hook_health_read_usecase:
        Option<std::sync::Arc<crate::usecase::provider_lifecycle::ProviderHookHealthReadUsecase>>,
    pub(crate) review_comment_usecase:
        Option<std::sync::Arc<crate::usecase::comment::ReviewCommentUsecase>>,
    pub(crate) config_repository:
        Option<std::sync::Arc<dyn crate::domain::app_config::ConfigRepository>>,
    pub(crate) workflow_runtime_usecase:
        Option<std::sync::Arc<crate::usecase::workflow::WorkflowRuntimeUsecase>>,
    pub(crate) editor_launcher: Arc<dyn crate::domain::external_editor::EditorLauncherGateway>,
    pub(crate) watcher: Arc<crate::usecase::watcher::WatcherUsecase>,
    pub(crate) data_dir: Result<std::path::PathBuf, String>,
    pub(crate) comment_notify: Arc<crate::adaptor::gateway::push::CommentChangeGateway>,
    pub(crate) process_port:
        Arc<dyn crate::adaptor::controller::application_lifecycle::ApplicationProcessActionPort>,
}
