use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use crate::adaptor::gateway::repository::worktree::WorktreeGateway;
use crate::domain::app_config::value_objects::AppSettings;
use crate::domain::app_config::ConfigRepository;
use crate::domain::repository::WorktreeRepository;
use crate::domain::workflow::{
    ManagedWorktreeGateway, RepositoryWorktreeInventory, WorkflowError, WorktreeInventoryEntry,
    WorktreeInventoryGateway,
};
use crate::usecase::repository_usecase::RepositoryUsecase;

fn configured_repo_paths(app: &AppSettings) -> Vec<String> {
    let mut paths = app.last_repo_paths.clone();
    if !app.last_root_path.is_empty() && !paths.contains(&app.last_root_path) {
        paths.push(app.last_root_path.clone());
    }
    paths
}

fn inventory_snapshot_inner(
    repository: &dyn WorktreeRepository,
    repo_paths: Vec<String>,
) -> Vec<RepositoryWorktreeInventory> {
    let mut seen = BTreeSet::new();
    let mut snapshots = Vec::new();
    for configured_path in repo_paths {
        let Ok(repository_root) = repository.main_repo_path(&configured_path) else {
            continue;
        };
        if !seen.insert(repository_root.clone()) {
            continue;
        }
        let Ok(worktrees) = repository.list(&repository_root) else {
            continue;
        };
        let worktrees = worktrees
            .into_iter()
            .map(|worktree| {
                WorktreeInventoryEntry::new(&repository_root, worktree.path, worktree.branch)
            })
            .collect();
        snapshots.push(RepositoryWorktreeInventory::new(repository_root, worktrees));
    }
    snapshots
}

/// [05] API / CLI 共有 helper: `worktree_path` filter input を OS レベルで canonicalize し、
/// 末尾 `/` を除去した正規化済み文字列を返す。
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
    config: Arc<dyn ConfigRepository>,
    worktree_path: String,
) -> Result<String, String> {
    let repo_paths = configured_repo_paths(&config.load().map_err(|e| e.to_string())?.app);
    tokio::task::spawn_blocking(move || {
        canonicalize_managed_worktree_path_inner(&usecase, repo_paths, worktree_path)
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[derive(Clone)]
pub(crate) struct RepositoryManagedWorktreeGateway {
    repository: Arc<RepositoryUsecase>,
    config: Arc<dyn ConfigRepository>,
}

impl RepositoryManagedWorktreeGateway {
    pub(crate) fn new(
        repository: Arc<RepositoryUsecase>,
        config: Arc<dyn ConfigRepository>,
    ) -> Self {
        Self { repository, config }
    }
}

impl ManagedWorktreeGateway for RepositoryManagedWorktreeGateway {
    fn resolve(&self, worktree_path: &str) -> Result<String, WorkflowError> {
        let config = self
            .config
            .load()
            .map_err(|e| WorkflowError::external(e.to_string()))?;
        let repo_paths = configured_repo_paths(&config.app);
        canonicalize_managed_worktree_path_inner(
            &self.repository,
            repo_paths,
            worktree_path.to_string(),
        )
        .map_err(WorkflowError::external)
    }
}

#[derive(Clone)]
pub(crate) struct RepositoryWorktreeInventoryGateway {
    repository: Arc<dyn WorktreeRepository>,
    config: Arc<dyn ConfigRepository>,
}

impl RepositoryWorktreeInventoryGateway {
    pub(crate) fn new(config: Arc<dyn ConfigRepository>) -> Self {
        Self {
            repository: Arc::new(WorktreeGateway),
            config,
        }
    }
}

impl WorktreeInventoryGateway for RepositoryWorktreeInventoryGateway {
    fn snapshot(&self) -> Result<Vec<RepositoryWorktreeInventory>, WorkflowError> {
        let config = self
            .config
            .load()
            .map_err(|error| WorkflowError::external(error.to_string()))?;
        Ok(inventory_snapshot_inner(
            self.repository.as_ref(),
            configured_repo_paths(&config.app),
        ))
    }
}

#[derive(Clone)]
pub(crate) struct RepoPathsManagedWorktreeGateway {
    repository: Arc<RepositoryUsecase>,
    repo_paths: Vec<String>,
}

impl RepoPathsManagedWorktreeGateway {
    pub(crate) fn new(repository: Arc<RepositoryUsecase>, repo_paths: Vec<String>) -> Self {
        Self {
            repository,
            repo_paths,
        }
    }
}

impl ManagedWorktreeGateway for RepoPathsManagedWorktreeGateway {
    fn resolve(&self, worktree_path: &str) -> Result<String, WorkflowError> {
        canonicalize_managed_worktree_path_inner(
            &self.repository,
            self.repo_paths.clone(),
            worktree_path.to_string(),
        )
        .map_err(WorkflowError::external)
    }
}

#[cfg(test)]
#[derive(Clone)]
pub(crate) struct PassthroughManagedWorktreeGateway;

#[cfg(test)]
impl ManagedWorktreeGateway for PassthroughManagedWorktreeGateway {
    fn resolve(&self, worktree_path: &str) -> Result<String, WorkflowError> {
        normalize_worktree_filter_path(worktree_path).map_err(WorkflowError::external)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::value_objects::{
        isolated_worktree_branch, isolated_worktree_path,
    };
    use git2::{BranchType, Repository};
    use std::fs;

    fn test_usecase() -> RepositoryUsecase {
        crate::adaptor::controller::wiring::build_repository_usecase()
    }

    #[test]
    fn canonicalize_managed_worktree_path_accepts_configured_git_worktree_only() {
        let (repo_dir, repo) = crate::test_support::git::create_test_repo();
        crate::test_support::git::create_initial_commit(&repo);
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

    #[test]
    fn inventory_snapshot_is_read_only_and_omits_repositories_that_cannot_be_read() {
        let parent = tempfile::TempDir::new().unwrap();
        let repo_path = parent.path().join("main-repo");
        fs::create_dir(&repo_path).unwrap();
        let repo = Repository::init(&repo_path).unwrap();
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "Test User").unwrap();
        config.set_str("user.email", "test@example.com").unwrap();
        crate::test_support::git::create_initial_commit(&repo);

        let repository_root = WorktreeGateway
            .main_repo_path(&repo_path.to_string_lossy())
            .unwrap();
        let node_execution_id = "node-1";
        let branch = isolated_worktree_branch(node_execution_id, 1);
        let worktree_path = isolated_worktree_path(&repository_root, node_execution_id, 1);
        WorktreeGateway
            .create(&repository_root, &worktree_path, &branch, true, None)
            .unwrap();

        let snapshots = inventory_snapshot_inner(
            &WorktreeGateway,
            vec![
                repository_root.clone(),
                parent
                    .path()
                    .join("missing-repository")
                    .to_string_lossy()
                    .to_string(),
            ],
        );

        assert_eq!(snapshots.len(), 1);
        assert!(snapshots[0]
            .worktrees
            .iter()
            .any(|worktree| worktree.worktree_path == worktree_path && worktree.branch == branch));
        assert!(std::path::Path::new(&worktree_path).exists());
        assert!(repo.find_branch(&branch, BranchType::Local).is_ok());
        assert!(repo.find_worktree("node-1-a1").is_ok());
    }
}
