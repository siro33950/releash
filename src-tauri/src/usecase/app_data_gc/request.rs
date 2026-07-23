use std::collections::HashSet;
use std::path::PathBuf;

use crate::domain::app_data_gc::RetentionPolicy;
#[cfg(test)]
use crate::usecase::agent_session::session::SessionState;

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
        let mut set = Self::default();
        for worktree in worktrees {
            set.paths.insert(worktree_path_key(&worktree.path));
            set.workspace_state_keys
                .extend(worktree.workspace_state_keys);
            set.review_comment_keys.extend(worktree.review_comment_keys);
        }
        set
    }

    #[cfg(test)]
    pub(super) fn contains_worktree_path(&self, worktree_path: &str) -> bool {
        self.paths.contains(&worktree_path_key(worktree_path))
    }

    pub(super) fn contains_workspace_state_key(&self, key: &str) -> bool {
        self.workspace_state_keys.contains(key)
    }

    pub(super) fn contains_review_comment_key(&self, key: &str) -> bool {
        self.review_comment_keys.contains(key)
    }
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GcWorktreePath {
    Resolved(String),
    NotFound(String),
    Unresolved(String),
}

#[cfg(test)]
impl GcWorktreePath {
    pub(crate) fn resolved(path: impl Into<String>) -> Self {
        Self::Resolved(worktree_path_key(&path.into()))
    }

    pub(crate) fn not_found(path: impl Into<String>) -> Self {
        Self::NotFound(worktree_path_key(&path.into()))
    }

    pub(crate) fn unresolved(path: impl Into<String>) -> Self {
        Self::Unresolved(worktree_path_key(&path.into()))
    }

    pub(crate) fn key(&self) -> &str {
        match self {
            Self::Resolved(path) | Self::NotFound(path) | Self::Unresolved(path) => path,
        }
    }

    pub(crate) fn is_unresolved(&self) -> bool {
        matches!(self, Self::Unresolved(_))
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
        let mut normalized_unresolved_repo_paths = Vec::new();
        for path in unresolved_repo_paths {
            normalized_unresolved_repo_paths.push(worktree_path_key(&path));
        }
        Self {
            live_worktrees,
            unresolved_repo_paths: normalized_unresolved_repo_paths,
            unresolved_workspace_state_key_prefixes,
        }
    }

    #[cfg(test)]
    pub(super) fn contains_worktree_path(&self, worktree_path: &GcWorktreePath) -> bool {
        if worktree_path.is_unresolved() {
            return false;
        }
        self.live_worktrees
            .contains_worktree_path(worktree_path.key())
    }

    pub(super) fn contains_workspace_state_key(&self, key: &str) -> bool {
        self.live_worktrees.contains_workspace_state_key(key)
    }

    pub(super) fn contains_review_comment_key(&self, key: &str) -> bool {
        self.live_worktrees.contains_review_comment_key(key)
    }

    #[cfg(test)]
    pub(super) fn worktree_path_may_be_unresolved(&self, worktree_path: &GcWorktreePath) -> bool {
        let normalized = worktree_path.key();
        self.unresolved_repo_paths
            .iter()
            .any(|repo_path| prefix_matches_at_boundary(normalized, repo_path, &['/', '\\']))
    }

    pub(super) fn workspace_state_key_may_be_unresolved(&self, key: &str) -> bool {
        self.unresolved_workspace_state_key_prefixes
            .iter()
            .any(|prefix| prefix_matches_at_boundary(key, prefix, &['_']))
    }

    pub(super) fn has_unresolved_repos(&self) -> bool {
        !self.unresolved_repo_paths.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimeProtection {
    pub(crate) active_session_ids: HashSet<String>,
    pub(crate) running_worktrees: HashSet<String>,
    pub(crate) protected_worktrees: LiveWorktreeSet,
    pub(crate) workspace_keyed_protection_complete: bool,
}

impl Default for RuntimeProtection {
    fn default() -> Self {
        Self {
            active_session_ids: HashSet::new(),
            running_worktrees: HashSet::new(),
            protected_worktrees: LiveWorktreeSet::default(),
            workspace_keyed_protection_complete: true,
        }
    }
}

impl RuntimeProtection {
    pub(crate) fn new(
        active_session_ids: HashSet<String>,
        running_worktrees: impl IntoIterator<Item = String>,
        protected_worktrees: LiveWorktreeSet,
    ) -> Self {
        Self {
            active_session_ids,
            running_worktrees: running_worktrees
                .into_iter()
                .map(|path| worktree_path_key(&path))
                .collect(),
            protected_worktrees,
            workspace_keyed_protection_complete: true,
        }
    }

    pub(crate) fn with_workspace_keyed_protection_complete(mut self, complete: bool) -> Self {
        self.workspace_keyed_protection_complete = complete;
        self
    }
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SessionGcRecord {
    pub(crate) id: String,
    pub(crate) delete_paths: Vec<PathBuf>,
    pub(crate) dir_path: Option<PathBuf>,
    pub(crate) worktree_path: Option<GcWorktreePath>,
    pub(crate) state: Option<SessionState>,
    pub(crate) updated_at: Option<f64>,
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionBlobStore {
    pub(crate) session_dir: PathBuf,
    pub(crate) messages_dir: PathBuf,
    pub(crate) tool_outputs_dir: PathBuf,
    pub(crate) attachments_dir: PathBuf,
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct WorkflowExecutionGcRecord {
    pub(crate) execution_id: String,
    pub(crate) worktree_path: GcWorktreePath,
    pub(crate) is_terminal: bool,
    pub(crate) manual_archived_at: Option<f64>,
    pub(crate) delete_paths: Vec<PathBuf>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProcessRecordStatus {
    Live,
    Stale,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProcessRecord {
    pub(crate) path: PathBuf,
    pub(crate) session_id: Option<String>,
    pub(crate) status: ProcessRecordStatus,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct WorkflowArchivePruneResult {
    pub(crate) records_removed: u64,
    pub(crate) reclaimed_bytes: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct StartupGcRequest {
    pub(crate) app_data_dir: PathBuf,
    /// None means the live worktree set could not be resolved reliably. In that
    /// mode, workspace-dependent whole-log deletion is skipped for safety.
    pub(crate) live_worktrees: Option<LiveWorktreeResolution>,
    #[cfg(test)]
    pub(crate) session_records: Vec<SessionGcRecord>,
    #[cfg(test)]
    pub(crate) workflow_executions: Vec<WorkflowExecutionGcRecord>,
    pub(crate) workspace_state_records: Vec<WorkspaceStateGcRecord>,
    pub(crate) review_comment_records: Vec<ReviewCommentGcRecord>,
    pub(crate) checkpoint_paths: Vec<PathBuf>,
    pub(crate) cache_records: Vec<CacheGcRecord>,
    pub(crate) legacy_comment_paths: Vec<PathBuf>,
    #[cfg(test)]
    pub(crate) session_blob_stores: Vec<SessionBlobStore>,
    pub(crate) process_records: Vec<ProcessRecord>,
    pub(crate) runtime_protection: RuntimeProtection,
    pub(crate) now_secs: f64,
    pub(crate) retention: RetentionPolicy,
}

pub(super) fn worktree_path_key(path: &str) -> String {
    trim_path_separators(path.trim())
}

fn prefix_matches_at_boundary(value: &str, prefix: &str, boundary_chars: &[char]) -> bool {
    if value == prefix {
        return true;
    }
    if prefix.is_empty() {
        return false;
    }
    value
        .strip_prefix(prefix)
        .and_then(|suffix| suffix.chars().next())
        .is_some_and(|ch| boundary_chars.contains(&ch))
}

fn trim_path_separators(path: &str) -> String {
    let mut value = path.to_string();
    while value.len() > 1 && (value.ends_with('/') || value.ends_with('\\')) {
        value.pop();
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn unresolved_worktree_path_matching_requires_path_boundary() {
        let resolution = LiveWorktreeResolution::new(
            LiveWorktreeSet::default(),
            vec!["/repo".to_string()],
            HashSet::new(),
        );

        assert!(resolution.worktree_path_may_be_unresolved(&GcWorktreePath::not_found("/repo")));
        assert!(resolution
            .worktree_path_may_be_unresolved(&GcWorktreePath::not_found("/repo/worktree")));
        assert!(resolution
            .worktree_path_may_be_unresolved(&GcWorktreePath::not_found("/repo\\worktree")));
        assert!(!resolution.worktree_path_may_be_unresolved(&GcWorktreePath::not_found("/repo2")));
    }

    #[test]
    fn unresolved_workspace_state_key_matching_requires_key_boundary() {
        let resolution = LiveWorktreeResolution::new(
            LiveWorktreeSet::default(),
            Vec::new(),
            HashSet::from(["_repo".to_string()]),
        );

        assert!(resolution.workspace_state_key_may_be_unresolved("_repo"));
        assert!(resolution.workspace_state_key_may_be_unresolved("_repo_worktree"));
        assert!(!resolution.workspace_state_key_may_be_unresolved("_repo2"));
    }
}
