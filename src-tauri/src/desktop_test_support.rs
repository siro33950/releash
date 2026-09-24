use std::sync::Arc;
use tauri::Manager;

pub(crate) struct TestDataDir(pub(crate) std::path::PathBuf);

pub(crate) fn data_dir<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> Result<std::path::PathBuf, String> {
    app.try_state::<TestDataDir>()
        .map(|value| Ok(value.0.clone()))
        .unwrap_or_else(crate::infrastructure::platform::app_data_dir::resolve_data_dir)
}

pub(crate) fn push_sink<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> Arc<crate::infrastructure::push::PushSink> {
    app.state::<Arc<crate::infrastructure::push::PushSink>>()
        .inner()
        .clone()
}

pub(crate) fn workflow_dependencies<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> crate::adaptor::gateway::workflow::workflow_host::WorkflowRuntimeDependencies {
    crate::adaptor::gateway::workflow::workflow_host::WorkflowRuntimeDependencies {
        store: app
            .try_state::<Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>>()
            .map(|state| state.inner().clone()),
        config: app
            .try_state::<Arc<dyn crate::domain::app_config::ConfigRepository>>()
            .map(|state| state.inner().clone()),
        secrets: app
            .try_state::<Arc<dyn crate::domain::app_config::ConfigSecretRepository>>()
            .map(|state| state.inner().clone()),
        state_changes: crate::usecase::state_subscription::StateSubscriptionPublisher::for_test(),
    }
}

pub(crate) fn build_watcher_usecase<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> std::sync::Arc<crate::usecase::watcher::WatcherUsecase> {
    use tauri::Manager;
    std::sync::Arc::new(crate::usecase::watcher::WatcherUsecase::new(
        app.try_state::<crate::adaptor::controller::state::AppState>().map(|state| state.repository_state.clone()),
        std::sync::Arc::new(crate::adaptor::gateway::repository::file_watcher::FileWatcherGateway::new(
            app.state::<std::sync::Arc<crate::infrastructure::file_watcher::FileWatcherManager>>().inner().clone(), push_sink(app),
        )),
    ))
}

pub(crate) fn build_client_dependencies<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> crate::adaptor::controller::client::ClientDependencies {
    use tauri::Manager;
    crate::adaptor::controller::client::ClientDependencies {
        application_startup_authority: app.try_state::<std::sync::Arc<crate::usecase::application_startup::ApplicationStartupAuthority>>().map(|state| state.inner().clone()),
        workspace_node_command_usecase: app.try_state::<std::sync::Arc<crate::usecase::workflow::WorkspaceNodeCommandUsecase>>().map(|state| state.inner().clone()),
        app_state: app.try_state::<crate::adaptor::controller::state::AppState>().map(|state| state.inner().clone()),
        workspace_state_store: app.try_state::<std::sync::Arc<crate::adaptor::gateway::workspace_state::WorkspaceStateStore>>().map(|state| state.inner().clone()),
        agent_session_lifecycle_usecase: app.try_state::<std::sync::Arc<crate::usecase::agent_session::AgentSessionLifecycleUsecase>>().map(|state| state.inner().clone()),
        agent_session_launch_usecase: app.try_state::<std::sync::Arc<crate::usecase::agent_session::AgentSessionLaunchUsecase>>().map(|state| state.inner().clone()),
        agent_session_read_usecase: app.try_state::<std::sync::Arc<crate::usecase::agent_session::AgentSessionReadUsecase>>().map(|state| state.inner().clone()),
        provider_availability_usecase: app.try_state::<std::sync::Arc<crate::usecase::agent_session::ProviderAvailabilityUsecase>>().map(|state| state.inner().clone()),
        agent_session_history_read_usecase: app.try_state::<std::sync::Arc<crate::usecase::agent_session::AgentSessionHistoryReadUsecase>>().map(|state| state.inner().clone()),
        provider_hook_health_read_usecase: app.try_state::<std::sync::Arc<crate::usecase::provider_lifecycle::ProviderHookHealthReadUsecase>>().map(|state| state.inner().clone()),
        review_comment_usecase: app.try_state::<std::sync::Arc<crate::usecase::comment::ReviewCommentUsecase>>().map(|state| state.inner().clone()),
        config_repository: app.try_state::<std::sync::Arc<dyn crate::domain::app_config::ConfigRepository>>().map(|state| state.inner().clone()),
        workflow_runtime_usecase: app.try_state::<std::sync::Arc<crate::usecase::workflow::WorkflowRuntimeUsecase>>().map(|state| state.inner().clone()),
        editor_launcher: Arc::new(crate::adaptor::gateway::external_editor::NativeEditorLauncherGateway),
        watcher: build_watcher_usecase(app),
        data_dir: data_dir(app).map_err(crate::other::AppError::new),
        comment_notify: Arc::new(crate::adaptor::gateway::push::CommentChangeGateway::new(push_sink(app))),
        process_port: Arc::new(crate::adaptor::gateway::application_lifecycle::TauriApplicationQuitIntentPort::new(app.clone())),
    }
}
