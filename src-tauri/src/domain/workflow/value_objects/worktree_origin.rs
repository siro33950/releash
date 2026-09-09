use crate::domain::repository::{normalize_repo_path, worktree_dir};

const ISOLATED_BRANCH_PREFIX: &str = "releash/isolated/";
const ISOLATED_DIRECTORY: &str = ".releash-isolated";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IsolatedWorktree {
    pub branch: String,
    pub path: String,
}

impl IsolatedWorktree {
    pub fn with_artifact(&self, artifact: Option<serde_json::Value>) -> serde_json::Value {
        let mut artifact = artifact.unwrap_or_else(|| serde_json::json!({}));
        artifact
            .as_object_mut()
            .expect("Artifact is an object")
            .insert(
                "worktree".to_string(),
                serde_json::json!({"branch": self.branch, "path": self.path}),
            );
        artifact
    }

    pub fn for_attempt(repository_root: &str, node_execution_id: &str, attempt: u32) -> Self {
        Self {
            branch: isolated_worktree_branch(node_execution_id, attempt),
            path: isolated_worktree_path(repository_root, node_execution_id, attempt),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct WorktreeInheritance {
    mode: super::WorktreeMode,
}

impl WorktreeInheritance {
    pub fn new(mode: Option<super::WorktreeMode>) -> Self {
        Self {
            mode: mode.unwrap_or_default(),
        }
    }

    pub fn is_isolated(self) -> bool {
        self.mode.is_isolated()
    }

    pub fn for_attempt(
        self,
        repository_root: Option<&str>,
        node_execution_id: &str,
        attempt: u32,
    ) -> Result<Option<IsolatedWorktree>, &'static str> {
        if !self.is_isolated() {
            return Ok(None);
        }
        let root = repository_root.ok_or("isolated execution requires repository root")?;
        Ok(Some(IsolatedWorktree::for_attempt(
            root,
            node_execution_id,
            attempt,
        )))
    }

    pub fn isolated_path(self, worktree: Option<&IsolatedWorktree>) -> Option<&str> {
        self.is_isolated()
            .then_some(worktree)
            .flatten()
            .map(|worktree| worktree.path.as_str())
    }

    pub fn effective_path<'a>(
        root_worktree_path: &'a str,
        ancestors: impl IntoIterator<Item = (Self, Option<&'a IsolatedWorktree>)>,
    ) -> &'a str {
        ancestors
            .into_iter()
            .find_map(|(rule, worktree)| rule.isolated_path(worktree))
            .unwrap_or(root_worktree_path)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeInventoryEntry {
    pub repository_root: String,
    pub worktree_path: String,
    pub branch: String,
}

impl WorktreeInventoryEntry {
    pub fn new(
        repository_root: impl AsRef<str>,
        worktree_path: impl AsRef<str>,
        branch: impl Into<String>,
    ) -> Self {
        Self {
            repository_root: normalize_repo_path(repository_root.as_ref()),
            worktree_path: normalize_repo_path(worktree_path.as_ref()),
            branch: branch.into(),
        }
    }

    pub fn matches_isolated_identity_rule(&self) -> bool {
        let Some(token) = self.branch.strip_prefix(ISOLATED_BRANCH_PREFIX) else {
            return false;
        };
        let Some((node_execution_id, attempt)) = parse_identity_token(token) else {
            return false;
        };
        self.branch == isolated_worktree_branch(node_execution_id, attempt)
            && self.worktree_path
                == isolated_worktree_path(&self.repository_root, node_execution_id, attempt)
    }
}

pub fn isolated_worktree_identity_token(node_execution_id: &str, attempt: u32) -> String {
    format!("{node_execution_id}-a{attempt}")
}

pub fn isolated_worktree_path(
    repository_root: &str,
    node_execution_id: &str,
    attempt: u32,
) -> String {
    isolated_worktree_path_for_token(
        repository_root,
        &isolated_worktree_identity_token(node_execution_id, attempt),
    )
}

pub fn isolated_worktree_branch(node_execution_id: &str, attempt: u32) -> String {
    format!(
        "{ISOLATED_BRANCH_PREFIX}{}",
        isolated_worktree_identity_token(node_execution_id, attempt)
    )
}

fn isolated_worktree_path_for_token(repository_root: &str, token: &str) -> String {
    normalize_repo_path(&format!(
        "{}/{ISOLATED_DIRECTORY}/{token}",
        worktree_dir(repository_root)
    ))
}

pub fn isolated_worktree_owner(path: &str) -> Option<(String, u32)> {
    let path = normalize_repo_path(path);
    let (directory, token) = path.rsplit_once('/')?;
    let (_, isolated_directory) = directory.rsplit_once('/')?;
    if isolated_directory != ISOLATED_DIRECTORY {
        return None;
    }
    let (node_execution_id, attempt) = parse_identity_token(token)?;
    (token == isolated_worktree_identity_token(node_execution_id, attempt))
        .then(|| (node_execution_id.to_string(), attempt))
}

fn parse_identity_token(token: &str) -> Option<(&str, u32)> {
    let (node_execution_id, attempt) = token.rsplit_once("-a")?;
    if node_execution_id.is_empty() {
        return None;
    }
    let attempt = attempt.parse().ok()?;
    Some((node_execution_id, attempt))
}

#[cfg(test)]
#[path = "worktree_origin_test.rs"]
mod worktree_origin_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_identity_uses_sibling_isolated_directory_and_branch() {
        assert_eq!(
            isolated_worktree_path("/projects/repo", "node-1", 2),
            "/projects/repo-worktrees/.releash-isolated/node-1-a2"
        );
        assert_eq!(
            isolated_worktree_branch("node-1", 2),
            "releash/isolated/node-1-a2"
        );
    }

    #[test]
    fn fallback_requires_matching_canonical_path_and_branch_token() {
        let canonical = WorktreeInventoryEntry::new(
            "/projects/repo",
            "/projects/repo-worktrees/.releash-isolated/node-1-a2",
            "releash/isolated/node-1-a2",
        );
        assert!(canonical.matches_isolated_identity_rule());

        let wrong_branch = WorktreeInventoryEntry::new(
            "/projects/repo",
            &canonical.worktree_path,
            "feature/node-1-a2",
        );
        assert!(!wrong_branch.matches_isolated_identity_rule());

        let wrong_path = WorktreeInventoryEntry::new(
            "/projects/repo",
            "/projects/repo-worktrees/node-1-a2",
            &canonical.branch,
        );
        assert!(!wrong_path.matches_isolated_identity_rule());
    }
}
