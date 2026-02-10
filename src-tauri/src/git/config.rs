use super::error::GitError;
use git2::Repository;

#[tauri::command]
pub fn get_releash_base(repo_path: String) -> Result<Option<String>, GitError> {
    let repo = Repository::open(&repo_path)?;
    let base = repo
        .config()
        .ok()
        .and_then(|cfg| cfg.get_string("releash.base").ok());
    Ok(base)
}

#[tauri::command]
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
