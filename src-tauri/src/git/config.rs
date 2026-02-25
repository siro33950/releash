use super::error::GitError;
use git2::Repository;

/// フォールバックチェーン付きでベースブランチ名を解決する（内部用）
/// branch.<name>.releash-base → releash.base → detect_default_branch()
pub(crate) fn resolve_branch_base(
    repo: &Repository,
    config: Option<&git2::Config>,
    branch_name: &str,
) -> Option<String> {
    if let Some(cfg) = config {
        if let Ok(base) = cfg.get_string(&format!("branch.{branch_name}.releash-base")) {
            return Some(base);
        }
    }
    if let Some(cfg) = config {
        if let Ok(base) = cfg.get_string("releash.base") {
            return Some(base);
        }
    }
    super::branch::detect_default_branch(repo)
}

pub fn get_branch_base(repo_path: String, branch_name: String) -> Result<Option<String>, GitError> {
    let repo = Repository::open(&repo_path)?;
    let config = repo.config().ok();
    Ok(resolve_branch_base(&repo, config.as_ref(), &branch_name))
}

pub fn set_branch_base(
    repo_path: String,
    branch_name: String,
    base: Option<String>,
) -> Result<(), GitError> {
    let repo = Repository::open(&repo_path)?;
    let mut config = repo.config()?;
    let key = format!("branch.{branch_name}.releash-base");
    match base {
        Some(b) => config.set_str(&key, &b)?,
        None => match config.remove(&key) {
            Ok(()) => {}
            Err(e) if e.code() == git2::ErrorCode::NotFound => {}
            Err(e) => return Err(e.into()),
        },
    }
    Ok(())
}

pub fn get_releash_base(repo_path: String) -> Result<Option<String>, GitError> {
    let repo = Repository::open(&repo_path)?;
    let base = repo
        .config()
        .ok()
        .and_then(|cfg| cfg.get_string("releash.base").ok());
    Ok(base)
}

pub fn set_releash_base(repo_path: String, base: Option<String>) -> Result<(), GitError> {
    let repo = Repository::open(&repo_path)?;
    let mut config = repo.config()?;
    match base {
        Some(b) => config.set_str("releash.base", &b)?,
        None => match config.remove("releash.base") {
            Ok(()) => {}
            Err(e) if e.code() == git2::ErrorCode::NotFound => {}
            Err(e) => return Err(e.into()),
        },
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::test_helpers::*;

    #[test]
    fn test_resolve_branch_base_per_branch() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        let repo_path = dir.path().to_str().unwrap().to_string();

        set_branch_base(
            repo_path.clone(),
            "feat".to_string(),
            Some("develop".to_string()),
        )
        .unwrap();
        let config = repo.config().ok();
        let result = resolve_branch_base(&repo, config.as_ref(), "feat");
        assert_eq!(result, Some("develop".to_string()));
    }

    #[test]
    fn test_resolve_branch_base_fallback_releash_base() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        let repo_path = dir.path().to_str().unwrap().to_string();

        set_releash_base(repo_path.clone(), Some("develop".to_string())).unwrap();
        let config = repo.config().ok();
        let result = resolve_branch_base(&repo, config.as_ref(), "feat");
        assert_eq!(result, Some("develop".to_string()));
    }

    #[test]
    fn test_resolve_branch_base_fallback_detect_default() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        let config = repo.config().ok();
        let result = resolve_branch_base(&repo, config.as_ref(), "feat");
        // detect_default_branch returns the main branch name
        assert!(result.is_some());
    }

    #[test]
    fn test_get_set_branch_base() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        let repo_path = dir.path().to_str().unwrap().to_string();

        let base = get_branch_base(repo_path.clone(), "feat".to_string()).unwrap();
        // No per-branch config set, falls back
        assert!(base.is_some());

        set_branch_base(
            repo_path.clone(),
            "feat".to_string(),
            Some("develop".to_string()),
        )
        .unwrap();
        let base = get_branch_base(repo_path.clone(), "feat".to_string()).unwrap();
        assert_eq!(base, Some("develop".to_string()));

        set_branch_base(repo_path.clone(), "feat".to_string(), None).unwrap();
        let base = get_branch_base(repo_path.clone(), "feat".to_string()).unwrap();
        // Falls back to detect_default_branch again
        assert!(base.is_some());
        assert_ne!(base, Some("develop".to_string()));
    }

    #[test]
    fn test_get_set_releash_base() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        let repo_path = dir.path().to_str().unwrap().to_string();

        let base = get_releash_base(repo_path.clone()).unwrap();
        assert_eq!(base, None);

        set_releash_base(repo_path.clone(), Some("develop".to_string())).unwrap();
        let base = get_releash_base(repo_path.clone()).unwrap();
        assert_eq!(base, Some("develop".to_string()));

        set_releash_base(repo_path.clone(), None).unwrap();
        let base = get_releash_base(repo_path).unwrap();
        assert_eq!(base, None);
    }
}
