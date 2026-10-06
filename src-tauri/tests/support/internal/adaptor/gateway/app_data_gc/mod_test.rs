pub(crate) mod tests {
    use super::super::*;
    use crate::domain::app_data_gc::GcCategory;
    use crate::usecase::app_data_gc::run_startup_gc;
    use parking_lot::RwLock;
    use std::sync::Mutex;
    use std::time::{Duration, SystemTime};

    #[derive(Default)]
    struct RecordingObserver {
        operations: Mutex<Vec<(StorePathOperation, PathBuf)>>,
    }

    impl StorePathObserver for RecordingObserver {
        fn observe(&self, operation: StorePathOperation, path: &Path) {
            self.operations
                .lock()
                .expect("recording observer")
                .push((operation, path.to_path_buf()));
        }
    }

    #[test]
    pub fn b070_background_gc_retention_and_restart_never_access_legacy_session_or_workflow_paths()
    {
        let app_data = tempfile::tempdir().expect("app data");
        let root = app_data.path();
        let legacy_roots = [
            "sessions",
            "workflow_runs",
            "workflow_logs",
            "workflow_execution_logs",
            "workflow_executions",
            "workflow_event_logs",
        ];
        let sentinel = b"\0B-070 malformed legacy sentinel";
        for name in legacy_roots {
            let directory = root.join(name);
            std::fs::create_dir(&directory).expect("legacy root");
            std::fs::write(directory.join("sentinel.invalid"), sentinel).expect("legacy sentinel");
        }
        std::fs::write(root.join("session_titles.json"), sentinel).expect("title sentinel");
        let legacy_paths = [
            root.join("sessions"),
            root.join("session_titles.json"),
            root.join("workflow_runs"),
            root.join("workflow_logs"),
            root.join("workflow_execution_logs"),
            root.join("workflow_executions"),
            root.join("workflow_event_logs"),
        ];
        let before = legacy_paths
            .iter()
            .map(|path| {
                if path.is_dir() {
                    (
                        path.clone(),
                        std::fs::read(path.join("sentinel.invalid")).expect("before bytes"),
                        std::fs::metadata(path.join("sentinel.invalid"))
                            .expect("before metadata")
                            .modified()
                            .expect("before mtime"),
                    )
                } else {
                    (
                        path.clone(),
                        std::fs::read(path).expect("before bytes"),
                        std::fs::metadata(path)
                            .expect("before metadata")
                            .modified()
                            .expect("before mtime"),
                    )
                }
            })
            .collect::<Vec<_>>();

        let cache = root.join("lsp/typescript");
        std::fs::create_dir_all(&cache).expect("cache root");
        let cache_file = cache.join("cache.bin");
        std::fs::write(&cache_file, b"regenerable").expect("cache bytes");
        let old = filetime::FileTime::from_system_time(
            SystemTime::now() - Duration::from_secs(8 * 24 * 60 * 60),
        );
        filetime::set_file_mtime(&cache_file, old).expect("old cache file");
        filetime::set_file_mtime(&cache, old).expect("old cache directory");

        let checkpoints = root.join("agent-worktree-checkpoints");
        std::fs::create_dir_all(&checkpoints).expect("checkpoint root");
        std::fs::write(checkpoints.join("retained.checkpoint"), b"checkpoint").expect("checkpoint");

        let legacy_process_dir = root.join("agent-processes");
        std::fs::create_dir_all(&legacy_process_dir).expect("process root");
        let legacy_process = legacy_process_dir.join("stale.codex.999999.json");
        std::fs::write(&legacy_process, b"legacy process record").expect("legacy process record");

        let observer = Arc::new(RecordingObserver::default());
        let file_system = StdGcFileSystem::with_observer(observer.clone());
        let repos: SharedRepoPaths = Arc::new(RwLock::new(Vec::new()));
        for _ in 0..2 {
            let mut request =
                build_startup_gc_request(root.to_path_buf(), repos.clone(), &file_system);
            apply_canonical_runtime_owners(&mut request, CanonicalRuntimeOwners::default());
            let report = run_startup_gc(request, &file_system);
            assert_eq!(report.errors, 0);
        }

        assert!(
            !cache.exists(),
            "expired regenerable cache must be collected"
        );
        assert!(
            legacy_process.exists(),
            "legacy Agent process data must not be deleted"
        );
        assert!(
            checkpoints.join("retained.checkpoint").exists(),
            "unmapped checkpoint must be retained conservatively"
        );
        let observed = observer.operations.lock().expect("observed operations");
        assert!(
            observed.iter().all(|(operation, path)| {
                !(*operation == StorePathOperation::ReadDir && path == root)
                    && legacy_paths
                        .iter()
                        .all(|legacy| path != legacy && !path.starts_with(legacy))
            }),
            "GC accessed a B-070 legacy source or enumerated app-data root: {observed:?}"
        );
        drop(observed);
        for (path, bytes, modified) in before {
            let sentinel_path = if path.is_dir() {
                path.join("sentinel.invalid")
            } else {
                path
            };
            assert_eq!(std::fs::read(&sentinel_path).expect("after bytes"), bytes);
            assert_eq!(
                std::fs::metadata(&sentinel_path)
                    .expect("after metadata")
                    .modified()
                    .expect("after mtime"),
                modified
            );
        }
    }

    #[test]
    pub fn issue_1372_nonlegacy_gc_keeps_live_workspace_data_and_collects_deleted_workspace_data() {
        let app_data = tempfile::tempdir().expect("app data");
        let repo = tempfile::tempdir().expect("repo");
        git2::Repository::init(repo.path()).expect("init repo");
        let root = app_data.path();
        let live_name = repo
            .path()
            .file_name()
            .and_then(|name| name.to_str())
            .expect("repo name");
        let live_workspace_key =
            crate::adaptor::gateway::workspace_state::repository_impl::storage_key(live_name);
        let live_review_key = crate::adaptor::gateway::comment::worktree_storage_key(
            repo.path().to_string_lossy().as_ref(),
        );
        let workspace_state = root.join("workspace_state");
        let review_comments = root.join("review-comments");
        std::fs::create_dir_all(&workspace_state).expect("workspace state");
        std::fs::create_dir_all(&review_comments).expect("review comments");
        let live_workspace = workspace_state.join(format!("{live_workspace_key}.json"));
        let stale_workspace = workspace_state.join("deleted-workspace.json");
        let live_review = review_comments.join(format!("{live_review_key}.events.json"));
        let stale_review = review_comments.join("deleted-workspace.events.json");
        for path in [
            &live_workspace,
            &stale_workspace,
            &live_review,
            &stale_review,
        ] {
            std::fs::write(path, b"{}").expect("fixture");
        }

        let file_system = StdGcFileSystem::default();
        let repos: SharedRepoPaths = Arc::new(RwLock::new(vec![repo
            .path()
            .to_string_lossy()
            .into_owned()]));
        let mut request = build_startup_gc_request(root.to_path_buf(), repos, &file_system);
        apply_canonical_runtime_owners(&mut request, CanonicalRuntimeOwners::default());
        let report = run_startup_gc(request, &file_system);

        assert!(live_workspace.exists());
        assert!(live_review.exists());
        assert!(!stale_workspace.exists());
        assert!(!stale_review.exists());
        assert_eq!(report.categories[&GcCategory::DeletedWorkspace].deleted, 2);
    }

    #[test]
    pub fn issue_1372_cache_boundary_and_legacy_comment_cleanup_remain_active() {
        let app_data = tempfile::tempdir().expect("app data");
        let root = app_data.path();
        let exact_boundary = root.join("lsp/exact-boundary");
        let expired = root.join("lsp/expired");
        std::fs::create_dir_all(exact_boundary.parent().expect("lsp parent")).expect("lsp");
        std::fs::write(&exact_boundary, b"keep").expect("boundary cache");
        std::fs::write(&expired, b"delete").expect("expired cache");
        for name in ["comments", "diff-comments", "threads"] {
            let path = root.join(name);
            std::fs::create_dir(&path).expect("legacy comment root");
            std::fs::write(path.join("record"), b"legacy").expect("legacy comment");
        }
        let current_review = root.join("review-comments/current.events.json");
        std::fs::create_dir_all(current_review.parent().expect("review parent")).expect("review");
        std::fs::write(&current_review, b"[]").expect("current review");

        let file_system = StdGcFileSystem::default();
        let repos: SharedRepoPaths = Arc::new(RwLock::new(Vec::new()));
        let mut request = build_startup_gc_request(root.to_path_buf(), repos, &file_system);
        request.now_secs = request.retention.cache_secs as f64;
        request.cache_records = vec![
            CacheGcRecord {
                path: exact_boundary.clone(),
                updated_at: 0.0,
            },
            CacheGcRecord {
                path: expired.clone(),
                updated_at: -0.001,
            },
        ];
        let report = run_startup_gc(request, &file_system);

        assert!(exact_boundary.exists(), "exactly seven days is retained");
        assert!(!expired.exists(), "older than seven days is collected");
        for name in ["comments", "diff-comments", "threads"] {
            assert!(!root.join(name).exists());
        }
        assert!(current_review.exists());
        assert_eq!(report.categories[&GcCategory::RegenerableCache].deleted, 1);
        assert_eq!(report.categories[&GcCategory::LegacyComments].deleted, 3);
    }
}
