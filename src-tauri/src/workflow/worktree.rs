use std::path::PathBuf;
use std::sync::Arc;

use crate::config::{AppConfig, ReleashConfig};

fn configured_repo_paths(config: &ReleashConfig) -> Vec<String> {
    let mut paths = config.app.last_repo_paths.clone();
    if !config.app.last_root_path.is_empty() && !paths.contains(&config.app.last_root_path) {
        paths.push(config.app.last_root_path.clone());
    }
    paths
}

pub(crate) fn canonicalize_managed_worktree_path_inner(
    repo_paths: Vec<String>,
    worktree_path: String,
) -> Result<String, String> {
    let requested = PathBuf::from(&worktree_path)
        .canonicalize()
        .map_err(|e| format!("invalid worktree_path: {e}"))?;
    for repo_path in repo_paths {
        let Ok(repo_path) = PathBuf::from(&repo_path).canonicalize() else {
            continue;
        };
        let repo_path_str = repo_path
            .to_str()
            .ok_or_else(|| "configured repo path has invalid encoding".to_string())?
            .to_string();
        let Ok(worktrees) = crate::git::list_worktrees(repo_path_str) else {
            continue;
        };
        for worktree in worktrees {
            let Ok(candidate) = PathBuf::from(&worktree.path).canonicalize() else {
                continue;
            };
            if candidate == requested {
                return requested
                    .to_str()
                    .map(|p| p.trim_end_matches('/').to_string())
                    .ok_or_else(|| "worktree_path has invalid encoding".to_string());
            }
        }
    }
    Err("worktree_path is not a configured git worktree".to_string())
}

pub(crate) async fn canonicalize_managed_worktree_path(
    config: Arc<AppConfig>,
    worktree_path: String,
) -> Result<String, String> {
    let repo_paths = configured_repo_paths(&config.get_config()?);
    tokio::task::spawn_blocking(move || {
        canonicalize_managed_worktree_path_inner(repo_paths, worktree_path)
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalize_managed_worktree_path_accepts_configured_git_worktree_only() {
        let (repo_dir, repo) = crate::git::test_helpers::create_test_repo();
        crate::git::test_helpers::create_initial_commit(&repo);
        let worktree_parent = tempfile::TempDir::new().unwrap();
        let worktree_path = worktree_parent.path().join("managed-wt");
        repo.worktree("managed-wt", &worktree_path, None).unwrap();

        let canonical = worktree_path.canonicalize().unwrap();
        let accepted = canonicalize_managed_worktree_path_inner(
            vec![repo_dir.path().to_string_lossy().to_string()],
            worktree_path.join(".").to_string_lossy().to_string(),
        )
        .unwrap();
        assert_eq!(std::path::PathBuf::from(accepted), canonical);

        let outside = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(outside.path()).unwrap();
        let err = canonicalize_managed_worktree_path_inner(
            vec![repo_dir.path().to_string_lossy().to_string()],
            outside.path().to_string_lossy().to_string(),
        )
        .unwrap_err();
        assert!(err.contains("not a configured git worktree"));
    }
}
