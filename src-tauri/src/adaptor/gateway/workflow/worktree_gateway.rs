use std::path::PathBuf;
use std::sync::Arc;

use crate::adaptor::gateway::repository::worktree::WorktreeGateway;
use crate::domain::app_config::value_objects::AppSettings;
use crate::domain::app_config::ConfigRepository;
use crate::domain::repository::WorktreeRepository;
use crate::domain::workflow::{ManagedWorktreeGateway, WorkflowError};
use crate::usecase::repository_usecase::RepositoryUsecase;

fn configured_repo_paths(app: &AppSettings) -> Vec<String> {
    let mut paths = app.last_repo_paths.clone();
    if !app.last_root_path.is_empty() && !paths.contains(&app.last_root_path) {
        paths.push(app.last_root_path.clone());
    }
    paths
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
pub(crate) struct RepositoryIsolatedWorktreeGateway;

impl crate::domain::workflow::IsolatedWorktreeGateway for RepositoryIsolatedWorktreeGateway {
    fn repository_root(&self, worktree_path: &str) -> Result<String, WorkflowError> {
        WorktreeGateway
            .main_repo_path(worktree_path)
            .map_err(|error| WorkflowError::external(error.to_string()))
    }

    fn is_created(
        &self,
        parent_worktree_path: &str,
        worktree: &crate::domain::workflow::IsolatedWorktree,
    ) -> Result<bool, WorkflowError> {
        let inspect = || -> Result<bool, Box<dyn std::error::Error>> {
            let repo = git2::Repository::open(parent_worktree_path)?;
            let path = std::path::Path::new(&worktree.path);
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or("invalid isolated worktree path")?;
            let existing = match repo.find_worktree(name) {
                Ok(existing) => existing,
                Err(error) if error.code() == git2::ErrorCode::NotFound => return Ok(false),
                Err(error) => return Err(error.into()),
            };
            existing.validate()?;
            if existing.path().canonicalize()? != path.canonicalize()? {
                return Err("isolated worktree path does not match its registration".into());
            }
            let isolated = git2::Repository::open(path)?;
            if isolated.commondir().canonicalize()? != repo.commondir().canonicalize()?
                || isolated.head()?.name()? != format!("refs/heads/{}", worktree.branch)
            {
                return Err("isolated worktree repository or branch does not match".into());
            }
            Ok(true)
        };
        inspect().map_err(|error| WorkflowError::external(error.to_string()))
    }

    fn create(
        &self,
        parent_worktree_path: &str,
        worktree: &crate::domain::workflow::IsolatedWorktree,
    ) -> Result<(), WorkflowError> {
        let create = || -> Result<(), Box<dyn std::error::Error>> {
            let repo = git2::Repository::open(parent_worktree_path)?;
            let commit = repo.head()?.peel_to_commit()?;
            let reference = repo
                .branch(&worktree.branch, &commit, false)?
                .into_reference();
            let path = std::path::Path::new(&worktree.path);
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or("invalid isolated worktree path")?;
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut options = git2::WorktreeAddOptions::new();
            options.reference(Some(&reference));
            repo.worktree(name, path, Some(&options))?;
            Ok(())
        };
        create().map_err(|error| WorkflowError::external(error.to_string()))
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
}

#[cfg(test)]
#[path = "worktree_gateway_test.rs"]
mod worktree_gateway_tests;
