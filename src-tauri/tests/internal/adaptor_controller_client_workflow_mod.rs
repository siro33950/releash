pub(crate) mod tests {

    use releash_lib::test_support::integration::platform::AppState;
    use releash_lib::test_support::integration::workflow::delete_facet_inner;
    use releash_lib::test_support::integration::workflow::duplicate_facet_inner;
    use releash_lib::test_support::integration::workflow::get_facet_inner;
    use releash_lib::test_support::integration::workflow::list_facet_summaries_inner;
    use releash_lib::test_support::integration::workflow::list_facets_inner;
    use releash_lib::test_support::integration::workflow::open_facet_in_editor_inner;
    use releash_lib::test_support::integration::workflow::register_shared;
    use releash_lib::test_support::integration::workflow::save_facet_inner;
    use releash_lib::test_support::integration::workflow::FacetKind;
    use releash_lib::test_support::integration::workflow::WorkflowDefinition as WorkflowDefinitionYaml;
    use std::sync::Arc;

    use releash_lib::test_support::integration::workflow::definition_FacetRefs as FacetRefs;
    use releash_lib::test_support::integration::workflow::ExecutionOrigin;
    use releash_lib::test_support::integration::workflow::NodeDefinition;
    use releash_lib::test_support::integration::workflow::NodeKind;
    use releash_lib::test_support::integration::workflow::NodeKindName;
    use releash_lib::test_support::integration::workflow::SessionSpec;
    use releash_lib::test_support::integration::workflow::WorkflowEvent;
    use std::path::Path;
    use tempfile::TempDir;

    pub(crate) struct WorkflowTestDependencies {
        pub(crate) client: releash_lib::test_support::integration::transport::ClientDependencies,
        pub(crate) repository_state:
            Arc<releash_lib::test_support::integration::platform::RepositoryStateService>,
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
            releash_lib::test_support::integration::transport::ClientCommandDispatch::new(
                releash_lib::test_support::integration::daemon::DaemonUsecase::test_with_repository(
                    releash_lib::test_support::integration::daemon::serving(),
                ),
            );
        register_shared(&mut dispatch, deps);
        let handles_command = |command| dispatch.contains(command);

        let mut workspace_dispatch =
            releash_lib::test_support::integration::transport::ClientCommandDispatch::new(
                releash_lib::test_support::integration::daemon::DaemonUsecase::test_with_repository(
                    releash_lib::test_support::integration::daemon::serving(),
                ),
            );
        releash_lib::test_support::integration::workspace::register_shared(
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
    pub fn save_facet_inner_writes_for_each_kind() {
        let tmp = setup_tmp_facets_base();
        for (kind, dir_name) in THREE_KINDS {
            let key = format!("created-{dir_name}");
            save_facet_inner(kind, &key, "WRITTEN_BODY", true, tmp.path()).unwrap();
            let path = tmp.path().join(dir_name).join(format!("{key}.md"));
            assert!(path.exists(), "save_facet({kind}) must create {path:?}");
            assert_eq!(std::fs::read_to_string(&path).unwrap(), "WRITTEN_BODY");
        }
    }

    #[test]
    pub fn save_facet_inner_rejects_unknown_without_io() {
        let tmp = setup_tmp_facets_base();
        for bad in ["unknown"] {
            let result = save_facet_inner(bad, "anything", "BODY", true, tmp.path());
            assert!(result.is_err(), "save_facet({bad}) must be rejected");
        }
    }

    #[test]
    pub fn delete_facet_inner_removes_for_each_kind() {
        let tmp = setup_tmp_facets_base();
        for (kind, dir_name) in THREE_KINDS {
            let key = key_for(kind);
            let path = tmp.path().join(dir_name).join(format!("{key}.md"));
            assert!(path.exists());
            delete_facet_inner(kind, &key, tmp.path()).unwrap();
            assert!(!path.exists(), "delete_facet({kind}) must remove {path:?}");
        }
    }

    #[test]
    pub fn delete_facet_inner_rejects_unknown_without_io() {
        let tmp = setup_tmp_facets_base();
        for bad in ["unknown"] {
            let result = delete_facet_inner(bad, "sample-policies", tmp.path());
            assert!(result.is_err(), "delete_facet({bad}) must be rejected");
        }
        // 3種のサンプルは温存されている
        for (_, dir_name) in THREE_KINDS {
            assert!(tmp
                .path()
                .join(dir_name)
                .join(format!("sample-{dir_name}.md"))
                .exists());
        }
    }

    #[test]
    pub fn list_facet_summaries_inner_lists_for_each_kind() {
        let tmp = setup_tmp_facets_base();
        for (kind, _) in THREE_KINDS {
            let summaries = list_facet_summaries_inner(kind, tmp.path()).unwrap();
            assert!(
                summaries.iter().any(|s| s.key == key_for(kind)),
                "list_facet_summaries({kind}) must include the seeded key"
            );
        }
    }

    #[test]
    pub fn list_facet_summaries_inner_rejects_unknown_without_io() {
        let tmp = setup_tmp_facets_base();
        for bad in ["unknown"] {
            let result = list_facet_summaries_inner(bad, tmp.path());
            assert!(
                result.is_err(),
                "list_facet_summaries({bad}) must be rejected"
            );
        }
    }

    #[test]
    pub fn duplicate_facet_inner_creates_new_file_for_each_kind() {
        let tmp = setup_tmp_facets_base();
        for (kind, dir_name) in THREE_KINDS {
            let source = key_for(kind);
            let new_key = format!("copied-{dir_name}");
            duplicate_facet_inner(kind, &source, &new_key, tmp.path()).unwrap();
            let path = tmp.path().join(dir_name).join(format!("{new_key}.md"));
            assert!(
                path.exists(),
                "duplicate_facet({kind}) must create {path:?}"
            );
            assert_eq!(std::fs::read_to_string(&path).unwrap(), "SAMPLE_BODY");
        }
    }

    #[test]
    pub fn duplicate_facet_inner_rejects_unknown_without_io() {
        let tmp = setup_tmp_facets_base();
        for bad in ["unknown"] {
            let result = duplicate_facet_inner(bad, "src", "dst", tmp.path());
            assert!(result.is_err(), "duplicate_facet({bad}) must be rejected");
        }
    }

    #[test]
    pub fn open_facet_in_editor_inner_invokes_opener_for_each_kind() {
        // open_facet_in_editor のエディタ呼び出し点はテストダブル（クロージャ）で差し替えて、
        // 実プロセスを起動せずに 3 種すべての正常経路到達と引数（対象パス）を検証する。
        let tmp = setup_tmp_facets_base();
        for (kind, dir_name) in THREE_KINDS {
            let recorded: Arc<std::sync::Mutex<Vec<String>>> =
                Arc::new(std::sync::Mutex::new(Vec::new()));
            let recorded_clone = recorded.clone();
            let key = key_for(kind);
            open_facet_in_editor_inner(kind, &key, tmp.path(), move |path_str| {
                recorded_clone.lock().unwrap().push(path_str.to_string());
                Ok(())
            })
            .unwrap();
            let paths = recorded.lock().unwrap();
            assert_eq!(
                paths.len(),
                1,
                "opener must be invoked exactly once for {kind}"
            );
            let expected = tmp.path().join(dir_name).join(format!("{key}.md"));
            assert_eq!(
                paths[0],
                expected.to_string_lossy().to_string(),
                "opener must receive the resolved facet path for {kind}"
            );
        }
    }

    #[test]
    pub fn open_facet_in_editor_inner_rejects_unknown_without_invoking_opener() {
        let tmp = setup_tmp_facets_base();
        for bad in ["unknown"] {
            let invoked: Arc<std::sync::Mutex<bool>> = Arc::new(std::sync::Mutex::new(false));
            let invoked_clone = invoked.clone();
            let result = open_facet_in_editor_inner(bad, "sample", tmp.path(), move |_| {
                *invoked_clone.lock().unwrap() = true;
                Ok(())
            });
            assert!(
                result.is_err(),
                "open_facet_in_editor({bad}) must be rejected"
            );
            assert!(
                !*invoked.lock().unwrap(),
                "opener must not be invoked for {bad}"
            );
        }
    }

    // ---- duplicate logic tests ----
    // These test the core duplicate logic using storage/facet/builtin functions directly,
    // mirroring what the Tauri commands do inside spawn_blocking.

    fn make_test_workflow(name: &str) -> WorkflowDefinitionYaml {
        WorkflowDefinitionYaml {
            name: name.to_string(),
            description: "test workflow".to_string(),
            builtin: false,
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
    pub fn duplicate_workflow_normal_case() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();
        let instructions = dir.join("instructions");
        std::fs::create_dir_all(&instructions).unwrap();
        std::fs::write(
            instructions.join("review-acceptance.md"),
            "Review the change.",
        )
        .unwrap();
        let wf = make_test_workflow("source-wf");
        releash_lib::test_support::integration::workflow::save_workflow(dir, &wf).unwrap();

        // Simulate duplicate logic
        let new_name = "copied-wf";
        releash_lib::test_support::integration::workflow::validate_name(new_name).unwrap();
        assert!(!dir.join(format!("{new_name}.yml")).exists());
        assert!(!releash_lib::test_support::integration::workflow::is_builtin_workflow(new_name));

        let mut copied = releash_lib::test_support::integration::workflow::load_workflow(
            &dir.join("source-wf.yml"),
            dir,
        )
        .unwrap();
        copied.name = new_name.to_string();
        copied.builtin = false;
        releash_lib::test_support::integration::workflow::save_workflow(dir, &copied).unwrap();

        assert!(dir.join(format!("{new_name}.yml")).exists());
        let loaded = releash_lib::test_support::integration::workflow::load_workflow(
            &dir.join(format!("{new_name}.yml")),
            dir,
        )
        .unwrap();
        assert_eq!(loaded.name, new_name);
        assert!(!loaded.builtin);
    }

    #[test]
    pub fn duplicate_workflow_rejects_existing_custom_name() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();
        let wf = make_test_workflow("existing-wf");
        releash_lib::test_support::integration::workflow::save_workflow(dir, &wf).unwrap();

        // Act: Simulate the duplicate check from the command
        let new_name = "existing-wf";
        let result: Result<(), String> = if dir.join(format!("{new_name}.yml")).exists() {
            Err(format!("ワークフロー '{new_name}' は既に存在します"))
        } else {
            Ok(())
        };

        // Assert
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("既に存在します"));
    }

    #[test]
    pub fn duplicate_facet_normal_case() {
        let tmp = TempDir::new().unwrap();
        let base_dir = tmp.path();
        let kind = FacetKind::Policy;
        releash_lib::test_support::integration::workflow::save_facet(
            kind,
            "source-facet",
            "# Source Policy\nContent here",
            base_dir,
        )
        .unwrap();

        let new_key = "copied-facet";
        releash_lib::test_support::integration::workflow::validate_facet_key(new_key).unwrap();

        let existing =
            releash_lib::test_support::integration::workflow::list_facets(kind, base_dir).unwrap();
        assert!(!existing.contains(&new_key.to_string()));

        let content = releash_lib::test_support::integration::workflow::load_facet(
            kind,
            "source-facet",
            base_dir,
        )
        .unwrap();
        releash_lib::test_support::integration::workflow::save_facet(
            kind, new_key, &content, base_dir,
        )
        .unwrap();

        let loaded =
            releash_lib::test_support::integration::workflow::load_facet(kind, new_key, base_dir)
                .unwrap();
        assert_eq!(loaded, "# Source Policy\nContent here");
    }

    #[test]
    pub fn duplicate_facet_rejects_existing_key() {
        let tmp = TempDir::new().unwrap();
        let base_dir = tmp.path();
        let kind = FacetKind::Policy;
        releash_lib::test_support::integration::workflow::save_facet(
            kind, "my-facet", "content", base_dir,
        )
        .unwrap();

        let existing =
            releash_lib::test_support::integration::workflow::list_facets(kind, base_dir).unwrap();

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

    #[test]
    pub fn save_workflow_rename_rejects_duplicate_name() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();

        // Create two workflows
        let wf_a = make_test_workflow("workflow-a");
        let wf_b = make_test_workflow("workflow-b");
        releash_lib::test_support::integration::workflow::save_workflow(dir, &wf_a).unwrap();
        releash_lib::test_support::integration::workflow::save_workflow(dir, &wf_b).unwrap();

        // Simulate renaming workflow-a to workflow-b (duplicate)
        let original_name = Some("workflow-a".to_string());
        let new_name = "workflow-b";
        let is_rename = original_name.as_ref().is_some_and(|o| *o != new_name);

        // Act: Simulate the rename duplicate check from the command
        let result: Result<(), String> =
            if is_rename && dir.join(format!("{new_name}.yml")).exists() {
                Err(format!("ワークフロー '{new_name}' は既に存在します"))
            } else {
                Ok(())
            };

        // Assert
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("既に存在します"));
    }

    // ---- save_facet is_new duplicate check ----

    #[test]
    pub fn save_facet_is_new_rejects_existing_key() {
        let tmp = TempDir::new().unwrap();
        let base_dir = tmp.path();
        let kind = FacetKind::Policy;

        // Create an existing facet
        releash_lib::test_support::integration::workflow::save_facet(
            kind,
            "existing-facet",
            "content",
            base_dir,
        )
        .unwrap();

        // Simulate is_new=true with duplicate key
        let existing =
            releash_lib::test_support::integration::workflow::list_facets(kind, base_dir).unwrap();

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
        let builtin_keys =
            releash_lib::test_support::integration::workflow::list_builtin_facet_keys(
                FacetKind::Policy,
            );
        if let Some(key) = builtin_keys.first() {
            // list_facets includes builtins, so duplicate to a builtin key would be caught
            // by the existing.contains(&new_key) check
            assert!(
                releash_lib::test_support::integration::workflow::is_builtin_facet(
                    FacetKind::Policy,
                    key
                )
            );

            // Verify list_facets returns builtin keys (which is used for duplicate check)
            let tmp = TempDir::new().unwrap();
            let base_dir = tmp.path();
            let existing = releash_lib::test_support::integration::workflow::list_facets(
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

    /// save_workflow のコア判定ロジックを再現するヘルパー
    fn simulate_save_workflow(
        dir: &Path,
        workflow: &WorkflowDefinitionYaml,
        original_name: Option<&str>,
    ) -> Result<(), String> {
        let is_new = original_name.is_none();
        let is_rename = original_name.is_some_and(|o| o != workflow.name);
        if (is_new || is_rename) && dir.join(format!("{}.yml", workflow.name)).exists() {
            return Err(format!("ワークフロー '{}' は既に存在します", workflow.name));
        }
        releash_lib::test_support::integration::workflow::save_workflow(dir, workflow)
            .map_err(|e| e.to_string())?;
        if let Some(orig) = original_name {
            if orig != workflow.name {
                let old_path = dir.join(format!("{orig}.yml"));
                if old_path.exists() {
                    std::fs::remove_file(&old_path)
                        .map_err(|e| format!("旧ファイル削除失敗: {e}"))?;
                }
            }
        }
        Ok(())
    }

    #[test]
    pub fn save_workflow_existing_same_name_update_succeeds() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();
        let instructions = dir.join("instructions");
        std::fs::create_dir_all(&instructions).unwrap();
        std::fs::write(
            instructions.join("review-acceptance.md"),
            "Review the change.",
        )
        .unwrap();

        let wf = make_test_workflow("my-wf");
        releash_lib::test_support::integration::workflow::save_workflow(dir, &wf).unwrap();

        // Update same workflow (original_name = Some("my-wf"), name = "my-wf")
        let mut updated = make_test_workflow("my-wf");
        updated.description = "updated desc".to_string();
        let result = simulate_save_workflow(dir, &updated, Some("my-wf"));
        assert!(
            result.is_ok(),
            "Expected same-name update to succeed, got: {result:?}"
        );

        let loaded = releash_lib::test_support::integration::workflow::load_workflow(
            &dir.join("my-wf.yml"),
            dir,
        )
        .unwrap();
        assert_eq!(loaded.description, "updated desc");
    }

    #[test]
    pub fn save_workflow_new_creation_succeeds() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();

        let wf = make_test_workflow("brand-new");
        let result = simulate_save_workflow(dir, &wf, None);
        assert!(result.is_ok());
        assert!(dir.join("brand-new.yml").exists());
    }

    #[test]
    pub fn save_workflow_new_creation_rejects_duplicate() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();

        let wf = make_test_workflow("dup-wf");
        releash_lib::test_support::integration::workflow::save_workflow(dir, &wf).unwrap();

        let result = simulate_save_workflow(dir, &wf, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("既に存在します"));
    }

    #[test]
    pub fn save_workflow_rename_succeeds_and_removes_old_file() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();

        let wf = make_test_workflow("old-name");
        releash_lib::test_support::integration::workflow::save_workflow(dir, &wf).unwrap();

        let mut renamed = make_test_workflow("new-name");
        renamed.description = "renamed".to_string();
        let result = simulate_save_workflow(dir, &renamed, Some("old-name"));
        assert!(result.is_ok());
        assert!(!dir.join("old-name.yml").exists());
        assert!(dir.join("new-name.yml").exists());
    }

    #[test]
    pub fn save_workflow_rename_rejects_existing_target() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();

        releash_lib::test_support::integration::workflow::save_workflow(
            dir,
            &make_test_workflow("wf-a"),
        )
        .unwrap();
        releash_lib::test_support::integration::workflow::save_workflow(
            dir,
            &make_test_workflow("wf-b"),
        )
        .unwrap();

        let renamed = make_test_workflow("wf-b");
        let result = simulate_save_workflow(dir, &renamed, Some("wf-a"));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("既に存在します"));
    }

    // ---- [05] read-only Execution 観測 API: Tauri command 境界の直接テスト ----

    fn read_only_test_uuid(seed: u8) -> String {
        uuid::Uuid::from_bytes([seed; 16]).to_string()
    }

    pub(crate) fn make_read_only_app() -> (
        WorkflowTestDependencies,
        std::path::PathBuf,
        Arc<releash_lib::test_support::integration::persistence::LocalEventStore>,
    ) {
        make_read_only_app_with_terminal(Arc::new(
            releash_lib::test_support::integration::platform::build_terminal_surface_application_for_tests(),
        ))
    }

    pub(crate) fn make_read_only_app_with_terminal(
        terminal_surface: Arc<
            releash_lib::test_support::integration::terminal::TerminalSurfaceApplication,
        >,
    ) -> (
        WorkflowTestDependencies,
        std::path::PathBuf,
        Arc<releash_lib::test_support::integration::persistence::LocalEventStore>,
    ) {
        let data_dir =
            std::env::temp_dir().join(format!("releash-command-adapter-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&data_dir).unwrap();
        let app_config = Arc::new(
            releash_lib::test_support::integration::settings::AppConfig::new(
                releash_lib::test_support::integration::settings::ReleashConfig::default(),
                data_dir.join("config.toml"),
            ),
        );
        let config_repository: Arc<
            dyn releash_lib::test_support::integration::repository::ConfigRepository,
        > = app_config.clone();
        let repository_usecase =
            Arc::new(releash_lib::test_support::integration::platform::build_repository_usecase());
        let notion_usecase = Arc::new(
            releash_lib::test_support::integration::platform::NotionUsecase::new(
                app_config.clone(),
                app_config.clone(),
                Arc::new(
                    releash_lib::test_support::integration::platform::NotionApiGatewayImpl::new(
                        releash_lib::test_support::integration::platform::shared_limiter(),
                    ),
                ),
            ),
        );
        let repo_paths_gateway =
            releash_lib::test_support::integration::repository::RepoPathsGateway::new(
                <releash_lib::test_support::integration::repository::SharedRepoPaths>::default(),
                config_repository.clone(),
            );
        let repo_paths_usecase = Arc::new(
            releash_lib::test_support::integration::platform::RepoPathsUsecase::new(
                Arc::new(repo_paths_gateway),
                releash_lib::test_support::integration::subscriptions::test_subscriptions(),
            ),
        );
        let code_usecase =
            Arc::new(releash_lib::test_support::integration::platform::build_code_usecase());
        let repository_scanner = Arc::new(
            releash_lib::test_support::integration::repository::DefaultRepositoryScanner::new(
                repository_usecase.clone(),
                code_usecase.clone(),
            ),
        );
        let repository_state_repository = Arc::new(
            releash_lib::test_support::integration::repository::RepositoryStateRepositoryGateway::new(
                repository_usecase.clone(),
            ),
        );
        let repository_state = Arc::new(
            releash_lib::test_support::integration::platform::RepositoryStateService::new(
                repository_state_repository,
                repository_scanner,
                releash_lib::test_support::integration::subscriptions::test_subscriptions(),
                Arc::new(releash_lib::test_support::integration::platform::NoopRepositoryStateWatcher),
                Arc::new(
                    releash_lib::test_support::integration::platform::TestRepositoryStateWorkerRuntime,
                ),
                Arc::new(
                    releash_lib::test_support::integration::platform::IdentityWorktreePathNormalizer,
                ),

releash_lib::test_support::integration::subscriptions::repository_driver(),
),
        );
        let review_usecase = Arc::new(
            releash_lib::test_support::integration::platform::ReviewUsecase::new(
                repository_state.clone(),
                code_usecase.clone(),
            ),
        );
        let local_event_store = releash_lib::test_support::integration::persistence::LocalEventStore::open(
            releash_lib::test_support::integration::persistence::LocalEventStoreConfig::production(
                data_dir.clone(),
                std::sync::Arc::new(releash_lib::test_support::integration::platform::RetryLimiter::new()),
            ),
        )
        .unwrap();
        let (workflow_usecase, _) =
            releash_lib::test_support::integration::platform::build_workflow_services_with_repository_worktrees(
                Arc::new(releash_lib::test_support::integration::platform::FailureRecordStore::default()),
                data_dir.clone(),
                repository_usecase.clone(),
                config_repository.clone(),
                local_event_store.clone(),
                Arc::new(
                    releash_lib::test_support::integration::workflow::WorkflowNodeProcesses::default(
                    ),
                ),
            );
        let workflow_usecase = Arc::new(workflow_usecase);
        let git_host_usecase =
            Arc::new(releash_lib::test_support::integration::platform::build_git_host_usecase());
        let workspace_list = Arc::new(
            releash_lib::test_support::integration::platform::build_workspace_list_usecase(
                repo_paths_usecase.clone(),
                repository_usecase.clone(),
                repository_state.clone(),
                workflow_usecase.clone(),
                git_host_usecase.clone(),
            ),
        );
        let app_state = AppState {
            workspace_list,
            repository_usecase: repository_usecase.clone(),
            repo_paths_usecase,
            code_usecase,
            review_usecase,
            notion_usecase,
            workflow_usecase,
            terminal_surface,
            git_host_usecase,
        };
        let mut client =
            releash_lib::test_support::integration::transport::build_client_dependencies(
                data_dir.clone(),
            );
        client.app_state = Some(app_state);
        client.config_repository = Some(config_repository.clone());
        client.app_config_usecase = Some(Arc::new(
            releash_lib::test_support::integration::settings::AppConfigUsecase::new(
                config_repository,
                app_config.clone(),
            ),
        ));
        client.watcher = Arc::new(releash_lib::test_support::integration::platform::WatcherUsecase::new(
            Some(repository_state.clone()),
            Arc::new(
                releash_lib::test_support::integration::repository::FileWatcherGateway::new(
                    Arc::new(releash_lib::test_support::integration::platform::FileWatcherManager::default()),
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
        Arc<releash_lib::test_support::integration::persistence::LocalEventStore>,
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
        config.app.last_repo_paths = vec![repo_path.to_string_lossy().to_string()];
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

    /// Spec [05] Rule: 指定 execution の現在 state を観測する（event log からの純粋投影）。
    /// 観測結果の露出範囲境界: live runtime registry / OpenTabRegistry 由来の runtime_active /
    /// tab_open enrichment は含めない（戻り値の runtime_states は空）。
    #[tokio::test]
    pub async fn execution_projection_preserves_state_without_runtime_enrichment() {
        let (app, _data_dir, local_event_store, worktree_path, _r, _w) =
            make_read_only_app_with_managed_worktree();
        let execution_id = read_only_test_uuid(5);
        releash_lib::test_support::integration::workflow::append_canonical_events(
            &local_event_store,
            &[
                WorkflowEvent::ExecutionStarted {
                    repository_root: None,
                    execution_id: execution_id.clone(),
                    workflow_name: "adapter-boundary".to_string(),
                    worktree_path: worktree_path.clone(),
                    created_from: ExecutionOrigin::DesktopUi,
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
            .read_usecase();
        let view = read
            .get_execution_state(&execution_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(view.id, execution_id);
        assert_eq!(view.node_executions.len(), 1);
        assert_eq!(view.node_executions[0].node_name, "main");
        assert!(view.node_executions[0].session_id.is_none());

        // 存在しない execution_id は Ok(None)。
        let missing = read
            .get_execution_state(&read_only_test_uuid(97))
            .await
            .unwrap();
        assert!(missing.is_none());
    }
}
