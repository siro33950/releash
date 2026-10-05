use std::sync::Arc;

pub(crate) fn state_subscriptions() -> crate::usecase::state_subscription::StateSubscriptionUsecase
{
    crate::usecase::state_subscription::StateSubscriptionUsecase::new_with_output(
        crate::adaptor::presenter::state_subscription::test_output(),
        crate::adaptor::controller::state_subscription::drive(Arc::new(|| {
            let period = crate::domain::git_host::CacheTtl::EXTERNAL_INFORMATION.duration();
            Box::pin(crate::infrastructure::timer::ticks_after(period, period))
        })),
    )
}

pub(crate) fn build_client_dependencies(
    data_dir: std::path::PathBuf,
) -> crate::adaptor::controller::client::ClientDependencies {
    crate::adaptor::controller::client::ClientDependencies {
        workspace_node_command_usecase: None,
        app_state: None,
        workspace_state_store: None,
        agent_session_lifecycle_usecase: None,
        agent_session_launch_usecase: None,
        agent_session_read_usecase: None,
        provider_availability_usecase: None,
        agent_session_history_read_usecase: None,
        provider_hook_health_read_usecase: None,
        review_comment_usecase: None,
        config_repository: None,
        app_config_usecase: None,
        workflow_runtime_usecase: None,
        editor_launcher: Arc::new(
            crate::adaptor::gateway::external_editor::NativeEditorLauncherGateway,
        ),
        watcher: Arc::new(crate::usecase::watcher::WatcherUsecase::new(
            None,
            Arc::new(
                crate::adaptor::gateway::repository::file_watcher::FileWatcherGateway::new(
                    Arc::new(crate::infrastructure::file_watcher::FileWatcherManager::default()),
                ),
            ),
        )),
        data_dir: Ok(data_dir),
        daemon: crate::usecase::daemon::DaemonUsecase(crate::adaptor::gateway::daemon::serving()),
        process_port: tokio::sync::mpsc::channel(1).0,
    }
}
