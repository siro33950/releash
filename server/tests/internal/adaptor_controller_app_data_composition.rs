pub(crate) mod tests {

    use std::path::Path;
    use std::path::PathBuf;
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::time::Duration;
    use std::time::SystemTime;

    use parking_lot::RwLock;

    struct NoExecutionTrees;
    #[async_trait::async_trait]
    impl releashd::test_support::integration::persistence::ExecutionTreeGc for NoExecutionTrees {
        async fn record_repository_root(
            &self,
            _: &str,
            _: &str,
        ) -> Result<(), releashd::test_support::integration::workflow::WorkflowError> {
            unreachable!()
        }
        async fn execution_trees(
            &self,
            _: Option<&str>,
        ) -> Result<
            Vec<releashd::test_support::integration::workflow::ExecutionTreeArchiveCandidate>,
            releashd::test_support::integration::workflow::WorkflowError,
        > {
            Ok(Vec::new())
        }
        async fn archive_removed_tree(
            &self,
            _: &str,
        ) -> Result<(), releashd::test_support::integration::workflow::WorkflowError> {
            unreachable!()
        }
    }
    use releashd::test_support::integration::platform::AppDataPathObserver;
    use releashd::test_support::integration::platform::ProductionAppDataComposition;
    use releashd::test_support::integration::repository::LocalEventTransactionRepository;

    use releashd::test_support::integration::platform::AppDataPathOperation;
    use releashd::test_support::integration::platform::CanonicalRuntimeOwnerView;
    use releashd::test_support::integration::platform::CommitBatchError;
    use releashd::test_support::integration::platform::CommitBatchResult;
    use releashd::test_support::integration::platform::CommitIdentity;
    use releashd::test_support::integration::platform::CommitResolution;
    use releashd::test_support::integration::platform::DomainEventPage;
    use releashd::test_support::integration::platform::LocalAtomicBatch;
    use releashd::test_support::integration::platform::LocalEventQuery;
    use releashd::test_support::integration::platform::LocalEventQueryError;
    use releashd::test_support::integration::platform::LocalEventQueryResult;
    use releashd::test_support::integration::platform::LocalStateMutation;

    #[derive(Default)]
    struct RecordingObserver {
        operations: Mutex<Vec<(AppDataPathOperation, PathBuf)>>,
    }

    impl AppDataPathObserver for RecordingObserver {
        fn observe(&self, operation: AppDataPathOperation, path: &Path) {
            self.operations
                .lock()
                .expect("recording observer")
                .push((operation, path.to_path_buf()));
        }
    }

    enum OwnerQueryAction {
        AppendFacts(
            Vec<(
                releashd::test_support::integration::workflow::NodeFactMeta,
                releashd::test_support::integration::workflow::NodeFact,
            )>,
        ),
        FailRevalidation,
        ReturnWrongShape,
        ReturnOversizeSnapshot(Vec<CanonicalRuntimeOwnerView>),
    }

    struct OwnerRaceRepository {
        inner: Arc<releashd::test_support::integration::persistence::LocalEventStore>,
        action: Mutex<Option<OwnerQueryAction>>,
        action_at_owner_query: usize,
        owner_query_calls: AtomicUsize,
        paged_owner_query_calls: AtomicUsize,
        committed_action_count: AtomicUsize,
    }

    impl OwnerRaceRepository {
        fn new(
            inner: Arc<releashd::test_support::integration::persistence::LocalEventStore>,
            action: OwnerQueryAction,
        ) -> Self {
            Self {
                inner,
                action: Mutex::new(Some(action)),
                action_at_owner_query: 1,
                owner_query_calls: AtomicUsize::new(0),
                paged_owner_query_calls: AtomicUsize::new(0),
                committed_action_count: AtomicUsize::new(0),
            }
        }

        fn at_initial_query(
            inner: Arc<releashd::test_support::integration::persistence::LocalEventStore>,
            action: OwnerQueryAction,
        ) -> Self {
            Self {
                inner,
                action: Mutex::new(Some(action)),
                action_at_owner_query: 0,
                owner_query_calls: AtomicUsize::new(0),
                paged_owner_query_calls: AtomicUsize::new(0),
                committed_action_count: AtomicUsize::new(0),
            }
        }

        fn owner_query_calls(&self) -> usize {
            self.owner_query_calls.load(Ordering::SeqCst)
        }

        fn paged_owner_query_calls(&self) -> usize {
            self.paged_owner_query_calls.load(Ordering::SeqCst)
        }

        fn committed_action_count(&self) -> usize {
            self.committed_action_count.load(Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl LocalEventTransactionRepository for OwnerRaceRepository {
        fn canonical_mutation_identity_v1(
            &self,
            mutation: &LocalStateMutation,
        ) -> Result<Vec<u8>, String> {
            self.inner.canonical_mutation_identity_v1(mutation)
        }

        fn canonical_event_batch_identity_v1(
            &self,
            events: &[releashd::test_support::integration::platform::UncommittedDomainEvent],
        ) -> Result<Vec<u8>, String> {
            self.inner.canonical_event_batch_identity_v1(events)
        }

        async fn commit_batch(
            &self,
            batch: LocalAtomicBatch,
        ) -> Result<CommitBatchResult, CommitBatchError> {
            self.inner.commit_batch(batch).await
        }

        async fn resolve_commit(
            &self,
            identity: CommitIdentity,
        ) -> Result<CommitResolution, LocalEventQueryError> {
            self.inner.resolve_commit(identity).await
        }

        async fn load_stream(
            &self,
            request: releashd::test_support::integration::platform::LoadStreamRequest,
        ) -> Result<DomainEventPage, LocalEventQueryError> {
            self.inner.load_stream(request).await
        }

        async fn query(
            &self,
            request: LocalEventQuery,
        ) -> Result<LocalEventQueryResult, LocalEventQueryError> {
            if !matches!(
                request,
                LocalEventQuery::CanonicalRuntimeOwnerSnapshot { .. }
            ) {
                return self.inner.query(request).await;
            }
            let call = self.owner_query_calls.fetch_add(1, Ordering::SeqCst);
            if call == self.action_at_owner_query {
                let action = self.action.lock().expect("owner race action").take();
                if let Some(action) = action {
                    match action {
                        OwnerQueryAction::AppendFacts(facts) => {
                            let seed_identity = facts
                                .first()
                                .map(|(meta, _)| meta.tree_id.as_str())
                                .unwrap_or("owner-race-empty");
                            releashd::test_support::integration::workflow::append_fact_batch_for_seed(
                                &self.inner,
                                &facts,
                                1,
                                seed_identity,
                            )
                            .expect("facts appended after candidate planning");
                            self.committed_action_count.fetch_add(1, Ordering::SeqCst);
                        }
                        OwnerQueryAction::FailRevalidation => {
                            return Err(LocalEventQueryError::Internal {
                                correlation_id: "gc-revalidation-failure".to_string(),
                            });
                        }
                        OwnerQueryAction::ReturnWrongShape => {
                            return Ok(LocalEventQueryResult::SessionProjectionByIdentity(None));
                        }
                        OwnerQueryAction::ReturnOversizeSnapshot(snapshot) => {
                            return Ok(LocalEventQueryResult::CanonicalRuntimeOwnerSnapshot(
                                snapshot,
                            ));
                        }
                    }
                }
            }
            self.inner.query(request).await
        }
    }

    fn session_root_facts(
        session_id: &str,
        worktree_path: &str,
    ) -> Vec<(
        releashd::test_support::integration::workflow::NodeFactMeta,
        releashd::test_support::integration::workflow::NodeFact,
    )> {
        releashd::test_support::integration::workflow::SessionExecutionTreeRootFacts::new(
            session_id,
            worktree_path,
            worktree_path,
            releashd::test_support::integration::providers::ProviderKind::Claude,
            None,
        )
        .unwrap()
        .into_facts()
        .into_iter()
        .collect()
    }

    fn workflow_root_facts(
        execution_id: &str,
        worktree_path: &str,
    ) -> Vec<(
        releashd::test_support::integration::workflow::NodeFactMeta,
        releashd::test_support::integration::workflow::NodeFact,
    )> {
        use releashd::test_support::integration::workflow::ExecutionOrigin;
        use releashd::test_support::integration::workflow::ExecutionTreeLaunch;
        use releashd::test_support::integration::workflow::NodeCompletion;
        use releashd::test_support::integration::workflow::NodeDefinition;
        use releashd::test_support::integration::workflow::NodeFact;
        use releashd::test_support::integration::workflow::NodeFactMeta;
        use releashd::test_support::integration::workflow::NodeKind;
        use releashd::test_support::integration::workflow::NodeKindName;
        use releashd::test_support::integration::workflow::SessionSpec;
        use releashd::test_support::integration::workflow::StartedFact;
        use releashd::test_support::integration::workflow::TreeRootFact;
        use releashd::test_support::integration::workflow::WorkflowDefinition;
        vec![(
            NodeFactMeta {
                tree_id: execution_id.to_string(),
                node_execution_id: execution_id.to_string(),
                parent_id: None,
                node_name: "main".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
            },
            NodeFact::Started(StartedFact {
                worktree: None,
                parent: None,
                root: Some(Box::new(TreeRootFact {
                    repository_root: None,
                    workspace_identity: worktree_path.to_string(),
                    worktree_path: worktree_path.to_string(),
                    created_from: ExecutionOrigin::Cli,
                    request: String::new(),
                    workflow_name: "wf".to_string(),
                    definition: Some(WorkflowDefinition {
                        name: "wf".to_string(),
                        description: String::new(),
                        builtin: false,
                        schemas: Default::default(),
                        nodes: vec![NodeDefinition {
                            name: "main".to_string(),
                            kind: NodeKind::Session(SessionSpec::default()),
                            artifact: None,
                            input: Vec::new(),
                            completion: NodeCompletion::default(),
                            worktree: None,
                        }],
                        entry: "main".to_string(),
                    }),
                    launched_as: ExecutionTreeLaunch::Workflow,
                })),
            }),
        )]
    }

    fn oversize_owner_snapshot() -> Vec<CanonicalRuntimeOwnerView> {
        (0..8_193)
            .map(|index| CanonicalRuntimeOwnerView::ActiveWorkflow {
                worktree_path: format!("/oversize-owner/{index}"),
            })
            .collect()
    }

    fn create_workspace_keyed_candidates(
        app_data_dir: &Path,
        worktree_path: &str,
    ) -> (PathBuf, PathBuf) {
        let workspace_key =
            releashd::test_support::integration::platform::storage_key(worktree_path);
        let review_key =
            releashd::test_support::integration::platform::worktree_storage_key(worktree_path);
        let workspace_state = app_data_dir
            .join("workspace_state.migrated")
            .join(format!("{workspace_key}.json"));
        let review_comments = app_data_dir
            .join("review-comments")
            .join(format!("{review_key}.events.json"));
        std::fs::create_dir_all(workspace_state.parent().expect("workspace state parent"))
            .expect("workspace state directory");
        std::fs::create_dir_all(review_comments.parent().expect("review comments parent"))
            .expect("review comments directory");
        std::fs::write(&workspace_state, b"{}").expect("workspace state fixture");
        std::fs::write(&review_comments, b"[]").expect("review comment fixture");
        (workspace_state, review_comments)
    }

    fn live_repo_paths() -> (
        tempfile::TempDir,
        releashd::test_support::integration::repository::SharedRepoPaths,
    ) {
        let live_repo = tempfile::tempdir().expect("live repo");
        git2::Repository::init(live_repo.path()).expect("initialize live repo");
        let repo_paths = Arc::new(RwLock::new(vec![live_repo
            .path()
            .to_string_lossy()
            .into_owned()]));
        (live_repo, repo_paths)
    }

    fn ui_state() -> releashd::test_support::integration::platform::WorkspaceState {
        use releashd::test_support::integration::platform::{
            PaneLayout, WorkspaceLayoutState, WorkspaceState, WorkspaceTabsState,
        };
        WorkspaceState {
            panes: Some(PaneLayout::pane("old-pane".into(), vec![], None).unwrap()),
            version: 1,
            tabs: WorkspaceTabsState {
                editors: vec![],
                active_editor_path: None,
            },
            layout: WorkspaceLayoutState {
                center_tab: "agent".into(),
                active_view: "git".into(),
                left_nav_collapsed: false,
                right_collapsed: false,
                right_bottom_collapsed: false,
                right_bottom_active_tab: None,
                selected_diff_file: None,
            },
        }
    }

    async fn assert_facts_appended_at_sweep_boundary_are_protected(
        facts: Vec<(
            releashd::test_support::integration::workflow::NodeFactMeta,
            releashd::test_support::integration::workflow::NodeFact,
        )>,
        protected_worktree: &str,
    ) {
        let app_data = tempfile::tempdir().expect("app data");
        let observer = Arc::new(RecordingObserver::default());
        let composition = ProductionAppDataComposition::with_observer(
            app_data.path().to_path_buf(),
            observer.clone(),
        );
        let store = composition
            .open_local_event_store()
            .expect("open canonical store");
        let repository = Arc::new(OwnerRaceRepository::new(
            store,
            OwnerQueryAction::AppendFacts(facts),
        ));
        let (workspace_state, review_comments) =
            create_workspace_keyed_candidates(app_data.path(), protected_worktree);
        let (_live_repo, repo_paths) = live_repo_paths();
        observer
            .operations
            .lock()
            .expect("recorded operations")
            .clear();

        use releashd::test_support::integration::repository::WorkspaceStateRepository;
        let states = Arc::new(
            releashd::test_support::integration::platform::WorkspaceStateStore::with_store(
                app_data.path().into(),
                repository.inner.clone(),
            ),
        );
        let pending_state = ui_state();
        releashd::test_support::integration::platform::save_workspace_state(
            states.as_ref(),
            None,
            protected_worktree,
            pending_state,
        )
        .await
        .unwrap();
        let report = composition
            .run_startup_gc_pass(
                repo_paths,
                repository.clone(),
                Arc::new(NoExecutionTrees),
                states.clone(),
            )
            .await
            .expect("run startup GC");

        assert_eq!(report.errors, 0);
        assert_eq!(
            states
                .load(protected_worktree, protected_worktree)
                .await
                .unwrap(),
            Some(ui_state())
        );
        assert!(
            workspace_state.exists(),
            "active canonical owner committed after planning must retain workspace_state"
        );
        assert!(
            review_comments.exists(),
            "active canonical owner committed after planning must retain review-comments"
        );
        assert!(
            repository.owner_query_calls() >= 2,
            "canonical owners must be queried again at the sweep boundary"
        );
        assert_eq!(
            repository.paged_owner_query_calls(),
            0,
            "GC owner completeness must not be assembled from independent pages"
        );
        assert_eq!(
            repository.committed_action_count(),
            1,
            "the active owner projection must commit after candidate planning"
        );
        let operations = observer.operations.lock().expect("recorded operations");
        assert_eq!(
            operations
                .iter()
                .filter(|(operation, _)| *operation == AppDataPathOperation::Remove)
                .count(),
            0,
            "no removal may start after revalidation protects the only candidates"
        );
    }

    #[tokio::test]
    pub async fn app_data_gc_active_session_committed_after_plan_is_revalidated_before_remove() {
        let worktree = "/deleted-before-gc-but-session-became-active";
        assert_facts_appended_at_sweep_boundary_are_protected(
            session_root_facts("gc-race-active-session", worktree),
            worktree,
        )
        .await;
    }

    #[tokio::test]
    pub async fn app_data_gc_running_workflow_committed_after_plan_is_revalidated_before_remove() {
        let worktree = "/deleted-before-gc-but-workflow-became-running";
        assert_facts_appended_at_sweep_boundary_are_protected(
            workflow_root_facts("gc-race-running-workflow", worktree),
            worktree,
        )
        .await;
    }

    #[tokio::test]
    pub async fn app_data_gc_revalidation_query_failure_retains_workspace_keyed_candidates() {
        let app_data = tempfile::tempdir().expect("app data");
        let observer = Arc::new(RecordingObserver::default());
        let composition = ProductionAppDataComposition::with_observer(
            app_data.path().to_path_buf(),
            observer.clone(),
        );
        let store = composition
            .open_local_event_store()
            .expect("open canonical store");
        let repository = Arc::new(OwnerRaceRepository::new(
            store,
            OwnerQueryAction::FailRevalidation,
        ));
        let (workspace_state, review_comments) = create_workspace_keyed_candidates(
            app_data.path(),
            "/deleted-worktree-revalidation-failure",
        );
        let legacy_comments = app_data.path().join("comments");
        std::fs::create_dir(&legacy_comments).expect("legacy comments");
        std::fs::write(legacy_comments.join("entry"), b"legacy").expect("legacy comment fixture");
        let expired_cache = app_data.path().join("lsp/typescript");
        std::fs::create_dir_all(&expired_cache).expect("cache directory");
        let expired_cache_entry = expired_cache.join("cache.bin");
        std::fs::write(&expired_cache_entry, b"regenerable").expect("cache fixture");
        let expired = filetime::FileTime::from_system_time(
            SystemTime::now() - Duration::from_secs(8 * 24 * 60 * 60),
        );
        filetime::set_file_mtime(&expired_cache_entry, expired).expect("cache entry mtime");
        filetime::set_file_mtime(&expired_cache, expired).expect("cache directory mtime");
        let process_directory = app_data.path().join("agent-processes");
        std::fs::create_dir(&process_directory).expect("process directory");
        let stale_process_record = process_directory.join("stale.codex.4294967295.json");
        std::fs::write(&stale_process_record, b"legacy process record")
            .expect("legacy process fixture");
        let (_live_repo, repo_paths) = live_repo_paths();

        let report = composition
            .run_startup_gc_pass(
                repo_paths,
                repository.clone(),
                Arc::new(NoExecutionTrees),
                Arc::new(
                    releashd::test_support::integration::platform::WorkspaceStateStore::with_store(
                        app_data.path().into(),
                        repository.inner.clone(),
                    ),
                ),
            )
            .await
            .expect("run startup GC");

        assert_eq!(report.errors, 0);
        assert!(workspace_state.exists());
        assert!(review_comments.exists());
        assert!(
            !legacy_comments.exists(),
            "owner-independent legacy-comment cleanup must remain active"
        );
        assert!(
            !expired_cache.exists(),
            "owner-independent regenerable-cache cleanup must remain active"
        );
        assert!(
            stale_process_record.exists(),
            "legacy Agent process data must not be deleted by the cutover"
        );
        assert!(repository.owner_query_calls() >= 2);
        assert_eq!(repository.paged_owner_query_calls(), 0);
        let operations = observer.operations.lock().expect("recorded operations");
        assert_eq!(
            operations
                .iter()
                .filter(|(operation, path)| {
                    *operation == AppDataPathOperation::Remove
                        && (path == &workspace_state || path == &review_comments)
                })
                .count(),
            0
        );
    }

    #[tokio::test]
    pub async fn app_data_gc_oversize_revalidation_snapshot_retains_workspace_keyed_candidates() {
        let app_data = tempfile::tempdir().expect("app data");
        let observer = Arc::new(RecordingObserver::default());
        let composition = ProductionAppDataComposition::with_observer(
            app_data.path().to_path_buf(),
            observer.clone(),
        );
        let store = composition
            .open_local_event_store()
            .expect("open canonical store");
        let repository = Arc::new(OwnerRaceRepository::new(
            store,
            OwnerQueryAction::ReturnOversizeSnapshot(oversize_owner_snapshot()),
        ));
        let (workspace_state, review_comments) = create_workspace_keyed_candidates(
            app_data.path(),
            "/deleted-worktree-with-oversize-owner-snapshot",
        );
        let (_live_repo, repo_paths) = live_repo_paths();
        observer
            .operations
            .lock()
            .expect("recorded operations")
            .clear();

        let report = composition
            .run_startup_gc_pass(
                repo_paths,
                repository.clone(),
                Arc::new(NoExecutionTrees),
                Arc::new(
                    releashd::test_support::integration::platform::WorkspaceStateStore::with_store(
                        app_data.path().into(),
                        repository.inner.clone(),
                    ),
                ),
            )
            .await
            .expect("run startup GC");

        assert_eq!(report.errors, 0);
        assert!(workspace_state.exists());
        assert!(review_comments.exists());
        assert_eq!(repository.paged_owner_query_calls(), 0);
        let operations = observer.operations.lock().expect("recorded operations");
        assert_eq!(
            operations
                .iter()
                .filter(|(operation, _)| *operation == AppDataPathOperation::Remove)
                .count(),
            0,
            "oversize canonical owner snapshot must fail closed"
        );
    }

    #[tokio::test]
    pub async fn app_data_gc_wrong_shape_initial_snapshot_retains_workspace_keyed_candidates() {
        let app_data = tempfile::tempdir().expect("app data");
        let observer = Arc::new(RecordingObserver::default());
        let composition = ProductionAppDataComposition::with_observer(
            app_data.path().to_path_buf(),
            observer.clone(),
        );
        let store = composition
            .open_local_event_store()
            .expect("open canonical store");
        let repository = Arc::new(OwnerRaceRepository::at_initial_query(
            store,
            OwnerQueryAction::ReturnWrongShape,
        ));
        let (workspace_state, review_comments) = create_workspace_keyed_candidates(
            app_data.path(),
            "/deleted-worktree-with-wrong-shape-initial-owner-snapshot",
        );
        let (_live_repo, repo_paths) = live_repo_paths();
        observer
            .operations
            .lock()
            .expect("recorded operations")
            .clear();

        let report = composition
            .run_startup_gc_pass(
                repo_paths,
                repository.clone(),
                Arc::new(NoExecutionTrees),
                Arc::new(
                    releashd::test_support::integration::platform::WorkspaceStateStore::with_store(
                        app_data.path().into(),
                        repository.inner.clone(),
                    ),
                ),
            )
            .await
            .expect("run startup GC");

        assert_eq!(report.errors, 0);
        assert!(workspace_state.exists());
        assert!(review_comments.exists());
        assert!(
            repository.owner_query_calls() >= 2,
            "fresh sweep-boundary owner query must still run after initial wrong shape"
        );
        assert_eq!(repository.paged_owner_query_calls(), 0);
        let operations = observer.operations.lock().expect("recorded operations");
        assert_eq!(
            operations
                .iter()
                .filter(|(operation, _)| *operation == AppDataPathOperation::Remove)
                .count(),
            0,
            "initial wrong-shape canonical snapshot must prevent workspace planning"
        );
    }
    #[tokio::test]
    pub async fn test_ui状態のgc_未解決時は保護し削除後は無効化して同じpathに復元しない() {
        use releashd::test_support::integration::platform::WorkspaceStateStore;
        use releashd::test_support::integration::repository::WorkspaceStateRepository;
        // Given
        let data = tempfile::tempdir().unwrap();
        let composition = ProductionAppDataComposition::with_observer(
            data.path().into(),
            Arc::new(RecordingObserver::default()),
        );
        let events = composition.open_local_event_store().unwrap();
        let states = Arc::new(WorkspaceStateStore::with_store(
            data.path().into(),
            events.clone(),
        ));
        let (repo_dir, repo) = crate::test_support_git::create_test_repo();
        crate::test_support_git::create_initial_commit(&repo);
        let main = repo_dir
            .path()
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let removed = repo_dir.path().join("removed");
        repo.worktree("removed", &removed, None).unwrap();
        let key = removed
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let state = ui_state();
        for path in [&main, &key] {
            let pending_state = state.clone();
            releashd::test_support::integration::platform::save_workspace_state(
                states.as_ref(),
                None,
                path,
                pending_state,
            )
            .await
            .unwrap();
        }
        let migrated = data.path().join("workspace_state.migrated");
        let legacy = data.path().join("workspace_state");
        std::fs::create_dir_all(&migrated).unwrap();
        std::fs::create_dir_all(&legacy).unwrap();
        let imported = migrated.join(format!(
            "{}.json",
            releashd::test_support::integration::platform::storage_key(&key)
        ));
        std::fs::write(&imported, "{}").unwrap();
        let unimported = legacy.join("unknown.json");
        std::fs::write(&unimported, "broken").unwrap();
        std::fs::remove_dir_all(&removed).unwrap();
        repo.find_worktree("removed").unwrap().prune(None).unwrap();
        let unresolved = repo_dir
            .path()
            .join("missing")
            .to_string_lossy()
            .into_owned();
        // When
        let report = composition
            .run_startup_gc_pass(
                Arc::new(RwLock::new(vec![main.clone(), unresolved])),
                events.clone(),
                Arc::new(NoExecutionTrees),
                states.clone(),
            )
            .await
            .unwrap();
        // Then
        assert_eq!(report.errors, 0);
        assert!(states.load(&key, &key).await.unwrap().is_some());
        assert!(imported.exists());
        // When
        let report = composition
            .run_startup_gc_pass(
                Arc::new(RwLock::new(vec![main.clone()])),
                events.clone(),
                Arc::new(NoExecutionTrees),
                states.clone(),
            )
            .await
            .unwrap();
        // Then
        assert_eq!(report.errors, 0);
        assert!(states.load(&key, &key).await.unwrap().is_none());
        assert_eq!(states.load(&main, &main).await.unwrap(), Some(state));
        assert!(!imported.exists());
        assert_eq!(std::fs::read_to_string(&unimported).unwrap(), "broken");
        let reference = repo.find_reference("refs/heads/removed").unwrap();
        let mut options = git2::WorktreeAddOptions::new();
        options.reference(Some(&reference));
        repo.worktree("removed", &removed, Some(&options)).unwrap();
        assert!(states.load(&key, &key).await.unwrap().is_none());
        let connection =
            rusqlite::Connection::open(data.path().join("local-event-store.sqlite3")).unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT count(*) FROM events WHERE event_type = 'workspace.state_invalidated'",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT count(*) FROM events WHERE event_type = 'workspace.state_changed'",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            2
        );
        drop(states);
        drop(events);
        let restarted = releashd::test_support::integration::platform::workspace_state_store(
            data.path().into(),
        );
        assert!(restarted.load(&key, &key).await.unwrap().is_none());
    }
}

use releashd::test_support::integration::platform::ProductionAppDataComposition;
use std::sync::Arc;

#[tokio::test]
pub async fn test_startup_gc結線_消失候補の未終了実行木をabortしてarchiveする() {
    use crate::adaptor_gateway_workflow_workflow_host_test_helpers::archive_fixture;
    use releashd::test_support::integration::workflow::ExecutionStatus;
    use releashd::test_support::integration::workflow::ExecutionTreeArchiveRepository;
    use releashd::test_support::integration::workflow::NodeFact;
    use releashd::test_support::integration::workflow::SessionExecutionTreeRootFacts;
    // Given
    let fixture = archive_fixture();
    let repo = tempfile::tempdir().unwrap();
    git2::Repository::init(repo.path()).unwrap();
    let path = repo.path().join("removed-worktree");
    std::fs::create_dir(&path).unwrap();
    let id = "00000000-0000-4000-8000-000000000995";
    let mut facts = SessionExecutionTreeRootFacts::new(
        id,
        path.to_str().unwrap(),
        path.to_str().unwrap(),
        releashd::test_support::integration::providers::ProviderKind::Codex,
        None,
    )
    .unwrap();
    if let NodeFact::Started(started) = &mut facts.started {
        started.root.as_mut().unwrap().repository_root =
            Some(repo.path().to_string_lossy().into_owned());
    }
    releashd::test_support::integration::workflow::append_fact_batch_for_seed(
        &fixture.store,
        &facts.into_facts(),
        1,
        id,
    )
    .unwrap();
    let composition = ProductionAppDataComposition::new(
        fixture.directory.path().into(),
        std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
    );
    // When
    let report = composition
        .run_startup_gc_pass(
            Arc::new(parking_lot::RwLock::new(vec![repo
                .path()
                .to_string_lossy()
                .into_owned()])),
            fixture.store.clone(),
            Arc::new(fixture.runtime),
            Arc::new(
                releashd::test_support::integration::platform::WorkspaceStateStore::with_store(
                    fixture.directory.path().into(),
                    fixture.store.clone(),
                ),
            ),
        )
        .await
        .unwrap();
    // Then
    assert_eq!(report.errors, 0);
    assert!(path.exists());
    assert_eq!(
        fixture.repository.target(id).await.unwrap().status,
        ExecutionStatus::Aborted
    );
    assert_eq!(
        fixture
            .repository
            .archive_snapshot_for(&[id.into()])
            .await
            .unwrap()
            .records[0]
            .archive_reason,
        "worktree_removed"
    );
}
