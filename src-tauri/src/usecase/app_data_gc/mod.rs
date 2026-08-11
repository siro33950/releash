use std::collections::{HashSet, VecDeque};
use std::fmt;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use crate::domain::app_data_gc::{is_expired, GcCategory, GcReport, RetentionPolicy};
use crate::domain::local_event::{
    CanonicalRuntimeOwnerView, LocalEventQuery, LocalEventQueryResult,
    LocalEventTransactionRepository,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GcFileType {
    File,
    Directory,
    Symlink,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct GcMetadata {
    pub(crate) file_type: GcFileType,
    pub(crate) len: u64,
    pub(crate) modified_secs: Option<f64>,
}

pub(crate) trait GcFileSystem: Send + Sync {
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
pub(crate) struct GcFileSystemError {
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
pub(crate) struct LiveWorktree {
    pub(crate) path: String,
    pub(crate) workspace_state_keys: Vec<String>,
    pub(crate) review_comment_keys: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct LiveWorktreeSet {
    paths: HashSet<String>,
    workspace_state_keys: HashSet<String>,
    review_comment_keys: HashSet<String>,
}

impl LiveWorktreeSet {
    pub(crate) fn from_worktrees(worktrees: impl IntoIterator<Item = LiveWorktree>) -> Self {
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

    fn contains_workspace_state_key(&self, key: &str) -> bool {
        self.workspace_state_keys.contains(key)
    }

    fn contains_review_comment_key(&self, key: &str) -> bool {
        self.review_comment_keys.contains(key)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LiveWorktreeResolution {
    live_worktrees: LiveWorktreeSet,
    unresolved_repo_paths: Vec<String>,
    unresolved_workspace_state_key_prefixes: HashSet<String>,
}

impl LiveWorktreeResolution {
    pub(crate) fn new(
        live_worktrees: LiveWorktreeSet,
        unresolved_repo_paths: Vec<String>,
        unresolved_workspace_state_key_prefixes: HashSet<String>,
    ) -> Self {
        Self {
            live_worktrees,
            unresolved_repo_paths: unresolved_repo_paths
                .into_iter()
                .map(|path| worktree_path_key(&path))
                .collect(),
            unresolved_workspace_state_key_prefixes,
        }
    }

    fn contains_workspace_state_key(&self, key: &str) -> bool {
        self.live_worktrees.contains_workspace_state_key(key)
    }

    fn contains_review_comment_key(&self, key: &str) -> bool {
        self.live_worktrees.contains_review_comment_key(key)
    }

    fn workspace_state_key_may_be_unresolved(&self, key: &str) -> bool {
        self.unresolved_workspace_state_key_prefixes
            .iter()
            .any(|prefix| prefix_matches_at_boundary(key, prefix, &['_']))
    }

    fn has_unresolved_repos(&self) -> bool {
        !self.unresolved_repo_paths.is_empty()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CanonicalRuntimeOwners {
    pub(crate) protected_worktree_paths: HashSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimeProtection {
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
    pub(crate) key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReviewCommentGcRecord {
    pub(crate) path: PathBuf,
    pub(crate) key: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CacheGcRecord {
    pub(crate) path: PathBuf,
    pub(crate) updated_at: f64,
}

#[derive(Debug, Clone)]
pub(crate) struct StartupGcRequest {
    pub(crate) app_data_dir: PathBuf,
    pub(crate) live_worktrees: Option<LiveWorktreeResolution>,
    pub(crate) workspace_state_records: Vec<WorkspaceStateGcRecord>,
    pub(crate) review_comment_records: Vec<ReviewCommentGcRecord>,
    /// Checkpoints remain retained until their worktree mapping can be proven.
    /// Keeping the observed paths in the request makes that conservative
    /// decision explicit and testable.
    pub(crate) checkpoint_paths: Vec<PathBuf>,
    pub(crate) cache_records: Vec<CacheGcRecord>,
    pub(crate) legacy_comment_paths: Vec<PathBuf>,
    pub(crate) runtime_protection: RuntimeProtection,
    pub(crate) now_secs: f64,
    pub(crate) retention: RetentionPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CandidateRevalidation {
    None,
    WorkspaceState { key: String },
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

    collect_workspace_keyed_deletions(&request, &mut plan);
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
        log::info!(
            "app data gc retained checkpoint root {} because its worktree mapping is not verified",
            path.display()
        );
    }

    StartupGcPlan {
        request,
        deletions: plan,
    }
}

pub(crate) fn sweep_startup_gc(
    plan: StartupGcPlan,
    revalidated_runtime_protection: RuntimeProtection,
    file_system: &dyn GcFileSystem,
) -> GcReport {
    let report = sweep(
        plan.deletions,
        &plan.request,
        &revalidated_runtime_protection,
        file_system,
    );
    log::info!("{}", report.log_summary());
    report
}

#[cfg(test)]
pub(crate) fn run_startup_gc(
    request: StartupGcRequest,
    file_system: &dyn GcFileSystem,
) -> GcReport {
    let revalidated_runtime_protection = request.runtime_protection.clone();
    let plan = plan_startup_gc(request);
    sweep_startup_gc(plan, revalidated_runtime_protection, file_system)
}

fn collect_workspace_keyed_deletions(request: &StartupGcRequest, plan: &mut DeletionPlan) {
    let Some(live_worktrees) = request.live_worktrees.as_ref() else {
        log::info!(
            "app data gc skipped workspace-dependent rules because live worktrees were unavailable"
        );
        return;
    };
    let runtime = &request.runtime_protection;
    if !runtime.workspace_keyed_protection_complete {
        log::info!(
            "app data gc skipped workspace-keyed cleanup because canonical runtime protection was incomplete"
        );
        return;
    }
    if live_worktrees.has_unresolved_repos() {
        log::info!(
            "app data gc skipped workspace-keyed cleanup because some repo worktrees were unavailable"
        );
        return;
    }

    for record in &request.workspace_state_records {
        if !live_worktrees.contains_workspace_state_key(&record.key)
            && !runtime
                .protected_worktrees
                .contains_workspace_state_key(&record.key)
            && !live_worktrees.workspace_state_key_may_be_unresolved(&record.key)
        {
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
            log::warn!(
                "app data gc skipped candidate outside app data: {}",
                candidate.path.display()
            );
            continue;
        }
        if !candidate_still_valid(&candidate, request, revalidated_runtime_protection) {
            continue;
        }
        let reclaimed_bytes = match file_system.recursive_size(&candidate.path) {
            Ok(size) => size,
            Err(error) if error.is_not_found() => 0,
            Err(error) => {
                log::warn!(
                    "app data gc could not measure {}: {error}",
                    candidate.path.display()
                );
                0
            }
        };
        match file_system.remove_path(&candidate.path) {
            Ok(true) => report.record_deleted(candidate.category, reclaimed_bytes),
            Ok(false) => {}
            Err(error) => {
                report.record_error();
                log::warn!(
                    "app data gc failed to remove {}: {error}",
                    candidate.path.display()
                );
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
            request.live_worktrees.as_ref().is_some_and(|live| {
                revalidated_runtime_protection.workspace_keyed_protection_complete
                    && !live.has_unresolved_repos()
                    && !live.contains_workspace_state_key(key)
                    && !revalidated_runtime_protection
                        .protected_worktrees
                        .contains_workspace_state_key(key)
                    && !live.workspace_state_key_may_be_unresolved(key)
            })
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

fn prefix_matches_at_boundary(value: &str, prefix: &str, boundary_chars: &[char]) -> bool {
    value == prefix
        || (!prefix.is_empty()
            && value
                .strip_prefix(prefix)
                .and_then(|suffix| suffix.chars().next())
                .is_some_and(|character| boundary_chars.contains(&character)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn live_worktree(path: &str, key: &str) -> LiveWorktree {
        LiveWorktree {
            path: path.to_string(),
            workspace_state_keys: vec![key.to_string()],
            review_comment_keys: vec![key.to_string()],
        }
    }

    #[test]
    fn canonical_projection_owners_protect_active_session_and_running_workflow_paths() {
        let mut owners = CanonicalRuntimeOwners::default();
        apply_runtime_owner(
            CanonicalRuntimeOwnerView::AgentSession {
                projection_id: "active-session".to_string(),
                session_id: "active-session".to_string(),
                worktree_path: "/worktrees/active".to_string(),
                active: true,
                shutdown_target: true,
                workflow_node_session: false,
            },
            &mut owners,
        );
        apply_runtime_owner(
            CanonicalRuntimeOwnerView::ActiveWorkflow {
                worktree_path: "/worktrees/running".to_string(),
            },
            &mut owners,
        );

        assert!(owners
            .protected_worktree_paths
            .contains("/worktrees/active"));
        assert!(owners
            .protected_worktree_paths
            .contains("/worktrees/running"));
    }

    #[test]
    fn workspace_cleanup_is_closed_when_runtime_projection_is_incomplete() {
        let app_data = PathBuf::from("/app-data");
        let mut request = StartupGcRequest {
            app_data_dir: app_data.clone(),
            live_worktrees: Some(LiveWorktreeResolution::new(
                LiveWorktreeSet::from_worktrees([live_worktree("/live", "live")]),
                Vec::new(),
                HashSet::new(),
            )),
            workspace_state_records: vec![WorkspaceStateGcRecord {
                path: app_data.join("workspace_state/stale.json"),
                key: "stale".to_string(),
            }],
            review_comment_records: Vec::new(),
            checkpoint_paths: Vec::new(),
            cache_records: Vec::new(),
            legacy_comment_paths: Vec::new(),
            runtime_protection: RuntimeProtection::incomplete(),
            now_secs: 0.0,
            retention: RetentionPolicy::default(),
        };
        let mut plan = DeletionPlan::new(app_data);
        collect_workspace_keyed_deletions(&request, &mut plan);
        assert!(plan.candidates.is_empty());

        request.runtime_protection = RuntimeProtection::complete(LiveWorktreeSet::default());
        collect_workspace_keyed_deletions(&request, &mut plan);
        assert_eq!(plan.candidates.len(), 1);
    }

    #[test]
    fn candidate_containment_rejects_parent_traversal_and_siblings() {
        assert!(candidate_is_contained(
            Path::new("/app-data"),
            Path::new("/app-data/lsp/old")
        ));
        assert!(!candidate_is_contained(
            Path::new("/app-data"),
            Path::new("/app-data/../outside")
        ));
        assert!(!candidate_is_contained(
            Path::new("/app-data"),
            Path::new("/app-data-other/file")
        ));
    }
}
