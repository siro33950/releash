//! Production app-data composition boundary.
//!
//! Issue #1499 B-070 is a lifecycle-wide constraint, not a property of the
//! SQLite adapter in isolation.  This composition owns the one app-data root
//! and the one path observer supplied to every app-data collaborator used by
//! startup maintenance: the fixed SQLite store, process-record cleanup, and
//! issue #1372 GC/retention.  Production installs the no-op observer; the
//! acceptance composition replaces it once here and therefore cannot
//! accidentally test an independently hand-wired set of adapters.

use std::path::PathBuf;
use std::sync::Arc;

use crate::adaptor::gateway::app_data_gc::{
    apply_canonical_runtime_owners, build_startup_gc_request, canonical_runtime_protection,
    live_process_session_ids, StdGcFileSystem,
};
use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
use crate::adaptor::gateway::repository::repo_paths::SharedRepoPaths;
use crate::domain::app_data_gc::GcReport;
use crate::domain::local_event::LocalEventTransactionRepository;
use crate::infrastructure::app_data_path::{AppDataPathObserver, NoopAppDataPathObserver};
use crate::infrastructure::process::pid_registry::CleanupReport;
#[cfg(test)]
use crate::infrastructure::process::pid_registry::PidRegistration;

#[derive(Clone)]
pub(crate) struct ProductionAppDataComposition {
    app_data_dir: PathBuf,
    observer: Arc<dyn AppDataPathObserver>,
}

impl ProductionAppDataComposition {
    pub(crate) fn new(app_data_dir: PathBuf) -> Self {
        Self {
            app_data_dir,
            observer: Arc::new(NoopAppDataPathObserver),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_observer(
        app_data_dir: PathBuf,
        observer: Arc<dyn AppDataPathObserver>,
    ) -> Self {
        Self {
            app_data_dir,
            observer,
        }
    }

    pub(crate) fn open_local_event_store(
        &self,
    ) -> Result<
        Arc<LocalEventStore>,
        crate::adaptor::gateway::local_event_store::store::LocalEventStoreOpenError,
    > {
        let mut config = LocalEventStoreConfig::production(self.app_data_dir.clone());
        config.path_observer = self.observer.clone();
        LocalEventStore::open(config)
    }

    pub(crate) fn cleanup_orphan_processes(&self) -> CleanupReport {
        crate::infrastructure::process::pid_registry::cleanup_orphan_processes_with_observer(
            &self.app_data_dir,
            self.observer.clone(),
        )
    }

    #[cfg(test)]
    pub(crate) fn register_process(
        &self,
        session_id: &str,
        backend_id: &str,
        pid: u32,
    ) -> Option<PidRegistration> {
        crate::infrastructure::process::pid_registry::save_pgid_with_observer(
            Some(&self.app_data_dir),
            session_id,
            backend_id,
            pid,
            self.observer.clone(),
        )
    }

    /// Execute the exact production GC/retention pass.
    ///
    /// Inventory and sweeping remain on blocking workers.  Failure to load
    /// canonical runtime owners leaves workspace-keyed protection incomplete,
    /// so those deletions fail closed while independent cache/comment/process
    /// retention can still run.
    pub(crate) async fn run_startup_gc_pass(
        &self,
        shared_repo_paths: SharedRepoPaths,
        repository: Arc<dyn LocalEventTransactionRepository>,
    ) -> Result<GcReport, String> {
        let file_system = StdGcFileSystem::with_observer(self.observer.clone());
        let inventory_file_system = file_system.clone();
        let app_data_dir = self.app_data_dir.clone();
        let inventory = tokio::task::spawn_blocking(move || {
            build_startup_gc_request(app_data_dir, shared_repo_paths, &inventory_file_system)
        })
        .await
        .map_err(|error| format!("app data gc inventory task failed: {error}"))?;
        let mut request = inventory;

        let live_process_session_ids = live_process_session_ids(&request);
        match crate::usecase::app_data_gc::load_canonical_runtime_owners(
            repository.clone(),
            live_process_session_ids.clone(),
        )
        .await
        {
            Ok(owners) => apply_canonical_runtime_owners(&mut request, owners),
            Err(error) => {
                log::warn!(
                    "app data gc retained workspace-keyed data because canonical protection failed: {error}"
                );
            }
        }

        let plan = crate::usecase::app_data_gc::plan_startup_gc(request);
        let revalidated_runtime_protection =
            match crate::usecase::app_data_gc::load_canonical_runtime_owners(
                repository,
                live_process_session_ids.clone(),
            )
            .await
            {
                Ok(owners) => canonical_runtime_protection(owners),
                Err(error) => {
                    log::warn!(
                        "app data gc retained workspace-keyed candidates because sweep-boundary canonical revalidation failed: {error}"
                    );
                    crate::usecase::app_data_gc::RuntimeProtection::incomplete(
                        live_process_session_ids,
                    )
                }
            };

        tokio::task::spawn_blocking(move || {
            crate::usecase::app_data_gc::sweep_startup_gc(
                plan,
                revalidated_runtime_protection,
                &file_system,
            )
        })
        .await
        .map_err(|error| format!("app data gc sweep task failed: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, SystemTime};

    use parking_lot::RwLock;
    use sha2::{Digest, Sha256};

    use super::*;
    use crate::domain::local_event::{
        AgentSessionMetadataRecord, AgentSessionProjectionRecord, AgentSessionStateRecord,
        CanonicalRuntimeOwnerView, CommitBatchError, CommitBatchResult, CommitIdentity,
        CommitOperationKind, CommitResolution, DomainEventPage, GlobalSequence, IdempotencyBinding,
        LocalAtomicBatch, LocalEventQuery, LocalEventQueryError, LocalEventQueryResult,
        LocalEventSubscription, LocalStateMutation, Revision, RevisionGuard,
        SessionProjectionMutation, SessionProjectionRecord, WorkflowWorktreeOwnerRecord,
    };
    use crate::infrastructure::app_data_path::AppDataPathOperation;

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
        Commit(LocalAtomicBatch),
        FailRevalidation,
        ReturnWrongShape,
        ReturnOversizeSnapshot(Vec<CanonicalRuntimeOwnerView>),
    }

    struct OwnerRaceRepository {
        inner: Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>,
        action: Mutex<Option<OwnerQueryAction>>,
        action_at_owner_query: usize,
        owner_query_calls: AtomicUsize,
        paged_owner_query_calls: AtomicUsize,
        committed_action_count: AtomicUsize,
    }

    impl OwnerRaceRepository {
        fn new(
            inner: Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>,
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
            inner: Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>,
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

        fn without_sweep_action(
            inner: Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>,
        ) -> Self {
            Self {
                inner,
                action: Mutex::new(None),
                action_at_owner_query: usize::MAX,
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
            events: &[crate::domain::local_event::UncommittedDomainEvent],
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
            request: crate::domain::local_event::LoadStreamRequest,
        ) -> Result<DomainEventPage, LocalEventQueryError> {
            self.inner.load_stream(request).await
        }

        async fn query(
            &self,
            request: LocalEventQuery,
        ) -> Result<LocalEventQueryResult, LocalEventQueryError> {
            if matches!(request, LocalEventQuery::SessionProjectionPage { .. }) {
                self.paged_owner_query_calls.fetch_add(1, Ordering::SeqCst);
                return self.inner.query(request).await;
            }
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
                        OwnerQueryAction::Commit(batch) => {
                            self.inner
                                .commit_batch(batch)
                                .await
                                .expect("projection committed after candidate planning");
                            self.committed_action_count.fetch_add(1, Ordering::SeqCst);
                        }
                        OwnerQueryAction::FailRevalidation => {
                            return Err(LocalEventQueryError::Internal {
                                correlation_id: "gc-revalidation-failure".to_string(),
                            });
                        }
                        OwnerQueryAction::ReturnWrongShape => {
                            return Ok(LocalEventQueryResult::SessionProjectionPage(Vec::new()));
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

        fn subscribe(&self, after: GlobalSequence) -> LocalEventSubscription {
            self.inner.subscribe(after)
        }
    }

    fn projection_commit(
        installation_id: &str,
        suffix: &str,
        session_id: String,
        projection: SessionProjectionRecord,
    ) -> LocalAtomicBatch {
        LocalAtomicBatch {
            commit_id: CommitIdentity::parse(&format!("gc-race-commit-{suffix}"))
                .expect("commit identity"),
            idempotency: IdempotencyBinding {
                installation_id: installation_id.to_string(),
                operation_kind: CommitOperationKind::Projection,
                idempotency_key: format!("gc-race-key-{suffix}"),
                payload_hash: [37; 32],
            },
            expected_heads: Vec::new(),
            events: Vec::new(),
            state_mutations: vec![LocalStateMutation::SessionProjection(
                SessionProjectionMutation {
                    session_id,
                    projection,
                    expected: RevisionGuard::Absent,
                    revision: Revision::new(0).expect("initial projection revision"),
                },
            )],
        }
    }

    fn active_session_projection(session_id: &str, worktree_path: &str) -> SessionProjectionRecord {
        SessionProjectionRecord::AgentSession(Box::new(AgentSessionProjectionRecord {
            meta: AgentSessionMetadataRecord {
                id: session_id.to_string(),
                worktree_path: worktree_path.to_string(),
                state: AgentSessionStateRecord::Active,
                error_reason: None,
                state_revision: 0,
                created_at_bits: 0,
                updated_at_bits: 0,
                agent_session_id: None,
                provider_session_generation: 0,
                provider_session_observation_id: None,
                context_reinjection_generation: None,
                context_carry: None,
                pending_recovery_message: None,
                recovery_publication_snapshot: None,
                permission_mode: "ask".to_string(),
                plan_mode: false,
                selected_model: None,
                permission_profile_id: None,
                backend_id: "codex".to_string(),
                workflow_node_session: false,
                workflow_node_context: None,
                workflow_instructions: Vec::new(),
                agent_read_paths: None,
                context_epoch: None,
                last_turn_interruption: None,
                last_turn_id: None,
                first_message_preview: String::new(),
                message_count: 0,
                body_format_version: 1,
            },
            title: None,
            reducer_events: Vec::new(),
            queue_paused_at_bits: None,
            latest_token_usage: None,
            pending_send_queue: Vec::new(),
        }))
    }

    fn running_workflow_projection(worktree_path: &str) -> (String, SessionProjectionRecord) {
        let key = format!(
            "workflow-worktree:{}",
            hex::encode(Sha256::digest(worktree_path.as_bytes()))
        );
        (
            key,
            SessionProjectionRecord::WorkflowWorktreeOwner(WorkflowWorktreeOwnerRecord {
                worktree_path: worktree_path.to_string(),
                execution_id: "gc-race-running-workflow".to_string(),
                active: true,
            }),
        )
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
            crate::adaptor::gateway::workspace_state::repository_impl::storage_key(worktree_path);
        let review_key = crate::adaptor::gateway::comment::worktree_storage_key(worktree_path);
        let workspace_state = app_data_dir
            .join("workspace_state")
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
        crate::adaptor::gateway::repository::repo_paths::SharedRepoPaths,
    ) {
        let live_repo = tempfile::tempdir().expect("live repo");
        git2::Repository::init(live_repo.path()).expect("initialize live repo");
        let repo_paths = Arc::new(RwLock::new(vec![live_repo
            .path()
            .to_string_lossy()
            .into_owned()]));
        (live_repo, repo_paths)
    }

    async fn assert_projection_committed_at_sweep_boundary_is_protected(
        suffix: &str,
        projection_id: String,
        projection: SessionProjectionRecord,
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
        let commit = projection_commit(store.installation_id(), suffix, projection_id, projection);
        let repository = Arc::new(OwnerRaceRepository::new(
            store,
            OwnerQueryAction::Commit(commit),
        ));
        let (workspace_state, review_comments) =
            create_workspace_keyed_candidates(app_data.path(), protected_worktree);
        let (_live_repo, repo_paths) = live_repo_paths();
        observer
            .operations
            .lock()
            .expect("recorded operations")
            .clear();

        let report = composition
            .run_startup_gc_pass(repo_paths, repository.clone())
            .await
            .expect("run startup GC");

        assert_eq!(report.errors, 0);
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
    async fn app_data_gc_active_session_committed_after_plan_is_revalidated_before_remove() {
        let worktree = "/deleted-before-gc-but-session-became-active";
        assert_projection_committed_at_sweep_boundary_is_protected(
            "active-session",
            "gc-race-active-session".to_string(),
            active_session_projection("gc-race-active-session", worktree),
            worktree,
        )
        .await;
    }

    #[tokio::test]
    async fn app_data_gc_running_workflow_committed_after_plan_is_revalidated_before_remove() {
        let worktree = "/deleted-before-gc-but-workflow-became-running";
        let (projection_id, projection) = running_workflow_projection(worktree);
        assert_projection_committed_at_sweep_boundary_is_protected(
            "running-workflow",
            projection_id,
            projection,
            worktree,
        )
        .await;
    }

    #[tokio::test]
    async fn app_data_gc_revalidation_query_failure_retains_workspace_keyed_candidates() {
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
        std::fs::write(
            &stale_process_record,
            serde_json::to_vec(&crate::infrastructure::process::pid_registry::PidFileV1 {
                version: 1,
                session_id: "stale-process-session".to_string(),
                backend_id: "codex".to_string(),
                pid: u32::MAX,
                pgid: i32::MAX,
                owner_app_pid: None,
                owner_start_time: None,
                created_at_ms: 1,
            })
            .expect("stale process JSON"),
        )
        .expect("stale process fixture");
        let (_live_repo, repo_paths) = live_repo_paths();

        let report = composition
            .run_startup_gc_pass(repo_paths, repository.clone())
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
            !stale_process_record.exists(),
            "owner-independent stale-process cleanup must remain active"
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
    async fn app_data_gc_unresolved_live_pid_owner_retains_all_workspace_keyed_candidates() {
        let app_data = tempfile::tempdir().expect("app data");
        let observer = Arc::new(RecordingObserver::default());
        let composition = ProductionAppDataComposition::with_observer(
            app_data.path().to_path_buf(),
            observer.clone(),
        );
        let store = composition
            .open_local_event_store()
            .expect("open canonical store");
        let repository = Arc::new(OwnerRaceRepository::without_sweep_action(store));
        let (workspace_state, review_comments) = create_workspace_keyed_candidates(
            app_data.path(),
            "/deleted-worktree-with-unresolved-live-pid-owner",
        );
        let process_directory = app_data.path().join("agent-processes");
        std::fs::create_dir(&process_directory).expect("process directory");
        let process_record = process_directory.join("unresolved.codex.current.json");
        std::fs::write(
            &process_record,
            serde_json::to_vec(&crate::infrastructure::process::pid_registry::PidFileV1 {
                version: 1,
                session_id: "canonical-projection-does-not-exist".to_string(),
                backend_id: "codex".to_string(),
                pid: std::process::id(),
                pgid: std::process::id() as i32,
                owner_app_pid: None,
                owner_start_time: None,
                created_at_ms: 1,
            })
            .expect("live process record JSON"),
        )
        .expect("live process record");
        let (_live_repo, repo_paths) = live_repo_paths();
        observer
            .operations
            .lock()
            .expect("recorded operations")
            .clear();

        let report = composition
            .run_startup_gc_pass(repo_paths, repository.clone())
            .await
            .expect("run startup GC");

        assert_eq!(report.errors, 0);
        assert!(workspace_state.exists());
        assert!(review_comments.exists());
        assert!(process_record.exists(), "live process record remains live");
        assert!(
            repository.owner_query_calls() >= 2,
            "unresolved live PID projection must fail both initial and fresh owner snapshots"
        );
        assert_eq!(repository.paged_owner_query_calls(), 0);
        let operations = observer.operations.lock().expect("recorded operations");
        assert_eq!(
            operations
                .iter()
                .filter(|(operation, _)| *operation == AppDataPathOperation::Remove)
                .count(),
            0,
            "unresolved live PID projection makes all available candidates non-removable"
        );
    }

    #[tokio::test]
    async fn app_data_gc_oversize_revalidation_snapshot_retains_workspace_keyed_candidates() {
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
            .run_startup_gc_pass(repo_paths, repository.clone())
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
    async fn app_data_gc_wrong_shape_initial_snapshot_retains_workspace_keyed_candidates() {
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
            .run_startup_gc_pass(repo_paths, repository.clone())
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
}
