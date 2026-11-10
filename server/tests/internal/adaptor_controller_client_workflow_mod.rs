pub(crate) mod tests {
    use releashd::test_support::integration::workflow::{
        FacetFileQueryService, FacetQueryService, NodeKindName, WorkflowEvent,
    };

    use std::str::FromStr;

    use releashd::test_support::integration::platform::AppState;
    use releashd::test_support::integration::workflow::get_facet_inner;
    use releashd::test_support::integration::workflow::list_facets_inner;
    use releashd::test_support::integration::workflow::register_shared;
    use releashd::test_support::integration::workflow::FacetKind;
    use releashd::test_support::integration::workflow::WorkflowDefinitionContent as WorkflowDefinitionYaml;
    use std::sync::Arc;

    use releashd::test_support::integration::workflow::definition_FacetRefs as FacetRefs;

    use releashd::test_support::integration::workflow::NodeDefinition;
    use releashd::test_support::integration::workflow::NodeKind;

    use releashd::test_support::integration::workflow::SessionSpec;

    use tempfile::TempDir;

    pub(crate) struct WorkflowTestDependencies {
        pub(crate) client: releashd::test_support::integration::transport::ClientDependencies,
        pub(crate) repository_state:
            Arc<releashd::test_support::integration::platform::RepositoryStateService>,
    }

    const REQUIRED_WORKSPACE_EXECUTION_COMMANDS: &[&str] = &[
        "approve_workspace_node",
        "archive_workspace_workflow_execution",
        "restore_workspace_workflow_execution",
        "rename_workspace_session_node",
        "retry_workspace_node",
    ];

    #[test]
    pub fn workflow_command_registry_uses_execution_and_node_names() {
        let (app, _data_dir, _store) = make_read_only_app();
        let deps = &app.client;
        let mut dispatch =
            releashd::test_support::integration::transport::ClientCommandDispatch::new(
                releashd::test_support::integration::daemon::DaemonUsecase::test_with_repository(
                    releashd::test_support::integration::daemon::serving(),
                ),
            );
        register_shared(&mut dispatch, deps);
        let handles_command = |command| dispatch.contains(command);

        let mut workspace_dispatch =
            releashd::test_support::integration::transport::ClientCommandDispatch::new(
                releashd::test_support::integration::daemon::DaemonUsecase::test_with_repository(
                    releashd::test_support::integration::daemon::serving(),
                ),
            );
        releashd::test_support::integration::workspace::register_shared(
            &mut workspace_dispatch,
            deps,
        );
        for command in REQUIRED_WORKSPACE_EXECUTION_COMMANDS {
            assert!(
                workspace_dispatch.contains(command),
                "missing workspace workflow command: {command}"
            );
        }

        assert!(handles_command("start_workflow"));
        assert!(!handles_command("stop_workflow"));
        assert!(!handles_command("resume_workflow"));
        assert!(handles_command("workflow_submit_output"));
        assert!(handles_command("diagnose_workflow_directory"));
        assert!(!handles_command("get_git_status"));
    }

    //
    // テンポラリディレクトリ上で検証する。

    const THREE_KINDS: [(&str, &str); 3] = [
        ("policy", "policies"),
        ("knowledge", "knowledge"),
        ("instruction", "instructions"),
    ];

    /// 3 種それぞれのディレクトリを作成し、各種に既存の非ビルトインキー（"sample-{kind}"）を配置する。
    fn setup_tmp_facets_base() -> tempfile::TempDir {
        let tmp = tempfile::TempDir::new().unwrap();
        for (_kind, dir_name) in THREE_KINDS {
            let dir = tmp.path().join(dir_name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(format!("sample-{dir_name}.md")), "SAMPLE_BODY").unwrap();
        }
        tmp
    }

    fn key_for(kind: &str) -> String {
        let (_, dir) = THREE_KINDS.iter().find(|(k, _)| *k == kind).unwrap();
        format!("sample-{dir}")
    }

    #[test]
    pub fn list_facets_inner_reaches_listing_path_for_each_kind() {
        let tmp = setup_tmp_facets_base();
        for (kind, _) in THREE_KINDS {
            let listed = list_facets_inner(kind, tmp.path()).unwrap();
            assert!(
                listed.iter().any(|k| k == &key_for(kind)),
                "list_facets({kind}) must include the seeded key"
            );
        }
    }

    #[test]
    pub fn list_facets_inner_rejects_unknown_without_io() {
        let tmp = setup_tmp_facets_base();
        for bad in ["unknown"] {
            let result = list_facets_inner(bad, tmp.path());
            assert!(result.is_err(), "list_facets({bad}) must be rejected");
        }
    }

    #[test]
    pub fn get_facet_inner_reaches_load_path_for_each_kind() {
        let tmp = setup_tmp_facets_base();
        for (kind, _) in THREE_KINDS {
            let body = get_facet_inner(kind, &key_for(kind), tmp.path()).unwrap();
            assert_eq!(body, "SAMPLE_BODY", "get_facet({kind}) body mismatch");
        }
    }

    #[test]
    pub fn get_facet_inner_rejects_unknown_without_io() {
        let tmp = setup_tmp_facets_base();
        for bad in ["unknown"] {
            let result = get_facet_inner(bad, "sample-policies", tmp.path());
            assert!(result.is_err(), "get_facet({bad}) must be rejected");
        }
    }

    #[test]
    pub fn list_facet_summaries_inner_lists_for_each_kind() {
        // Given
        let tmp = setup_tmp_facets_base();
        for (kind, _) in THREE_KINDS {
            // When
            let summaries = FacetFileQueryService::new(tmp.path())
                .list_facet_summaries(FacetKind::from_str(kind).unwrap())
                .unwrap();
            // Then
            assert!(
                summaries.iter().any(|s| s.key == key_for(kind)),
                "list_facet_summaries({kind}) must include the seeded key"
            );
        }
    }

    #[test]
    pub fn list_facet_summaries_inner_rejects_unknown_without_io() {
        // Given
        let tmp = setup_tmp_facets_base();
        let bad = "unknown";
        // When
        let result = FacetKind::from_str(bad)
            .and_then(|kind| FacetFileQueryService::new(tmp.path()).list_facet_summaries(kind));
        // Then
        assert!(
            result.is_err(),
            "list_facet_summaries({bad}) must be rejected"
        );
    }

    // ---- duplicate logic tests ----
    // These test the core duplicate logic using storage/facet/builtin functions directly,
    // mirroring what the ClientService handlers do inside spawn_blocking.

    fn make_test_workflow(name: &str) -> WorkflowDefinitionYaml {
        WorkflowDefinitionYaml {
            name: name.to_string(),
            description: "test workflow".to_string(),
            schemas: Default::default(),
            nodes: vec![NodeDefinition {
                name: "main".to_string(),
                kind: NodeKind::Session(SessionSpec {
                    facets: FacetRefs {
                        instruction: Some("review-acceptance".to_string()),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
                ..NodeDefinition::default()
            }],
            entry: "main".to_string(),
        }
    }

    #[test]
    pub fn duplicate_facet_normal_case() {
        let tmp = TempDir::new().unwrap();
        let base_dir = tmp.path();
        let kind = FacetKind::Policy;
        releashd::test_support::integration::workflow::save_facet(
            kind,
            "source-facet",
            "# Source Policy\nContent here",
            base_dir,
        )
        .unwrap();

        let new_key = "copied-facet";
        releashd::test_support::integration::workflow::validate_facet_key(new_key).unwrap();

        let existing =
            releashd::test_support::integration::workflow::list_facets(kind, base_dir).unwrap();
        assert!(!existing.contains(&new_key.to_string()));

        let content = releashd::test_support::integration::workflow::load_facet(
            kind,
            "source-facet",
            base_dir,
        )
        .unwrap();
        releashd::test_support::integration::workflow::save_facet(
            kind, new_key, &content, base_dir,
        )
        .unwrap();

        let loaded =
            releashd::test_support::integration::workflow::load_facet(kind, new_key, base_dir)
                .unwrap();
        assert_eq!(loaded, "# Source Policy\nContent here");
    }

    #[test]
    pub fn duplicate_facet_rejects_existing_key() {
        let tmp = TempDir::new().unwrap();
        let base_dir = tmp.path();
        let kind = FacetKind::Policy;
        releashd::test_support::integration::workflow::save_facet(
            kind, "my-facet", "content", base_dir,
        )
        .unwrap();

        let existing =
            releashd::test_support::integration::workflow::list_facets(kind, base_dir).unwrap();

        // Act: Simulate the duplicate check from the command
        let new_key = "my-facet";
        let result: Result<(), String> = if existing.contains(&new_key.to_string()) {
            Err(format!("ファセット '{new_key}' は既に存在します"))
        } else {
            Ok(())
        };

        // Assert
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("既に存在します"));
    }

    // ---- Builtin guard tests ----

    // ---- save_workflow rename duplicate check ----

    // ---- save_facet is_new duplicate check ----

    #[test]
    pub fn save_facet_is_new_rejects_existing_key() {
        let tmp = TempDir::new().unwrap();
        let base_dir = tmp.path();
        let kind = FacetKind::Policy;

        // Create an existing facet
        releashd::test_support::integration::workflow::save_facet(
            kind,
            "existing-facet",
            "content",
            base_dir,
        )
        .unwrap();

        // Simulate is_new=true with duplicate key
        let existing =
            releashd::test_support::integration::workflow::list_facets(kind, base_dir).unwrap();

        // Act: Simulate the is_new duplicate check from the command
        let is_new = true;
        let key = "existing-facet";
        let result: Result<(), String> = if is_new && existing.contains(&key.to_string()) {
            Err(format!("ファセット '{key}' は既に存在します"))
        } else {
            Ok(())
        };

        // Assert
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("既に存在します"));
    }

    // ---- delete_workflow builtin guard ----

    // ---- open_workflow_in_editor builtin guard ----

    // ---- duplicate_facet rejects builtin key ----

    #[test]
    pub fn duplicate_facet_rejects_builtin_key() {
        let builtin_keys = releashd::test_support::integration::workflow::list_builtin_facet_keys(
            FacetKind::Policy,
        );
        if let Some(key) = builtin_keys.first() {
            // list_facets includes builtins, so duplicate to a builtin key would be caught
            // by the existing.contains(&new_key) check
            assert!(
                releashd::test_support::integration::workflow::get_builtin_facet(
                    FacetKind::Policy,
                    key
                )
                .is_some()
            );

            // Verify list_facets returns builtin keys (which is used for duplicate check)
            let tmp = TempDir::new().unwrap();
            let base_dir = tmp.path();
            let existing = releashd::test_support::integration::workflow::list_facets(
                FacetKind::Policy,
                base_dir,
            )
            .unwrap();
            assert!(
                existing.contains(&key.to_string()),
                "list_facets should include builtin key '{key}'"
            );
        }
    }

    // ---- save_workflow create/update/rename logic ----

    // ---- [05] read-only Execution 観測 API: ClientService 境界の直接テスト ----

    pub(crate) fn make_read_only_app() -> (
        WorkflowTestDependencies,
        std::path::PathBuf,
        Arc<releashd::test_support::integration::persistence::LocalEventStore>,
    ) {
        make_read_only_app_with_terminal(Arc::new(
            releashd::test_support::integration::platform::build_terminal_surface_application_for_tests(),
        ))
    }

    pub(crate) fn make_read_only_app_with_terminal(
        terminal_surface: Arc<
            releashd::test_support::integration::terminal::TerminalSurfaceApplication,
        >,
    ) -> (
        WorkflowTestDependencies,
        std::path::PathBuf,
        Arc<releashd::test_support::integration::persistence::LocalEventStore>,
    ) {
        let data_dir =
            std::env::temp_dir().join(format!("releash-command-adapter-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&data_dir).unwrap();
        let config_mutation = Arc::new(parking_lot::Mutex::new(()));
        let app_config = Arc::new(
            releashd::test_support::integration::settings::AppConfig::new(
                releashd::test_support::integration::settings::ReleashConfig::default(),
                data_dir.join("config.toml"),
            ),
        );
        let config_repository: Arc<
            dyn releashd::test_support::integration::repository::AppConfigRepository,
        > = app_config.clone();
        let git_usecase =
            Arc::new(releashd::test_support::integration::platform::build_repository_usecase());
        let notion_usecase = Arc::new(
            releashd::test_support::integration::platform::NotionUsecase::new(
                app_config.clone(),
                app_config.clone(),
                Arc::new(
                    releashd::test_support::integration::platform::NotionApiGatewayImpl::new(
                        releashd::test_support::integration::platform::shared_limiter(),
                    ),
                ),
                config_mutation.clone(),
            ),
        );
        let repo_paths_gateway = Arc::new(
            releashd::test_support::integration::repository::RepoPathsGateway::new(
                <releashd::test_support::integration::repository::SharedRepoPaths>::default(),
                config_repository.clone(),
            ),
        );
        let repo_paths_usecase = Arc::new(
            releashd::test_support::integration::platform::RepoPathsUsecase::new(
                repo_paths_gateway.clone(),
                repo_paths_gateway,
                releashd::test_support::integration::subscriptions::test_subscriptions(),
                config_mutation.clone(),
            ),
        );
        let code_usecase =
            Arc::new(releashd::test_support::integration::platform::build_code_usecase());
        let repository_scanner = Arc::new(
            releashd::test_support::integration::repository::DefaultRepositoryScanner::new(
                git_usecase.clone(),
                Arc::new(
                    releashd::test_support::integration::settings::repository_config_usecase(),
                ),
            ),
        );
        let repository_state_repository = Arc::new(
            releashd::test_support::integration::repository::RepositoryStateRepositoryGateway::new(
                git_usecase.clone(),
            ),
        );
        let repository_state = Arc::new(
            releashd::test_support::integration::platform::RepositoryStateService::new(
                repository_state_repository,
                repository_scanner,
                releashd::test_support::integration::subscriptions::test_subscriptions(),
                Arc::new(releashd::test_support::integration::platform::NoopRepositoryStateWatcher),
                Arc::new(
                    releashd::test_support::integration::platform::TestRepositoryStateWorkerRuntime,
                ),
                Arc::new(
                    releashd::test_support::integration::platform::IdentityWorktreePathNormalizer,
                ),
                releashd::test_support::integration::subscriptions::repository_driver(),
            ),
        );
        let review_usecase = Arc::new(
            releashd::test_support::integration::platform::ReviewUsecase::new(
                repository_state.clone(),
                code_usecase.clone(),
            ),
        );
        let local_event_store =
            releashd::test_support::integration::persistence::LocalEventStore::open(
                releashd::test_support::integration::persistence::LocalEventStoreConfig::production(
                    data_dir.clone(),
                    std::sync::Arc::new(
                        releashd::test_support::integration::platform::RetryLimiter::new(),
                    ),
                ),
            )
            .unwrap();
        let (workflow_usecase, _) =
            releashd::test_support::integration::platform::build_workflow_services_with_repository_worktrees(
                Arc::new(releashd::test_support::integration::platform::FailureRecordStore::default()),
                git_usecase.clone(),
                config_repository.clone(),
                releashd::test_support::integration::workspace::ExecutionReads::new(local_event_store.clone()),
                Arc::new(
                    releashd::test_support::integration::workflow::WorkflowNodeProcesses::default(
                    ),
                ),
                releashd::test_support::integration::platform::WorkflowEditorDependencies { settings: Arc::new(releashd::test_support::integration::workflow::NoopEditorSettingsGateway), launcher: Arc::new(releashd::test_support::integration::workflow::NoopEditorLauncherGateway), workflows_dir: data_dir.join("workflows") },
            );
        let workflow_usecase = Arc::new(workflow_usecase);
        let git_host_usecase =
            Arc::new(releashd::test_support::integration::platform::build_git_host_usecase());
        let workspace_list = Arc::new(
            releashd::test_support::integration::platform::build_workspace_list_usecase(
                repo_paths_usecase.clone(),
                git_usecase.clone(),
                repository_state.clone(),
                workflow_usecase.clone(),
                git_host_usecase.clone(),
            ),
        );
        let app_state = AppState {
            workspace_list,
            git_usecase: git_usecase.clone(),
            repository_config_usecase: Arc::new(
                releashd::test_support::integration::settings::repository_config_usecase(),
            ),
            repo_paths_usecase,
            code_usecase,
            review_usecase,
            notion_usecase,
            workflow_usecase,
            terminal_surface,
            git_host_usecase,
        };
        let mut client = releashd::test_support::integration::transport::build_client_dependencies(
            data_dir.clone(),
        );
        client.app_state = Some(app_state);
        client.config_repository = Some(config_repository.clone());
        client.config_mutation = config_mutation.clone();
        client.app_config_usecase = Some(Arc::new(
            releashd::test_support::integration::settings::AppConfigUsecase::new(
                config_repository,
                config_mutation.clone(),
            ),
        ));
        client.watcher = Arc::new(releashd::test_support::integration::platform::WatcherUsecase::new(
            Some(repository_state.clone()),
            Arc::new(
                releashd::test_support::integration::repository::FileWatcherGateway::new(
                    Arc::new(releashd::test_support::integration::platform::FileWatcherManager::default()),
                ),
            ),
        ));
        (
            WorkflowTestDependencies {
                client,
                repository_state,
            },
            data_dir,
            local_event_store,
        )
    }

    /// [05] worktree-scoped 認可境界のテスト用 fixture: 実 git repo + worktree を作り、
    /// `AppConfig.last_repo_paths` に親 repo を登録する。戻り値は test app / engine /
    /// data_dir / canonical worktree path / TempDir guards（lifetime 保持用）。
    fn make_read_only_app_with_managed_worktree() -> (
        WorkflowTestDependencies,
        std::path::PathBuf,
        Arc<releashd::test_support::integration::persistence::LocalEventStore>,
        String,
        TempDir,
        TempDir,
    ) {
        let (app, data_dir, local_event_store) = make_read_only_app();
        let repo_parent = TempDir::new().unwrap();
        let worktree_parent = TempDir::new().unwrap();
        let repo_path = repo_parent.path().join("repo");
        std::fs::create_dir(&repo_path).unwrap();
        let repo = git2::Repository::init(&repo_path).unwrap();
        std::fs::write(repo_path.join("README.md"), "test\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new("README.md")).unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let sig = git2::Signature::now("Test", "test@example.com").unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
            .unwrap();
        let worktree_path = worktree_parent.path().join("managed-wt");
        repo.worktree("managed-wt", &worktree_path, None).unwrap();
        let canonical = worktree_path.canonicalize().unwrap();
        let canonical_str = canonical.to_string_lossy().to_string();
        let config_repository = app.client.config_repository.as_ref().unwrap();
        let mut config = config_repository.load().unwrap();
        assert!(config.add_repo_path(&repo_path.to_string_lossy()));
        config_repository.save(config).unwrap();
        (
            app,
            data_dir,
            local_event_store,
            canonical_str,
            repo_parent,
            worktree_parent,
        )
    }

    #[test]
    fn test_managed_worktree同期解決_repositoryの登録を削除するとworktreeを拒否する() {
        // Given
        let (app, _data_dir, _store, worktree_path, repo_parent, _worktree_parent) =
            make_read_only_app_with_managed_worktree();
        let workflow = &app.client.app_state.as_ref().unwrap().workflow_usecase;
        assert_eq!(
            workflow.resolve_worktree_path(&worktree_path).unwrap(),
            worktree_path
        );
        let config_repository = app.client.config_repository.as_ref().unwrap();
        let mut config = config_repository.load().unwrap();
        assert!(config.remove_repo_path(&repo_parent.path().join("repo").to_string_lossy()));
        config_repository.save(config).unwrap();

        // When
        let result = workflow.resolve_worktree_path(&worktree_path);

        // Then
        assert!(matches!(
            result,
            Err(releashd::test_support::integration::workflow::WorkflowError::External(message))
                if message.contains("not a configured git worktree")
        ));
        assert!(std::path::Path::new(&worktree_path).exists());
    }

    /// Spec [05] Rule: 指定 execution の現在 state を観測する（event log からの純粋投影）。
    /// 観測結果の露出範囲境界: live runtime registry / OpenTabRegistry 由来の runtime_active /
    /// tab_open enrichment は含めない（戻り値の runtime_states は空）。
    #[tokio::test]
    pub async fn execution_projection_preserves_state_without_runtime_enrichment() {
        // Given
        let (app, _data_dir, local_event_store, worktree_path, _r, _w) =
            make_read_only_app_with_managed_worktree();
        let execution_id = uuid::Uuid::from_bytes([5; 16]).to_string();
        releashd::test_support::integration::workflow::append_canonical_events(
            &local_event_store,
            &[
                WorkflowEvent::ExecutionStarted {
                    repository_root: None,
                    execution_id: execution_id.clone(),
                    workflow_name: "adapter-boundary".to_string(),
                    worktree_path: worktree_path.clone(),
                    request: String::new(),
                    definition: make_test_workflow("adapter-boundary"),
                    timestamp: 500.0,
                },
                WorkflowEvent::NodeStarted {
                    worktree: None,
                    execution_id: execution_id.clone(),
                    node_execution_id: "ne-main-1".to_string(),
                    node_name: "main".to_string(),
                    kind: NodeKindName::Session,
                    attempt: 1,
                    parent: None,
                    timestamp: 500.0,
                },
            ],
        )
        .await
        .unwrap();

        let read = app
            .client
            .app_state
            .as_ref()
            .unwrap()
            .workflow_usecase
            .clone();
        // When
        let view = read
            .get_execution_state(&execution_id)
            .await
            .unwrap()
            .unwrap();
        // Then
        assert_eq!(view.id, execution_id);
        assert_eq!(view.node_executions.len(), 1);
        assert_eq!(view.node_executions[0].node_name, "main");
        assert!(view.node_executions[0].session_id.is_none());

        // 存在しない execution_id は Ok(None)。
        let missing = read
            .get_execution_state(&uuid::Uuid::from_bytes([97; 16]).to_string())
            .await
            .unwrap();
        assert!(missing.is_none());
    }
}

#[tokio::test]
async fn test_定義フォルダrpc_登録したhandlerがusecaseを呼びフォルダを用意する() {
    use releashd::test_support::integration::{transport, workflow};
    // Given
    let (app, data_dir, _store) = tests::make_read_only_app();
    let folder = data_dir.join("workflows");
    assert!(!folder.exists());
    let mut dispatch = transport::ClientCommandDispatch::new(app.client.daemon.clone());
    workflow::register_shared(&mut dispatch, &app.client);
    assert!(dispatch.contains("open_workflow_directory_in_editor"));
    // When
    use releashd::test_support::integration::wire;
    let command = wire::command_request::Command::OpenWorkflowDirectoryInEditor(Default::default());
    let result = dispatch.dispatch(command).await.unwrap();
    // Then
    assert!(matches!(
        result,
        wire::command_result::Command::OpenWorkflowDirectoryInEditor(_)
    ));
    assert!(folder.is_dir());
}

#[tokio::test]
async fn test_facet編集rpc_正規名だけをdomainの種類へ変換する() {
    use releashd::test_support::integration::{client_protocol::wire, transport, workflow};
    // Given
    let (app, data_dir, _store) = tests::make_read_only_app();
    std::fs::create_dir_all(data_dir.join("workflows/policies")).unwrap();
    std::fs::write(data_dir.join("workflows/policies/custom.md"), "# Custom").unwrap();
    let mut dispatch = transport::ClientCommandDispatch::new(app.client.daemon.clone());
    workflow::register_shared(&mut dispatch, &app.client);
    // When / Then
    for kind in ["policy", "policies", "unknown"] {
        let result = dispatch
            .dispatch(wire::command_request::Command::OpenFacetInEditor(
                wire::OpenFacetInEditorRequest {
                    kind: Some(kind.into()),
                    key: Some("custom".into()),
                },
            ))
            .await;
        if kind == "policy" {
            assert!(result.is_ok());
        } else {
            assert!(result.is_err());
        }
    }
}
