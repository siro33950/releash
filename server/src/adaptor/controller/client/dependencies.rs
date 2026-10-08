use std::sync::Arc;

pub struct ClientDependencies {
    pub(crate) workspace_node_command_usecase:
        Option<std::sync::Arc<crate::usecase::workflow::WorkspaceNodeCommandUsecase>>,
    pub app_state: Option<crate::adaptor::controller::state::AppState>,
    pub workspace_state_store:
        Option<std::sync::Arc<crate::adaptor::gateway::workspace_state::WorkspaceStateStore>>,
    pub(crate) agent_session_lifecycle_usecase:
        Option<std::sync::Arc<crate::usecase::agent_session::AgentSessionLifecycleUsecase>>,
    pub(crate) agent_session_launch_usecase:
        Option<std::sync::Arc<crate::usecase::agent_session::AgentSessionLaunchUsecase>>,
    pub(crate) agent_session_read_usecase:
        Option<std::sync::Arc<crate::usecase::agent_session::AgentSessionReadUsecase>>,
    pub provider_availability_usecase:
        Option<std::sync::Arc<crate::usecase::agent_session::ProviderAvailabilityUsecase>>,
    pub(crate) agent_session_history_read_usecase:
        Option<std::sync::Arc<crate::usecase::agent_session::AgentSessionHistoryReadUsecase>>,
    pub(crate) provider_hook_health_read_usecase:
        Option<std::sync::Arc<crate::usecase::provider_lifecycle::ProviderHookHealthReadUsecase>>,
    pub session_review_usecase:
        Option<std::sync::Arc<crate::usecase::comment::SessionReviewUsecase>>,
    pub review_comment_usecase:
        Option<std::sync::Arc<crate::usecase::comment::ReviewCommentUsecase>>,
    pub config_repository: Option<std::sync::Arc<dyn crate::domain::app_config::ConfigRepository>>,
    pub app_config_usecase: Option<std::sync::Arc<crate::usecase::app_config::AppConfigUsecase>>,
    pub workflow_runtime_usecase:
        Option<std::sync::Arc<crate::usecase::workflow::WorkflowRuntimeUsecase>>,
    pub(crate) editor_launcher: Arc<dyn crate::domain::external_editor::EditorLauncherGateway>,
    pub watcher: Arc<crate::usecase::watcher::WatcherUsecase>,
    pub(crate) data_dir: Result<std::path::PathBuf, crate::adaptor::presenter::error::AppError>,
    pub(crate) installation_usecase: Option<Arc<crate::usecase::installation::InstallationUsecase>>,
    pub daemon: crate::usecase::daemon::DaemonUsecase,
    pub process_port: tokio::sync::mpsc::Sender<i32>,
}
