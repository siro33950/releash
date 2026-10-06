use crate::usecase::repository_state::RepositoryStateError;

use crate::usecase::repository_state::runtime::WorktreePathNormalizer;

use std::path::PathBuf;

pub(crate) struct CanonicalWorktreePathNormalizer;

impl WorktreePathNormalizer for CanonicalWorktreePathNormalizer {
    fn normalize(&self, worktree_path: &str) -> Result<PathBuf, RepositoryStateError> {
        let path = PathBuf::from(worktree_path);
        path.canonicalize().map_err(|err| {
            RepositoryStateError::Watcher(format!(
                "Failed to canonicalize worktree path {}: {err}",
                path.display()
            ))
        })
    }
}
