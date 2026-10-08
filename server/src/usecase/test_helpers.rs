use crate::domain::repository::file_watcher::FileWatchGateway;
use crate::domain::repository::GitConfigRepository;
use crate::domain::repository::RepoLocator;
use crate::domain::repository::StatusRepository;
use crate::domain::repository::WorktreeRepository;
use crate::domain::repository::WorktreeTerminalGateway;
use crate::test_support::client_api_acceptance::Branch;
use crate::test_support::client_api_acceptance::BranchRepository;
use std::sync::Arc;

use crate::domain::repository::file_watcher::WatchChangeHandler;
use std::sync::Mutex as StdMutex;

#[derive(Default)]
pub struct Files(pub StdMutex<Vec<String>>);
impl FileWatchGateway for Files {
    fn start_tree(&self, path: &str, _on_change: WatchChangeHandler) -> Result<u64, String> {
        self.0.lock().unwrap().push(path.into());
        if path == "/missing" {
            Err("missing path".into())
        } else {
            Ok(42)
        }
    }
    fn stop(&self, watcher_id: u64) -> Result<(), String> {
        self.0.lock().unwrap().push(watcher_id.to_string());
        if watcher_id == 42 {
            Ok(())
        } else {
            Err("unknown watcher".into())
        }
    }
}

use crate::domain::repository::{RepositoryError, RepositoryStatusScan, Worktree};
use crate::usecase::repository_usecase::*;
use parking_lot::Mutex;

/// 委譲・順序・変換を検証するための記録付き手書き fake。
/// 1 つの構造体で repository ドメインの全 trait を実装する。
#[derive(Default)]
pub struct FakeRepo {
    pub current_branch: String,
    pub fail_current_branch: bool,
    pub stop_current_branch: Option<crate::common::operation_context::OperationStopped>,
    pub worktrees: Vec<Worktree>,
    pub dirty: u32,
    pub branch_base: Option<String>,
    pub fail_create_worktree: bool,
    pub fail_remove_worktree: bool,
    pub fail_cleanup: bool,
    pub remove_started: tokio::sync::Notify,
    pub remove_continue: Option<Mutex<std::sync::mpsc::Receiver<()>>>,
    pub cleanup_started: tokio::sync::Notify,
    pub cleanup_continue: Option<Mutex<std::sync::mpsc::Receiver<()>>>,
    pub fail_validate_removal: bool,
    pub operations: Arc<crate::usecase::worktree_operation::WorktreeOperations>,
    pub fail_archive: bool,
    pub archive_continue: Option<tokio::sync::Notify>,
    pub archived_worktrees: Mutex<Vec<(String, usize)>>,
    pub created_branches: Mutex<Vec<String>>,
    pub removed_worktrees: Mutex<Vec<(String, bool)>>,
    /// `kill_by_worktree` 呼び出し時の (対象 path, その時点の removed 件数)。
    pub killed_worktree_terminals: Mutex<Vec<(String, usize)>>,
    /// `remove` が返す「削除した worktree のブランチ名」。
    pub removed_branch: Option<String>,
    pub set_branch_base_override_calls: Mutex<Vec<(String, Option<String>)>>,
    pub set_releash_base_calls: Mutex<Vec<Option<String>>>,
    pub prune_calls: Mutex<Vec<Vec<String>>>,
    pub fail_main_repo_path: bool,
    pub listed_worktree_paths: Mutex<Vec<String>>,
    pub branches: Vec<Branch>,
}

#[async_trait::async_trait]
impl WorktreeExecutionArchiver for FakeRepo {
    async fn begin_worktree_deletion(
        &self,
        path: &str,
    ) -> Result<
        crate::usecase::worktree_operation::WorktreeDeletionGuard,
        crate::domain::workflow::WorkflowError,
    > {
        self.operations.delete(path).await.map_err(|error| {
            crate::domain::workflow::WorkflowError::invalid_state(error.to_string())
        })
    }
    async fn archive_worktree(
        &self,
        path: &str,
    ) -> Result<(), crate::domain::workflow::WorkflowError> {
        self.archived_worktrees
            .lock()
            .push((path.to_string(), self.removed_worktrees.lock().len()));
        if self.fail_archive {
            return Err(crate::domain::workflow::WorkflowError::external(
                "archive failed",
            ));
        }
        if let Some(ready) = &self.archive_continue {
            ready.notified().await;
        }
        Ok(())
    }
}

impl BranchRepository for FakeRepo {
    fn list(&self, _repo_path: &str) -> Result<Vec<Branch>, RepositoryError> {
        Ok(self.branches.clone())
    }
    fn current(&self, _repo_path: &str) -> Result<String, RepositoryError> {
        if let Some(stopped) = self.stop_current_branch {
            return Err(stopped.into());
        }
        if self.fail_current_branch {
            return Err(RepositoryError::External("branch unavailable".into()));
        }
        Ok(self.current_branch.clone())
    }
    fn create(&self, _repo_path: &str, branch_name: &str) -> Result<(), RepositoryError> {
        self.created_branches.lock().push(branch_name.to_string());
        Ok(())
    }
}

impl StatusRepository for FakeRepo {
    fn status_scan(&self, _repo_path: &str) -> Result<RepositoryStatusScan, RepositoryError> {
        Ok(RepositoryStatusScan {
            status: Vec::new(),
            diff_stats: Vec::new(),
            dirty_count: 0,
        })
    }
}

impl WorktreeRepository for FakeRepo {
    fn find_main_repo_path(&self, path: &str) -> Result<Option<String>, RepositoryError> {
        self.main_repo_path(path).map(Some)
    }
    fn main_repo_path(&self, _any_path: &str) -> Result<String, RepositoryError> {
        if self.fail_main_repo_path {
            return Err(RepositoryError::External(
                "main repo path is unavailable".to_string(),
            ));
        }
        Ok("/main".to_string())
    }
    fn list(&self, repo_path: &str) -> Result<Vec<Worktree>, RepositoryError> {
        self.listed_worktree_paths
            .lock()
            .push(repo_path.to_string());
        Ok(self.worktrees.clone())
    }
    fn create(
        &self,
        _repo_path: &str,
        worktree_path: &str,
        branch: &str,
        _create_branch: bool,
        _base_branch: Option<&str>,
    ) -> Result<Worktree, RepositoryError> {
        if self.fail_create_worktree {
            return Err(RepositoryError::External("boom".to_string()));
        }
        Ok(Worktree {
            name: "wt".to_string(),
            path: worktree_path.to_string(),
            branch: branch.to_string(),
            is_main: false,
            is_locked: false,
            is_merged: false,
        })
    }
    fn validate_removal(
        &self,
        _: &str,
        path: &str,
        force: bool,
    ) -> Result<String, RepositoryError> {
        if self.fail_validate_removal {
            return Err(RepositoryError::rule("worktree not found"));
        }
        let worktree = self
            .worktrees
            .iter()
            .find(|wt| wt.path == path)
            .cloned()
            .unwrap_or_else(|| wt(path, "feat", false));
        worktree.authorize_removal(force, self.dirty)?;
        Ok(path.to_string())
    }
    fn remove(
        &self,
        _repo_path: &str,
        worktree_path: &str,
        force: bool,
    ) -> Result<Option<String>, RepositoryError> {
        self.remove_started.notify_one();
        if let Some(receiver) = &self.remove_continue {
            receiver
                .lock()
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
        }
        if self.fail_remove_worktree {
            return Err(RepositoryError::External("remove failed".to_string()));
        }
        self.removed_worktrees
            .lock()
            .push((worktree_path.to_string(), force));
        Ok(self.removed_branch.clone())
    }
}

impl GitConfigRepository for FakeRepo {
    fn get_releash_base(&self, _repo_path: &str) -> Result<Option<String>, RepositoryError> {
        Ok(None)
    }
    fn set_releash_base(
        &self,
        _repo_path: &str,
        base: Option<&str>,
    ) -> Result<(), RepositoryError> {
        self.set_releash_base_calls
            .lock()
            .push(base.map(|s| s.to_string()));
        Ok(())
    }
    fn get_branch_base(
        &self,
        _repo_path: &str,
        _branch_name: &str,
    ) -> Result<Option<String>, RepositoryError> {
        Ok(self.branch_base.clone())
    }
    fn set_branch_base_override(
        &self,
        _repo_path: &str,
        branch_name: &str,
        base: Option<&str>,
    ) -> Result<(), RepositoryError> {
        if self.fail_cleanup {
            return Err(RepositoryError::External("cleanup failed".into()));
        }
        self.cleanup_started.notify_one();
        if let Some(receiver) = &self.cleanup_continue {
            receiver
                .lock()
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
        }
        self.set_branch_base_override_calls
            .lock()
            .push((branch_name.to_string(), base.map(|s| s.to_string())));
        Ok(())
    }
    fn prune_stale_branch_bases(
        &self,
        _repo_path: &str,
        existing_branches: &[String],
    ) -> Result<(), RepositoryError> {
        self.prune_calls.lock().push(existing_branches.to_vec());
        Ok(())
    }
    fn resolve_current_base_branch(
        &self,
        _path_hint: &str,
    ) -> Result<Option<String>, RepositoryError> {
        Ok(self.branch_base.clone())
    }
    fn resolve_base_commit_oid(
        &self,
        _path_hint: &str,
        _base_name: &str,
    ) -> Result<Option<String>, RepositoryError> {
        Ok(None)
    }
}

impl RepoLocator for FakeRepo {
    fn cwd(&self) -> Result<String, RepositoryError> {
        Ok("/cwd".to_string())
    }
}

impl WorktreeTerminalGateway for FakeRepo {
    fn kill_by_worktree(&self, worktree_path: &str) {
        let removed_so_far = self.removed_worktrees.lock().len();
        self.killed_worktree_terminals
            .lock()
            .push((worktree_path.to_string(), removed_so_far));
    }
}

pub fn usecase(fake: Arc<FakeRepo>) -> RepositoryUsecase {
    RepositoryUsecase::new(
        fake.clone(),
        fake.clone(),
        fake.clone(),
        fake.clone(),
        fake.clone(),
        fake.clone(),
        fake.operations.clone(),
    )
}

pub fn wt(path: &str, branch: &str, is_main: bool) -> Worktree {
    Worktree {
        name: "n".to_string(),
        path: path.to_string(),
        branch: branch.to_string(),
        is_main,
        is_locked: false,
        is_merged: false,
    }
}

impl FakeRepo {
    pub async fn wait_for_deletion(&self, path: &str) {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while self.operations.mutate(path).is_err() {
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("worktree deletion did not finish");
    }
}
#[cfg(test)]
pub fn deleting_rows(repository: &RepositoryUsecase) -> Vec<(Worktree, bool)> {
    repository.with_deleting_worktrees("/main", Vec::new())
}

use crate::usecase::state_subscription::{
    StateReadError, StateSubscriptionUsecase, SubscriptionError, SubscriptionTarget,
};
pub async fn start_read(
    usecase: &StateSubscriptionUsecase,
    client: &str,
    raw: &str,
    cursor: Option<(&str, u64)>,
) -> Result<(), StateReadError> {
    let target = SubscriptionTarget::parse(raw).map_err(StateReadError::from_error)?;
    usecase
        .deps()
        .start_subscription(client, &target, &format!("{client}:{raw}"), cursor)
        .await
}
pub async fn stop_read(
    usecase: &StateSubscriptionUsecase,
    client: &str,
    raw: &str,
) -> Result<(), SubscriptionError> {
    usecase
        .deps()
        .stop_subscription(&format!("{client}:{raw}"))
        .await
}

#[cfg(test)]
pub struct NoopPerformance;
#[cfg(test)]
impl crate::usecase::telemetry::TerminalLaunchCompletion for NoopPerformance {
    fn finish(self: Box<Self>) {}
}
#[cfg(test)]
impl crate::usecase::telemetry::PerformanceOutput for NoopPerformance {
    fn start_terminal_launch_phase(
        &self,
        _: crate::usecase::telemetry::TerminalLaunch,
    ) -> Box<dyn crate::usecase::telemetry::TerminalLaunchCompletion> {
        Box::new(Self)
    }
}

#[cfg(test)]
pub struct TestIdentity;
#[cfg(test)]
impl crate::domain::identity::IdentityIssuer for TestIdentity {
    fn issue(&self) -> String {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        uuid::Uuid::from_u128(NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst) as u128)
            .to_string()
    }
}

#[cfg(test)]
pub struct TestCredentials;
#[cfg(test)]
impl crate::domain::provider_lifecycle::ProviderLifecycleCredentialGateway for TestCredentials {
    fn issue(&self) -> crate::domain::provider_lifecycle::IssuedProviderLifecycleCredential {
        use crate::domain::identity::IdentityIssuer;
        let binding = TestIdentity.issue();
        let capability = TestIdentity.issue();
        let hash = self.hash(&capability);
        crate::domain::provider_lifecycle::IssuedProviderLifecycleCredential::new(
            binding, capability, hash,
        )
    }
    fn hash(
        &self,
        capability: &str,
    ) -> crate::domain::provider_lifecycle::ProviderLifecycleCapabilityHash {
        use sha2::Digest;
        crate::domain::provider_lifecycle::ProviderLifecycleCapabilityHash::from_digest(
            sha2::Sha256::digest(capability.as_bytes()).into(),
        )
    }
}

pub(crate) mod state_subscription {
    use crate::usecase::state_subscription::*;
    use parking_lot::Mutex;

    #[derive(Default)]
    pub struct RecordingOutput {
        pub(crate) initial: Mutex<Vec<SubscriptionTarget>>,
        pub initial_values: Mutex<Vec<StateValue>>,
        pub update_values: Mutex<Vec<StateValue>>,
        pub failures: Mutex<Vec<(SubscriptionTarget, String)>>,
        pub(crate) fail_initial: std::sync::atomic::AtomicBool,
        pub(crate) updates: Mutex<Vec<SubscriptionTarget>>,
        pub updated: tokio::sync::Notify,
    }

    impl StateSubscriptionOutput for RecordingOutput {
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }

        fn publish_failure(
            &self,
            target: &SubscriptionTarget,
            error: StateReadError,
        ) -> Result<(), SubscriptionError> {
            self.failures
                .lock()
                .push((target.clone(), error.to_string()));
            self.updated.notify_one();
            Ok(())
        }

        fn publish_initial(
            &self,
            target: &SubscriptionTarget,
            snapshot: StateValue,
        ) -> Result<(), SubscriptionError> {
            if self.fail_initial.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(SubscriptionError::EncodingFailed);
            }
            self.initial.lock().push(target.clone());
            self.initial_values.lock().push(snapshot);
            Ok(())
        }

        fn publish(
            &self,
            target: &SubscriptionTarget,
            snapshot: StateValue,
            _: Option<StateValue>,
        ) -> Result<(), SubscriptionError> {
            self.updates.lock().push(target.clone());
            self.update_values.lock().push(snapshot);
            self.updated.notify_one();
            Ok(())
        }
    }

    #[cfg(test)]
    pub(crate) struct RecordingReads {
        pub(crate) calls: std::sync::atomic::AtomicUsize,
    }

    #[async_trait::async_trait]
    #[cfg(test)]
    impl StateSubscriptionRead for RecordingReads {
        async fn read(&self, target: &SubscriptionTarget) -> Result<StateValue, StateReadError> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(match target {
                SubscriptionTarget::RepositoryPaths => {
                    StateValue::RepositoryPaths(vec!["/repo".into()])
                }
                _ => StateValue::NodeDetail(None),
            })
        }

        fn repositories(&self) -> Vec<String> {
            vec![]
        }
    }

    #[derive(Default)]
    #[cfg(test)]
    pub(crate) struct GatedReads {
        pub(crate) reads: std::sync::atomic::AtomicUsize,
        pub(crate) external: std::sync::atomic::AtomicUsize,
        pub(crate) blocked: tokio::sync::Notify,
        pub(crate) release: tokio::sync::Notify,
    }

    #[cfg(test)]
    impl GatedReads {
        pub(crate) fn reads(&self) -> usize {
            self.reads.load(std::sync::atomic::Ordering::SeqCst)
        }
        pub(crate) fn external(&self) -> usize {
            self.external.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    #[cfg(test)]
    impl StateSubscriptionRead for GatedReads {
        async fn read(&self, _: &SubscriptionTarget) -> Result<StateValue, StateReadError> {
            if self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 1 {
                self.blocked.notify_one();
                self.release.notified().await;
            }
            Ok(StateValue::NodeDetail(None))
        }

        async fn refresh_external(&self, _: &SubscriptionTarget) -> Result<(), StateReadError> {
            self.external
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }

        fn repositories(&self) -> Vec<String> {
            vec![]
        }
    }

    #[cfg(test)]
    pub(crate) struct FailingReads {
        pub(crate) fail_read: std::sync::atomic::AtomicBool,
        pub(crate) fail_refresh: std::sync::atomic::AtomicBool,
    }

    #[async_trait::async_trait]
    #[cfg(test)]
    impl StateSubscriptionRead for FailingReads {
        async fn read(&self, _: &SubscriptionTarget) -> Result<StateValue, StateReadError> {
            if self.fail_read.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(StateReadError::from_error(SubscriptionError::UnknownTarget));
            }
            Ok(StateValue::RepositoryPaths(vec!["/repo".into()]))
        }
        async fn refresh_external(&self, _: &SubscriptionTarget) -> Result<(), StateReadError> {
            if self.fail_refresh.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(StateReadError::from_error(
                    SubscriptionError::EncodingFailed,
                ));
            }
            Ok(())
        }
        fn repositories(&self) -> Vec<String> {
            vec![]
        }
    }

    pub fn notion_target() -> SubscriptionTarget {
        SubscriptionTarget::NotionTasks(crate::usecase::notion::usecase::NotionTaskListRequest {
            path: "/repo".into(),
            count: 20,
            title: None,
            labels: Default::default(),
        })
    }

    pub struct FakeDelivery;

    impl StateSubscriptionDelivery for FakeDelivery {
        fn start(&self) -> Result<Option<usize>, StateReadError> {
            Ok(None)
        }
        fn claim(&self) -> bool {
            true
        }
        fn finish(
            &self,
            _: &std::collections::HashSet<SubscriptionTarget>,
        ) -> Result<(), SubscriptionError> {
            Ok(())
        }
    }
}

pub(crate) mod watcher {
    use crate::domain::repository::file_watcher::{FileWatchGateway, WatchChangeHandler};
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct SubscriptionFiles {
        pub(crate) next: std::sync::atomic::AtomicU64,
        pub active: Mutex<std::collections::HashSet<u64>>,
        pub(crate) fail_stop: std::sync::atomic::AtomicBool,
    }
    impl FileWatchGateway for SubscriptionFiles {
        fn start_tree(&self, path: &str, _on_change: WatchChangeHandler) -> Result<u64, String> {
            if path == "/missing" {
                return Err("missing path".into());
            }
            let id = self.next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.active.lock().unwrap().insert(id);
            Ok(id)
        }
        fn stop(&self, id: u64) -> Result<(), String> {
            if self.fail_stop.load(std::sync::atomic::Ordering::SeqCst) {
                return Err("stop failed".into());
            }
            self.active.lock().unwrap().remove(&id);
            Ok(())
        }
    }
}

#[cfg(test)]
pub(crate) mod retry {
    use crate::common::retry::RetryLimiter;
    use crate::domain::failure::{FailureKey, FailureRecord, FailureRecordRepository, WorkFailure};
    use crate::usecase::failure::FailureRecordingUsecase;
    use crate::usecase::retry::Retrying;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    pub(crate) struct FakeFailureRecords(Mutex<Vec<FailureRecord>>);

    impl FakeFailureRecords {
        pub(crate) fn records(&self, target: &str) -> Vec<FailureRecord> {
            self.0
                .lock()
                .unwrap()
                .iter()
                .filter(|record| target == "*" || record.target == target)
                .cloned()
                .collect()
        }
    }

    impl FailureRecordRepository for FakeFailureRecords {
        fn record_observed(
            &self,
            key: &FailureKey,
            failure: WorkFailure,
            requires_attention: bool,
        ) -> bool {
            let mut records = self.0.lock().unwrap();
            for record in records
                .iter_mut()
                .filter(|r| r.operation == key.operation && r.target == key.target)
            {
                record.active = false;
                record.requires_attention = false;
            }
            if let Some(record) = records.iter_mut().find(|r| {
                r.operation == key.operation && r.target == key.target && r.kind == failure.kind
            }) {
                record.count += 1;
                record.message = failure.message;
                record.active = true;
                record.requires_attention = requires_attention;
            } else {
                records.push(FailureRecord {
                    operation: key.operation.clone(),
                    target: key.target.clone(),
                    kind: failure.kind,
                    message: failure.message,
                    active: true,
                    requires_attention,
                    count: 1,
                    first_observed_ms: 0,
                    last_observed_ms: 0,
                });
            }
            requires_attention
        }
        fn record_resolved(&self, key: &FailureKey) -> bool {
            let mut changed = false;
            for record in self
                .0
                .lock()
                .unwrap()
                .iter_mut()
                .filter(|r| r.operation == key.operation && r.target == key.target)
            {
                changed |= record.requires_attention;
                record.active = false;
                record.requires_attention = false;
            }
            changed
        }
        fn attention_messages(&self, target: &str) -> Vec<String> {
            self.records(target)
                .into_iter()
                .filter(|r| r.requires_attention)
                .map(|r| r.message)
                .collect()
        }
    }

    pub(crate) fn test_retrying_with_store() -> (Arc<Retrying>, Arc<FakeFailureRecords>) {
        let store = Arc::new(FakeFailureRecords::default());
        let retrying = Retrying::new(
            Arc::new(RetryLimiter::deterministic()),
            Arc::new(FailureRecordingUsecase::new(store.clone(), None)),
        );
        (retrying, store)
    }

    pub(crate) fn test_retrying() -> Arc<Retrying> {
        test_retrying_with_store().0
    }
}
