use crate::domain::workspace_state::WorkspaceStateKey;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use crate::domain::app_data_gc::{
    is_expired, GcCategory, GcNotice, GcReport, GcSkipReason, RetentionPolicy,
};
use crate::domain::local_event::{
    CanonicalRuntimeOwnerView, LocalEventQuery, LocalEventQueryResult,
    LocalEventTransactionRepository,
};

pub(crate) async fn inventory_startup_gc(
    states: &dyn crate::domain::workspace_state::WorkspaceStateRepository,
    live_inventory: impl std::future::Future<Output = Result<StartupGcRequest, String>>,
) -> Result<
    (
        Result<
            Vec<(WorkspaceStateKey, crate::domain::local_event::StreamVersion)>,
            crate::domain::workspace_state::WorkspaceStateError,
        >,
        StartupGcRequest,
    ),
    String,
> {
    let workspace_inventory = states.states().await;
    let request = live_inventory.await?;
    Ok((workspace_inventory, request))
}

pub(crate) async fn sweep_workspace_states(
    plan: &StartupGcPlan,
    runtime: &RuntimeProtection,
    states: &dyn crate::domain::workspace_state::WorkspaceStateRepository,
    inventory: Result<
        Vec<(WorkspaceStateKey, crate::domain::local_event::StreamVersion)>,
        crate::domain::workspace_state::WorkspaceStateError,
    >,
    report: &mut GcReport,
) {
    let request = &plan.request;
    let inventory = match inventory {
        Ok(states) => states,
        Err(error) => {
            report.record_error();
            report
                .notices
                .push(GcNotice::WorkspaceInventoryFailed(error.to_string()));
            return;
        }
    };
    for (key, expected) in inventory {
        if !can_remove_workspace_state(request, runtime, &key) {
            continue;
        }
        match states.invalidate(&key, expected).await {
            Ok(true) => report.record_deleted(GcCategory::DeletedWorkspace, 0),
            Ok(false) => {}
            Err(error) => {
                report.record_error();
                report.notices.push(GcNotice::WorkspaceInvalidationFailed {
                    key,
                    error: error.to_string(),
                });
            }
        }
    }
}

#[async_trait::async_trait]
pub trait ExecutionTreeGc: Send + Sync {
    async fn execution_trees(
        &self,
        after: Option<&str>,
    ) -> Result<
        Vec<crate::domain::workflow::ExecutionTreeArchiveCandidate>,
        crate::domain::workflow::WorkflowError,
    >;
    async fn record_repository_root(
        &self,
        execution_id: &str,
        root: &str,
    ) -> Result<(), crate::domain::workflow::WorkflowError>;
    async fn archive_removed_tree(
        &self,
        execution_id: &str,
    ) -> Result<(), crate::domain::workflow::WorkflowError>;
}

pub async fn archive_removed_execution_trees(
    resolution: Option<&LiveWorktreeResolution>,
    trees: &dyn ExecutionTreeGc,
) -> Result<GcReport, StartupGcFailure> {
    let Some(resolution) = resolution else {
        return Ok(GcReport::default());
    };
    let mut report = GcReport::default();
    let mut after = None;
    loop {
        let page = match trees.execution_trees(after.as_deref()).await {
            Ok(page) => page,
            Err(error) => {
                return Err(StartupGcFailure {
                    report,
                    message: format!("execution tree GC failed: {error}"),
                })
            }
        };
        let Some(last) = page.last() else { break };
        after = Some(last.execution_id.clone());
        for tree in page {
            let repository_root = tree
                .repository_root
                .as_deref()
                .map(worktree_path_key)
                .or_else(|| {
                    resolution
                        .worktree_repositories
                        .get(&worktree_path_key(&tree.worktree_path))
                        .cloned()
                })
                .or_else(|| {
                    crate::domain::app_data_gc::repository_for_worktree(
                        &worktree_path_key(&tree.workspace_identity),
                        &worktree_path_key(&tree.worktree_path),
                        &resolution.repository_paths,
                    )
                    .map(str::to_string)
                });
            if let Some(root) = &repository_root {
                if let Err(error) = trees.record_repository_root(&tree.execution_id, root).await {
                    report.record_error();
                    report.notices.push(GcNotice::RepositoryRecordFailed {
                        execution_id: tree.execution_id.clone(),
                        error: error.to_string(),
                    });
                    continue;
                }
            }
            if crate::domain::app_data_gc::worktree_removed(
                &worktree_path_key(&tree.worktree_path),
                repository_root.as_deref(),
                &resolution.live_worktrees.paths,
                &resolution.unresolved_repo_paths,
            ) {
                if let Err(error) = trees.archive_removed_tree(&tree.execution_id).await {
                    report.record_error();
                    report.notices.push(GcNotice::ArchiveFailed {
                        execution_id: tree.execution_id.clone(),
                        error: error.to_string(),
                    });
                }
            }
        }
    }
    Ok(report)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GcFileType {
    File,
    Directory,
    Symlink,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GcMetadata {
    pub(crate) file_type: GcFileType,
    pub(crate) len: u64,
    pub(crate) modified_secs: Option<f64>,
}

pub trait GcFileSystem: Send + Sync {
    fn metadata(&self, path: &Path) -> Result<GcMetadata, GcFileSystemError>;
    fn read_dir(&self, path: &Path) -> Result<Vec<PathBuf>, GcFileSystemError>;
    fn remove_path(&self, path: &Path) -> Result<bool, GcFileSystemError>;
    fn recursive_size(&self, path: &Path) -> Result<u64, GcFileSystemError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GcFileSystemErrorKind {
    NotFound,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcFileSystemError {
    kind: GcFileSystemErrorKind,
    message: String,
}

impl GcFileSystemError {
    pub(crate) fn not_found(message: impl Into<String>) -> Self {
        Self {
            kind: GcFileSystemErrorKind::NotFound,
            message: message.into(),
        }
    }

    pub(crate) fn other(message: impl Into<String>) -> Self {
        Self {
            kind: GcFileSystemErrorKind::Other,
            message: message.into(),
        }
    }

    pub(crate) fn is_not_found(&self) -> bool {
        self.kind == GcFileSystemErrorKind::NotFound
    }
}

impl fmt::Display for GcFileSystemError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(formatter)
    }
}

impl From<io::Error> for GcFileSystemError {
    fn from(error: io::Error) -> Self {
        if error.kind() == io::ErrorKind::NotFound {
            Self::not_found(error.to_string())
        } else {
            Self::other(error.to_string())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveWorktree {
    pub path: String,
    pub workspace_state_keys: Vec<WorkspaceStateKey>,
    pub review_comment_keys: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LiveWorktreeSet {
    paths: HashSet<String>,
    workspace_state_keys: HashSet<WorkspaceStateKey>,
    review_comment_keys: HashSet<String>,
}

impl LiveWorktreeSet {
    pub fn from_worktrees(worktrees: impl IntoIterator<Item = LiveWorktree>) -> Self {
        let mut result = Self::default();
        for worktree in worktrees {
            result.paths.insert(worktree_path_key(&worktree.path));
            result
                .workspace_state_keys
                .extend(worktree.workspace_state_keys);
            result
                .review_comment_keys
                .extend(worktree.review_comment_keys);
        }
        result
    }

    fn contains_workspace_state_key(&self, key: &WorkspaceStateKey) -> bool {
        self.workspace_state_keys.contains(key)
    }

    fn contains_review_comment_key(&self, key: &str) -> bool {
        self.review_comment_keys.contains(key)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveWorktreeResolution {
    live_worktrees: LiveWorktreeSet,
    unresolved_repo_paths: Vec<String>,
    repository_paths: Vec<String>,
    worktree_repositories: HashMap<String, String>,
    unresolved_workspace_state_keys: HashSet<WorkspaceStateKey>,
}

impl LiveWorktreeResolution {
    pub fn new(
        live_worktrees: LiveWorktreeSet,
        unresolved_repo_paths: Vec<String>,
        unresolved_workspace_state_keys: HashSet<WorkspaceStateKey>,
    ) -> Self {
        Self {
            repository_paths: Vec::new(),
            worktree_repositories: HashMap::new(),
            live_worktrees,
            unresolved_repo_paths: unresolved_repo_paths
                .into_iter()
                .map(|path| worktree_path_key(&path))
                .collect(),
            unresolved_workspace_state_keys,
        }
    }

    pub fn with_repository_paths(mut self, paths: Vec<String>) -> Self {
        self.repository_paths = paths
            .into_iter()
            .map(|path| worktree_path_key(&path))
            .collect();
        self
    }

    pub(crate) fn with_worktree_repositories(mut self, owners: HashMap<String, String>) -> Self {
        self.worktree_repositories = owners;
        self
    }

    fn contains_workspace_state_key(&self, key: &WorkspaceStateKey) -> bool {
        self.live_worktrees.contains_workspace_state_key(key)
    }

    fn contains_review_comment_key(&self, key: &str) -> bool {
        self.live_worktrees.contains_review_comment_key(key)
    }

    fn workspace_state_key_may_be_unresolved(&self, key: &WorkspaceStateKey) -> bool {
        self.unresolved_workspace_state_keys.contains(key)
    }

    fn has_unresolved_repos(&self) -> bool {
        !self.unresolved_repo_paths.is_empty()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CanonicalRuntimeOwners {
    pub(crate) protected_worktree_paths: HashSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeProtection {
    protected_worktrees: LiveWorktreeSet,
    pub(crate) workspace_keyed_protection_complete: bool,
}

impl RuntimeProtection {
    pub(crate) fn complete(protected_worktrees: LiveWorktreeSet) -> Self {
        Self {
            protected_worktrees,
            workspace_keyed_protection_complete: true,
        }
    }

    pub(crate) fn incomplete() -> Self {
        Self {
            protected_worktrees: LiveWorktreeSet::default(),
            workspace_keyed_protection_complete: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkspaceStateGcRecord {
    pub(crate) path: PathBuf,
    pub(crate) key: WorkspaceStateKey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReviewCommentGcRecord {
    pub(crate) path: PathBuf,
    pub(crate) key: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CacheGcRecord {
    pub path: PathBuf,
    pub updated_at: f64,
}

#[derive(Debug, Clone)]
pub struct StartupGcRequest {
    pub(crate) app_data_dir: PathBuf,
    pub live_worktrees: Option<LiveWorktreeResolution>,
    pub(crate) workspace_state_records: Vec<WorkspaceStateGcRecord>,
    pub(crate) review_comment_records: Vec<ReviewCommentGcRecord>,
    /// Checkpoints remain retained until their worktree mapping can be proven.
    /// Keeping the observed paths in the request makes that conservative
    /// decision explicit and testable.
    pub(crate) checkpoint_paths: Vec<PathBuf>,
    pub cache_records: Vec<CacheGcRecord>,
    pub(crate) legacy_comment_paths: Vec<PathBuf>,
    pub(crate) runtime_protection: RuntimeProtection,
    pub now_secs: f64,
    pub retention: RetentionPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CandidateRevalidation {
    None,
    WorkspaceState { key: WorkspaceStateKey },
    ReviewComment { key: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DeletionCandidate {
    path: PathBuf,
    category: GcCategory,
    revalidation: CandidateRevalidation,
}

#[derive(Debug)]
struct DeletionPlan {
    app_data_dir: PathBuf,
    candidates: VecDeque<DeletionCandidate>,
    paths: HashSet<PathBuf>,
}

impl DeletionPlan {
    fn new(app_data_dir: PathBuf) -> Self {
        Self {
            app_data_dir,
            candidates: VecDeque::new(),
            paths: HashSet::new(),
        }
    }

    fn add(&mut self, path: PathBuf, category: GcCategory, revalidation: CandidateRevalidation) {
        if self.paths.insert(path.clone()) {
            self.candidates.push_back(DeletionCandidate {
                path,
                category,
                revalidation,
            });
        }
    }
}

/// Candidate set frozen from the initial inventory and owner snapshot.
///
/// Production must obtain a separate canonical owner snapshot after this
/// value is created and pass it to [`sweep_startup_gc`].
pub(crate) struct StartupGcPlan {
    request: StartupGcRequest,
    deletions: DeletionPlan,
    report: GcReport,
}

/// Reads the canonical SQLite projection inventory only. No legacy
/// Session/Workflow path participates in runtime-protection construction.
pub(crate) async fn load_canonical_runtime_owners(
    repository: Arc<dyn LocalEventTransactionRepository>,
) -> Result<CanonicalRuntimeOwners, String> {
    const SNAPSHOT_LIMIT: usize = 8_192;

    let mut owners = CanonicalRuntimeOwners::default();
    let response = repository
        .query(LocalEventQuery::CanonicalRuntimeOwnerSnapshot {
            limit: SNAPSHOT_LIMIT,
        })
        .await
        .map_err(|error| format!("canonical runtime protection query failed: {error}"))?;
    let LocalEventQueryResult::CanonicalRuntimeOwnerSnapshot(snapshot) = response else {
        return Err("canonical runtime protection query returned the wrong shape".to_string());
    };
    if snapshot.len() > SNAPSHOT_LIMIT {
        return Err(
            "canonical runtime protection query exceeded the requested snapshot limit".to_string(),
        );
    }
    for owner in snapshot {
        apply_runtime_owner(owner, &mut owners);
    }
    Ok(owners)
}

fn apply_runtime_owner(owner: CanonicalRuntimeOwnerView, owners: &mut CanonicalRuntimeOwners) {
    match owner {
        CanonicalRuntimeOwnerView::AgentSession {
            worktree_path,
            active,
            ..
        } => {
            if active {
                owners
                    .protected_worktree_paths
                    .insert(worktree_path_key(&worktree_path));
            }
        }
        CanonicalRuntimeOwnerView::ActiveWorkflow { worktree_path } => {
            owners
                .protected_worktree_paths
                .insert(worktree_path_key(&worktree_path));
        }
    }
}

pub(crate) fn plan_startup_gc(request: StartupGcRequest) -> StartupGcPlan {
    let mut plan = DeletionPlan::new(request.app_data_dir.clone());

    let mut report = GcReport::default();
    collect_workspace_keyed_deletions(&request, &mut plan, &mut report);
    for record in &request.cache_records {
        if is_expired(
            request.now_secs,
            record.updated_at,
            request.retention.cache_secs,
        ) {
            plan.add(
                record.path.clone(),
                GcCategory::RegenerableCache,
                CandidateRevalidation::None,
            );
        }
    }
    for path in &request.legacy_comment_paths {
        plan.add(
            path.clone(),
            GcCategory::LegacyComments,
            CandidateRevalidation::None,
        );
    }
    for path in &request.checkpoint_paths {
        report
            .notices
            .push(GcNotice::CheckpointRetained(path.clone()));
    }

    StartupGcPlan {
        request,
        deletions: plan,
        report,
    }
}

pub(crate) fn sweep_startup_gc(
    plan: StartupGcPlan,
    revalidated_runtime_protection: RuntimeProtection,
    file_system: &dyn GcFileSystem,
) -> GcReport {
    let mut report = plan.report;
    let file_report = sweep(
        plan.deletions,
        &plan.request,
        &revalidated_runtime_protection,
        file_system,
    );
    report.merge(file_report);
    report
}

#[cfg(any(test, feature = "test-support"))]
pub fn run_startup_gc(request: StartupGcRequest, file_system: &dyn GcFileSystem) -> GcReport {
    let revalidated_runtime_protection = request.runtime_protection.clone();
    let plan = plan_startup_gc(request);
    sweep_startup_gc(plan, revalidated_runtime_protection, file_system)
}

fn can_remove_workspace_state(
    request: &StartupGcRequest,
    runtime: &RuntimeProtection,
    key: &WorkspaceStateKey,
) -> bool {
    let Some(live) = request.live_worktrees.as_ref() else {
        return false;
    };
    crate::domain::app_data_gc::workspace_state_removable(
        crate::domain::app_data_gc::WorkspaceStateRetention {
            live: live.contains_workspace_state_key(key),
            protected_at_scan: request
                .runtime_protection
                .protected_worktrees
                .contains_workspace_state_key(key),
            protected_at_sweep: runtime
                .protected_worktrees
                .contains_workspace_state_key(key),
            possibly_unresolved: live.workspace_state_key_may_be_unresolved(key),
            prerequisites_complete: !live.has_unresolved_repos()
                && request
                    .runtime_protection
                    .workspace_keyed_protection_complete
                && runtime.workspace_keyed_protection_complete,
        },
    )
}

fn collect_workspace_keyed_deletions(
    request: &StartupGcRequest,
    plan: &mut DeletionPlan,
    report: &mut GcReport,
) {
    let Some(live_worktrees) = request.live_worktrees.as_ref() else {
        report.notices.push(GcNotice::RulesSkipped(
            GcSkipReason::LiveWorktreesUnavailable,
        ));
        return;
    };
    let runtime = &request.runtime_protection;
    if !runtime.workspace_keyed_protection_complete {
        report.notices.push(GcNotice::RulesSkipped(
            GcSkipReason::RuntimeProtectionIncomplete,
        ));
        return;
    }
    if live_worktrees.has_unresolved_repos() {
        report.notices.push(GcNotice::RulesSkipped(
            GcSkipReason::RepositoryWorktreesUnavailable,
        ));
        return;
    }

    for record in &request.workspace_state_records {
        if can_remove_workspace_state(request, runtime, &record.key) {
            plan.add(
                record.path.clone(),
                GcCategory::DeletedWorkspace,
                CandidateRevalidation::WorkspaceState {
                    key: record.key.clone(),
                },
            );
        }
    }
    for record in &request.review_comment_records {
        if !live_worktrees.contains_review_comment_key(&record.key)
            && !runtime
                .protected_worktrees
                .contains_review_comment_key(&record.key)
        {
            plan.add(
                record.path.clone(),
                GcCategory::DeletedWorkspace,
                CandidateRevalidation::ReviewComment {
                    key: record.key.clone(),
                },
            );
        }
    }
}

fn sweep(
    mut plan: DeletionPlan,
    request: &StartupGcRequest,
    revalidated_runtime_protection: &RuntimeProtection,
    file_system: &dyn GcFileSystem,
) -> GcReport {
    let mut report = GcReport::default();
    while let Some(candidate) = plan.candidates.pop_front() {
        if !candidate_is_contained(&plan.app_data_dir, &candidate.path) {
            report
                .notices
                .push(GcNotice::OutsideAppData(candidate.path.clone()));
            continue;
        }
        if !candidate_still_valid(&candidate, request, revalidated_runtime_protection) {
            continue;
        }
        let reclaimed_bytes = match file_system.recursive_size(&candidate.path) {
            Ok(size) => size,
            Err(error) if error.is_not_found() => 0,
            Err(error) => {
                report.notices.push(GcNotice::MeasureFailed {
                    path: candidate.path.clone(),
                    error: error.to_string(),
                });
                0
            }
        };
        match file_system.remove_path(&candidate.path) {
            Ok(true) => report.record_deleted(candidate.category, reclaimed_bytes),
            Ok(false) => {}
            Err(error) => {
                report.record_error();
                report.notices.push(GcNotice::RemoveFailed {
                    path: candidate.path.clone(),
                    error: error.to_string(),
                });
            }
        }
    }
    report
}

fn candidate_still_valid(
    candidate: &DeletionCandidate,
    request: &StartupGcRequest,
    revalidated_runtime_protection: &RuntimeProtection,
) -> bool {
    match &candidate.revalidation {
        CandidateRevalidation::None => true,
        CandidateRevalidation::WorkspaceState { key } => {
            can_remove_workspace_state(request, revalidated_runtime_protection, key)
        }
        CandidateRevalidation::ReviewComment { key } => {
            request.live_worktrees.as_ref().is_some_and(|live| {
                revalidated_runtime_protection.workspace_keyed_protection_complete
                    && !live.has_unresolved_repos()
                    && !live.contains_review_comment_key(key)
                    && !revalidated_runtime_protection
                        .protected_worktrees
                        .contains_review_comment_key(key)
            })
        }
    }
}

fn candidate_is_contained(app_data_dir: &Path, path: &Path) -> bool {
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return false;
    }
    path.starts_with(app_data_dir)
}

fn worktree_path_key(path: &str) -> String {
    let mut value = path.trim().to_string();
    while value.len() > 1 && (value.ends_with('/') || value.ends_with('\\')) {
        value.pop();
    }
    value
}

#[cfg(test)]
#[path = "mod_test.rs"]
pub(crate) mod mod_tests;

#[async_trait::async_trait]
pub trait StartupGcInventory: Send + Sync {
    async fn inventory(&self) -> Result<StartupGcRequest, String>;
    fn runtime_protection(&self, owners: CanonicalRuntimeOwners) -> RuntimeProtection;
}

#[derive(Debug)]
pub struct StartupGcFailure {
    pub report: GcReport,
    pub message: String,
}
impl std::fmt::Display for StartupGcFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.message.fmt(formatter)
    }
}
impl std::error::Error for StartupGcFailure {}

pub async fn execute_startup_gc(
    inventory: &dyn StartupGcInventory,
    repository: Arc<dyn LocalEventTransactionRepository>,
    trees: &dyn ExecutionTreeGc,
    states: &dyn crate::domain::workspace_state::WorkspaceStateRepository,
    file_system: Arc<dyn GcFileSystem>,
) -> Result<GcReport, StartupGcFailure> {
    let (workspace_inventory, mut request) = inventory_startup_gc(states, inventory.inventory())
        .await
        .map_err(|message| StartupGcFailure {
            report: GcReport::default(),
            message,
        })?;
    let mut report =
        archive_removed_execution_trees(request.live_worktrees.as_ref(), trees).await?;
    match load_canonical_runtime_owners(repository.clone()).await {
        Ok(owners) => request.runtime_protection = inventory.runtime_protection(owners),
        Err(error) => report.notices.push(GcNotice::CanonicalProtectionFailed {
            revalidation: false,
            error,
        }),
    }
    let mut plan = plan_startup_gc(request);
    let runtime = match load_canonical_runtime_owners(repository).await {
        Ok(owners) => inventory.runtime_protection(owners),
        Err(error) => {
            report.notices.push(GcNotice::CanonicalProtectionFailed {
                revalidation: true,
                error,
            });
            RuntimeProtection::incomplete()
        }
    };
    report.merge(std::mem::take(&mut plan.report));
    sweep_workspace_states(&plan, &runtime, states, workspace_inventory, &mut report).await;
    let files = match crate::common::operation_context::spawn_blocking(move || {
        sweep_startup_gc(plan, runtime, file_system.as_ref())
    })
    .await
    {
        Ok(files) => files,
        Err(error) => {
            return Err(StartupGcFailure {
                report,
                message: format!("app data gc sweep task failed: {error}"),
            })
        }
    };
    report.merge(files);
    Ok(report)
}
