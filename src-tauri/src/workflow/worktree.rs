use std::path::PathBuf;
use std::sync::Arc;

use crate::config::{AppConfig, ReleashConfig};
use crate::usecase::repository_usecase::RepositoryUsecase;

fn configured_repo_paths(config: &ReleashConfig) -> Vec<String> {
    let mut paths = config.app.last_repo_paths.clone();
    if !config.app.last_root_path.is_empty() && !paths.contains(&config.app.last_root_path) {
        paths.push(config.app.last_root_path.clone());
    }
    paths
}

/// [05] API / CLI 共有 helper: `worktree_path` filter input を OS レベルで canonicalize し、
/// 末尾 `/` を除去した正規化済み文字列を返す。AppConfig 非依存で、相対パス / symlink
/// 経由でも同じ物理パスに対して同一の filter 値を生成することで、API / CLI の
/// 観測結果が分岐しないようにする（spec [05] API / CLI の意味的等価性境界）。
pub(crate) fn normalize_worktree_filter_path(worktree_path: &str) -> Result<String, String> {
    let canonical = PathBuf::from(worktree_path)
        .canonicalize()
        .map_err(|e| format!("invalid worktree_path: {e}"))?;
    canonical
        .to_str()
        .map(|p| p.trim_end_matches('/').to_string())
        .ok_or_else(|| "worktree_path has invalid encoding".to_string())
}

pub(crate) fn canonicalize_managed_worktree_path_inner(
    usecase: &RepositoryUsecase,
    repo_paths: Vec<String>,
    worktree_path: String,
) -> Result<String, String> {
    let requested_normalized = normalize_worktree_filter_path(&worktree_path)?;
    let requested = PathBuf::from(&requested_normalized);
    // usecase は composition root（lib.rs）で組み立てたものを注入で受け取り、
    // 各 repo_path の反復で再利用する（controller 配線を workflow へ漏らさない）。
    for repo_path in repo_paths {
        let Ok(repo_path) = PathBuf::from(&repo_path).canonicalize() else {
            continue;
        };
        let repo_path_str = repo_path
            .to_str()
            .ok_or_else(|| "configured repo path has invalid encoding".to_string())?
            .to_string();
        let Ok(worktrees) = usecase.list_worktrees(&repo_path_str) else {
            continue;
        };
        for worktree in worktrees {
            let Ok(candidate) = PathBuf::from(&worktree.path).canonicalize() else {
                continue;
            };
            if candidate == requested {
                return Ok(requested_normalized);
            }
        }
    }
    Err("worktree_path is not a configured git worktree".to_string())
}

pub(crate) async fn canonicalize_managed_worktree_path(
    usecase: Arc<RepositoryUsecase>,
    config: Arc<AppConfig>,
    worktree_path: String,
) -> Result<String, String> {
    let repo_paths = configured_repo_paths(&config.get_config()?);
    tokio::task::spawn_blocking(move || {
        canonicalize_managed_worktree_path_inner(&usecase, repo_paths, worktree_path)
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_usecase() -> RepositoryUsecase {
        crate::adaptor::controller::wiring::build_repository_usecase()
    }

    #[test]
    fn canonicalize_managed_worktree_path_accepts_configured_git_worktree_only() {
        let (repo_dir, repo) = crate::git::test_helpers::create_test_repo();
        crate::git::test_helpers::create_initial_commit(&repo);
        let worktree_parent = tempfile::TempDir::new().unwrap();
        let worktree_path = worktree_parent.path().join("managed-wt");
        repo.worktree("managed-wt", &worktree_path, None).unwrap();

        let usecase = test_usecase();
        let canonical = worktree_path.canonicalize().unwrap();
        let accepted = canonicalize_managed_worktree_path_inner(
            &usecase,
            vec![repo_dir.path().to_string_lossy().to_string()],
            worktree_path.join(".").to_string_lossy().to_string(),
        )
        .unwrap();
        assert_eq!(std::path::PathBuf::from(accepted), canonical);

        let outside = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(outside.path()).unwrap();
        let err = canonicalize_managed_worktree_path_inner(
            &usecase,
            vec![repo_dir.path().to_string_lossy().to_string()],
            outside.path().to_string_lossy().to_string(),
        )
        .unwrap_err();
        assert!(err.contains("not a configured git worktree"));
    }
}
