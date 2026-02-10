use super::error::GitError;
use git2::Repository;
use std::path::Path;

#[tauri::command]
pub fn get_cwd() -> Result<String, GitError> {
    std::env::current_dir()?
        .to_str()
        .ok_or_else(|| GitError::Custom("invalid path encoding".to_string()))
        .map(|s| s.to_string())
}

#[tauri::command]
pub fn get_repo_git_dir(file_path: String) -> Result<String, GitError> {
    let path = Path::new(&file_path);
    let repo = Repository::discover(path)?;

    repo.path()
        .to_str()
        .ok_or_else(|| GitError::Custom("invalid path encoding".to_string()))
        .map(|s| s.to_string())
}
