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

pub fn build_client_dependencies(
    data_dir: std::path::PathBuf,
) -> crate::adaptor::controller::client::ClientDependencies {
    crate::adaptor::controller::client::ClientDependencies {
        create_worktrees_usecase: None,
        installation_usecase: Some(Arc::new(crate::usecase::installation::InstallationUsecase(
            Arc::new(crate::adaptor::gateway::installation::LocalInstallationService),
        ))),
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
        session_review_usecase: None,
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
        daemon: crate::usecase::daemon::DaemonUsecase::new(
            crate::adaptor::gateway::daemon::serving(),
        ),
        process_port: tokio::sync::mpsc::channel(1).0,
    }
}

pub fn workflow_read(
    store: Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>,
    data_dir: impl Into<std::path::PathBuf>,
    workflows_dir: Option<std::path::PathBuf>,
) -> crate::usecase::workflow::WorkflowReadUsecase {
    workflow_services(store, data_dir, workflows_dir).read_usecase()
}

pub fn workflow_services(
    store: Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>,
    data_dir: impl Into<std::path::PathBuf>,
    workflows_dir: Option<std::path::PathBuf>,
) -> crate::usecase::workflow::WorkflowUsecase {
    crate::adaptor::controller::wiring::build_workflow_services_with_gateways(
        Arc::new(crate::adaptor::gateway::failure_records::FailureRecordStore::default()),
        data_dir,
        Arc::new(crate::adaptor::gateway::workflow::PassthroughManagedWorktreeGateway),
        Arc::new(crate::adaptor::gateway::workflow::NoopWorkflowExternalEditorGateway),
        store,
        None,
        workflows_dir,
    )
    .0
}

pub fn write_hook_failure(
    _data_dir: &std::path::Path,
    marker_path: &std::path::Path,
    provider: &str,
    launch_id: &str,
) -> std::io::Result<()> {
    std::fs::create_dir_all(marker_path.parent().unwrap())?;
    std::fs::write(
        marker_path,
        serde_json::to_vec(
            &serde_json::json!({"provider":provider,"launchId":launch_id,"reason":"local_api_unavailable"}),
        )?,
    )
}

pub fn client_binding(
    data_dir: std::path::PathBuf,
) -> Result<
    (
        crate::infrastructure::local_api::LocalApiServerBinding,
        crate::usecase::daemon::DaemonUsecase,
    ),
    crate::infrastructure::local_api::LocalApiServerError,
> {
    let pid = std::process::id();
    let started = crate::infrastructure::local_api::process_start_time(pid).ok_or_else(|| {
        crate::infrastructure::local_api::LocalApiServerError::Discovery(std::io::Error::other(
            "process identity unavailable",
        ))
    })?;
    let id = uuid::Uuid::new_v4().to_string();
    let binding = crate::infrastructure::local_api::LocalApiServerBinding::bind(
        data_dir,
        id.clone(),
        pid,
        started,
        std::sync::Arc::<str>::from("hook-token").into(),
    )?;
    let daemon = crate::usecase::daemon::DaemonUsecase::new(
        crate::adaptor::gateway::daemon::serving_with_identity(
            crate::domain::daemon::DaemonIdentity {
                daemon_id: id,
                pid,
                process_started_at: started,
            },
        ),
    );
    Ok((binding, daemon))
}
