use releashd::test_support::integration::persistence::LocalEventStore;
use releashd::test_support::integration::persistence::LocalEventStoreConfig;
use releashd::test_support::integration::platform::build_repository_usecase;
use releashd::test_support::integration::platform::build_terminal_surface_application_for_tests;
use releashd::test_support::integration::platform::build_workflow_runtime_usecase;
use releashd::test_support::integration::platform::build_workflow_services_with_repository_worktrees;
use releashd::test_support::integration::platform::compose_agent_sessions;
use releashd::test_support::integration::platform::AgentSessionCompositionInput;
use releashd::test_support::integration::platform::WorktreeExecutionArchiver;
use releashd::test_support::integration::repository::WorktreeDeletionTarget;
use releashd::test_support::integration::sessions::LocalProviderExecutableProbeGateway;
use releashd::test_support::integration::settings::AppConfig;
use releashd::test_support::integration::settings::ReleashConfig;
use releashd::test_support::integration::workflow::RepositoryIsolatedWorktreeGateway;
use releashd::test_support::integration::workflow::WorkflowNodeProcesses;
use releashd::test_support::integration::workflow::WorkflowRuntimeCommandGatewayDeps;
use releashd::test_support::integration::workflow::WorkflowRuntimeDependencies;
use std::sync::Arc;

#[tokio::test]
pub async fn test_worktree削除一覧_本番runtime配線で受理した削除状態を処理終了まで共有する() {
    // Given
    let retrying = releashd::test_support::integration::platform::test_retrying();
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().canonicalize().unwrap();
    let repo_path = root.join("repo");
    let repo_path = repo_path.to_str().unwrap();
    let repo = git2::Repository::init(repo_path).unwrap();
    crate::test_support_git::create_initial_commit(&repo);
    let repository = Arc::new(build_repository_usecase());
    let worktree = repository
        .create_worktree(repo_path, "feature", true, None)
        .unwrap();
    let data_dir = root.join("data");
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        data_dir.clone(),
        std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
    ))
    .unwrap();
    let config = Arc::new(AppConfig::new(
        ReleashConfig::default(),
        data_dir.join("releash.toml"),
    ));
    let publisher = releashd::test_support::integration::subscriptions::test_subscriptions();
    let terminal = Arc::new(build_terminal_surface_application_for_tests());
    let sessions = compose_agent_sessions(AgentSessionCompositionInput {
        config_mutation: Default::default(),
        hook_token: std::sync::Arc::<str>::from("hook-token"),
        launch_retention: releashd::test_support::integration::platform::run(
            releashd::test_support::integration::platform::delays(
                releashd::test_support::integration::platform::RETENTION,
            ),
        ),
        retrying: retrying.clone(),
        state_publisher: None,
        store: store.clone(),
        data_dir: data_dir.clone(),
        provider_executable_config: config.clone(),
        provider_executable_probe: Arc::new(LocalProviderExecutableProbeGateway::with_search_path(
            None,
        )),
        claude_config_dir: root.join("claude"),
        codex_home: root.join("codex"),
        cli_binary: "releash".into(),
        terminal: terminal.clone(),
        subscriptions: publisher.clone(),
    })
    .unwrap();
    let processes = Arc::new(WorkflowNodeProcesses::new(terminal));
    let (_, workspace_query) = build_workflow_services_with_repository_worktrees(
        Arc::new(releashd::test_support::integration::platform::FailureRecordStore::default()),
        data_dir,
        repository.clone(),
        config.clone(),
        store.clone(),
        processes.clone(),
        root.join("workflows"),
    );
    let (runtime, _) = build_workflow_runtime_usecase(
        retrying.clone(),
        WorkflowRuntimeDependencies {
            store: Some(store),
            config: Some(config.clone()),
            secrets: Some(config.clone()),
            state_changes: publisher.clone(),
        },
        WorkflowRuntimeCommandGatewayDeps {
            node_processes: processes,
            repository_usecase: repository.clone(),
            app_config: config,
            workspace_query,
            agent_session_launch: sessions.launch,
            agent_session_initial_instruction: sessions.initial_instruction,
            agent_session_lifecycle: sessions.lifecycle,
            provider_availability: sessions.availability_reader,
            isolated_worktrees: Arc::new(RepositoryIsolatedWorktreeGateway),
        },
        releashd::test_support::integration::daemon::serving(),
        root.join("workflows"),
    )
    .unwrap();
    let rows = || {
        repository.with_deleting_worktrees(
            repo_path,
            repository.list_working_worktrees(repo_path).unwrap(),
        )
    };
    assert!(rows().iter().all(|(_, deleting)| !deleting));

    // When
    let mut deletion = runtime
        .begin_worktree_deletion(&worktree.path)
        .await
        .unwrap();
    runtime.archive_worktree(&worktree.path).await.unwrap();
    deletion
        .accept(WorktreeDeletionTarget {
            repository_root: repo_path.into(),
            path: worktree.path.clone(),
            branch: Some(worktree.branch.clone()),
        })
        .unwrap();

    // Then
    for git_registration_removed in [false, true] {
        if git_registration_removed {
            releashd::test_support::integration::repository::remove_worktree(
                repo_path,
                &worktree.path,
                false,
            )
            .unwrap();
        }
        let rows = rows();
        assert_eq!(rows.len(), 2);
        let deleting: Vec<_> = rows.iter().filter(|(_, deleting)| *deleting).collect();
        assert_eq!(deleting.len(), 1);
        assert_eq!(deleting[0].0.branch, worktree.branch);
        assert_eq!(deleting[0].0.path, worktree.path);
    }
    drop(deletion);
    let rows = rows();
    assert!(rows.iter().all(|(_, deleting)| !deleting));
    assert_eq!(rows.len(), 1);
}
pub(crate) mod tests {

    use releashd::test_support::integration::persistence::LocalEventStore;
    use releashd::test_support::integration::persistence::LocalEventStoreConfig;
    use releashd::test_support::integration::platform::build_workflow_services_with_gateways;
    use releashd::test_support::integration::platform::workflow_read;

    use releashd::test_support::integration::workflow::NoopWorkflowExternalEditorGateway;
    use releashd::test_support::integration::workflow::PassthroughManagedWorktreeGateway;
    use std::sync::Arc;

    #[test]
    pub fn test_診断read_usecase_適用済みdirectoryを診断する() {
        // Given
        let data = tempfile::tempdir().unwrap();
        let workflows = tempfile::tempdir().unwrap();
        let _store = LocalEventStore::open(LocalEventStoreConfig::production(
            data.path().to_path_buf(),
            std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
        ))
        .unwrap();
        std::fs::write(workflows.path().join("configured.yml"), "name: [").unwrap();
        let read = workflow_read(
            _store.clone(),
            data.path(),
            Some(workflows.path().to_path_buf()),
        );

        // When
        let report = read
            .diagnose_all(
                releashd::test_support::integration::workflow::WorkflowDiagnosticsTarget::AppliedConfigDirectory,
            )
            .unwrap();

        // Then
        assert!(report.workflow_summaries.contains_key("configured"));
    }

    async fn seed_b006_execution(store: &Arc<LocalEventStore>, workspace: &str) {
        use releashd::test_support::integration::workflow::ExecutionOrigin;
        use releashd::test_support::integration::workflow::ExecutionStatus;

        let execution_id = "00000000-0000-4000-8000-000000001491";
        releashd::test_support::integration::workflow::seed_canonical_execution(
            store,
            &releashd::test_support::integration::workflow::WorkflowExecutionSummary {
                execution_id: execution_id.to_string(),
                workflow_name: "B006 workflow".to_string(),
                status: ExecutionStatus::Running,
                worktree_path: workspace.to_string(),
                current_node: None,
                created_from: ExecutionOrigin::DesktopUi,
                started_at: 1.0,
                updated_at: 2.0,
                completed_at: None,
                error_reason: None,
                total_token_usage: Default::default(),
            },
            &[],
        )
        .await;
    }

    #[tokio::test]
    pub async fn b006_all_client_surfaces_use_the_production_workspace_query_contract() {
        // Given: the production composition root owns one live query object,
        // and the standalone loopback composition opens the same SQLite
        // authority through its read-only backend.
        let root = tempfile::tempdir().unwrap();
        let store = LocalEventStore::open(
            releashd::test_support::integration::persistence::LocalEventStoreConfig::production(
                root.path().to_path_buf(),
                std::sync::Arc::new(
                    releashd::test_support::integration::platform::RetryLimiter::new(),
                ),
            ),
        )
        .unwrap();
        let workspace = root
            .path()
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        seed_b006_execution(&store, &workspace).await;
        let (workflow, query) = build_workflow_services_with_gateways(
            Arc::new(releashd::test_support::integration::platform::FailureRecordStore::default()),
            root.path(),
            Arc::new(PassthroughManagedWorktreeGateway),
            Arc::new(NoopWorkflowExternalEditorGateway),
            store.clone(),
            None,
            None,
        );
        let tree = workflow.workspace_tree(&workspace).await.unwrap();
        // Then
        assert!(!tree.visible().roots().is_empty());
        assert_eq!(
            query.execution_summaries(None, None).await.unwrap().len(),
            1
        );
    }
}

#[tokio::test]
async fn test_workflow開始_本番runtime配線で隔離したfacet本文をsession初期指示に渡す() {
    // Given
    let retrying = releashd::test_support::integration::platform::test_retrying();
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().canonicalize().unwrap();
    let repo_path = root.join("repo");
    let repo_path = repo_path.to_str().unwrap();
    let repo = git2::Repository::init(repo_path).unwrap();
    crate::test_support_git::create_initial_commit(&repo);
    let repository = Arc::new(build_repository_usecase());
    let worktree = repository
        .create_worktree(repo_path, "feature", true, None)
        .unwrap();
    let data_dir = root.join("data");
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        data_dir.clone(),
        std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
    ))
    .unwrap();
    let workflows_dir = root.join("workflows");
    let facet_key = "isolated-runtime-facet-2012";
    let instruction = "instruction from isolated workflow directory";
    std::fs::create_dir_all(workflows_dir.join("instructions")).unwrap();
    std::fs::write(
        workflows_dir
            .join("instructions")
            .join(format!("{facet_key}.md")),
        instruction,
    )
    .unwrap();
    std::fs::write(workflows_dir.join("isolated-runtime.yml"), format!(
        "name: isolated-runtime\ndescription: isolated runtime fixture\nnodes:\n  main:\n    session:\n      provider: claude\n      facets:\n        instruction: {facet_key}\n"
    )).unwrap();
    let captured = root.join("initial-instruction");
    let executable = root.join("claude-fixture");
    let command = crate::agent_tui_fixture::fixture_process_shell_command(
        &crate::agent_tui_fixture::FixturePlan::new("isolated-runtime", vec![]),
    );
    std::fs::write(&executable, format!(
        "#!/bin/sh\nif [ \"$1\" = --version ]; then echo fixture; exit 0; fi\nfor argument do instruction=$argument; done\nprintf '%s' \"$instruction\" > '{}'\nexec env {command}\n",
        captured.to_str().unwrap().replace('\'', "'\"'\"'")
    )).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut configuration = ReleashConfig::default();
    configuration.agents.claude.cli_path = Some(executable.to_string_lossy().into_owned());
    configuration.app.last_repo_paths = vec![repo_path.to_owned()];
    let config = Arc::new(AppConfig::new(configuration, data_dir.join("releash.toml")));
    let publisher = releashd::test_support::integration::subscriptions::test_subscriptions();
    let terminal = releashd::test_support::integration::platform::build_terminal_surface_application_with_data_dir(data_dir.clone());
    let sessions = compose_agent_sessions(AgentSessionCompositionInput {
        config_mutation: Default::default(),
        hook_token: std::sync::Arc::<str>::from("hook-token"),
        launch_retention: releashd::test_support::integration::platform::run(
            releashd::test_support::integration::platform::delays(
                releashd::test_support::integration::platform::RETENTION,
            ),
        ),
        retrying: retrying.clone(),
        state_publisher: None,
        store: store.clone(),
        data_dir: data_dir.clone(),
        provider_executable_config: config.clone(),
        provider_executable_probe: Arc::new(LocalProviderExecutableProbeGateway::with_search_path(
            None,
        )),
        claude_config_dir: root.join("claude"),
        codex_home: root.join("codex"),
        cli_binary: "releash".into(),
        terminal: terminal.clone(),
        subscriptions: publisher.clone(),
    })
    .unwrap();
    let processes = Arc::new(WorkflowNodeProcesses::new(terminal));
    let (_, workspace_query) = build_workflow_services_with_repository_worktrees(
        Arc::new(releashd::test_support::integration::platform::FailureRecordStore::default()),
        data_dir,
        repository.clone(),
        config.clone(),
        store.clone(),
        processes.clone(),
        workflows_dir.clone(),
    );
    let (runtime, _) = build_workflow_runtime_usecase(
        retrying.clone(),
        WorkflowRuntimeDependencies {
            store: Some(store),
            config: Some(config.clone()),
            secrets: Some(config.clone()),
            state_changes: publisher.clone(),
        },
        WorkflowRuntimeCommandGatewayDeps {
            node_processes: processes,
            repository_usecase: repository.clone(),
            app_config: config,
            workspace_query: workspace_query.clone(),
            agent_session_launch: sessions.launch.clone(),
            agent_session_initial_instruction: sessions.initial_instruction.clone(),
            agent_session_lifecycle: sessions.lifecycle.clone(),
            provider_availability: sessions.availability_reader.clone(),
            isolated_worktrees: Arc::new(RepositoryIsolatedWorktreeGateway),
        },
        releashd::test_support::integration::daemon::serving(),
        workflows_dir.clone(),
    )
    .unwrap();
    let runtime = Arc::new(runtime);
    releashd::test_support::integration::platform::bind_agent_session_runtime(
        &sessions,
        runtime.clone(),
    );

    // When
    let execution_id = runtime
        .start_execution(
            releashd::test_support::integration::workflow::StartExecutionCommand {
                workflow_name: "isolated-runtime".into(),
                worktree_path: worktree.path.clone(),
                request: None,
                created_from:
                    releashd::test_support::integration::workflow::ExecutionOrigin::DesktopUi,
            },
        )
        .await
        .unwrap();
    let received = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            match std::fs::read_to_string(&captured) {
                Ok(value) if !value.is_empty() => break value,
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => panic!("read initial instruction: {error}"),
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await;
    let summaries = workspace_query
        .execution_summaries(None, None)
        .await
        .unwrap();
    runtime
        .abort_execution(
            releashd::test_support::integration::workflow::AbortExecutionCommand {
                execution_id,
                expected_node_name: None,
            },
        )
        .await
        .unwrap();

    // Then
    let received = received
        .unwrap_or_else(|error| panic!("initial instruction: {error}; executions: {summaries:?}"));
    assert_eq!(received.split_once("\n\n").unwrap().0, instruction);
}
