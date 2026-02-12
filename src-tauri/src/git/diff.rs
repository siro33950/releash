use super::error::GitError;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use git2::Repository;
use std::path::Path;

pub fn get_file_at_ref(file_path: String, git_ref: String) -> Result<String, GitError> {
    let path = Path::new(&file_path);
    let repo = Repository::discover(path)?;

    let repo_workdir = repo
        .workdir()
        .ok_or_else(|| GitError::Custom("bare repository".to_string()))?;

    let relative_path = path.strip_prefix(repo_workdir)?;

    let obj = repo.revparse_single(&git_ref)?;
    let commit = obj.peel_to_commit()?;
    let tree = commit.tree()?;
    let entry = tree.get_path(relative_path)?;
    let blob = repo.find_blob(entry.id())?;

    let content = std::str::from_utf8(blob.content())?.to_string();
    Ok(content)
}

pub fn get_staged_content(file_path: String) -> Result<String, GitError> {
    let path = Path::new(&file_path);
    let repo = Repository::discover(path)?;

    let repo_workdir = repo
        .workdir()
        .ok_or_else(|| GitError::Custom("bare repository".to_string()))?;

    let relative_path = path.strip_prefix(repo_workdir)?;

    let index = repo.index()?;

    let relative_str = relative_path
        .to_str()
        .ok_or_else(|| GitError::Custom("invalid path encoding".to_string()))?;

    let entry = index
        .get_path(Path::new(relative_str), 0)
        .ok_or_else(|| GitError::Custom("file not staged".to_string()))?;

    let blob = repo.find_blob(entry.id)?;

    let content = std::str::from_utf8(blob.content())?.to_string();
    Ok(content)
}

pub fn get_binary_file_at_ref(file_path: String, git_ref: String) -> Result<String, GitError> {
    let path = Path::new(&file_path);
    let repo = Repository::discover(path)?;

    let repo_workdir = repo
        .workdir()
        .ok_or_else(|| GitError::Custom("bare repository".to_string()))?;

    let relative_path = path.strip_prefix(repo_workdir)?;

    let obj = repo.revparse_single(&git_ref)?;
    let commit = obj.peel_to_commit()?;
    let tree = commit.tree()?;
    let entry = tree.get_path(relative_path)?;
    let blob = repo.find_blob(entry.id())?;

    Ok(STANDARD.encode(blob.content()))
}

pub fn get_binary_staged_content(file_path: String) -> Result<String, GitError> {
    let path = Path::new(&file_path);
    let repo = Repository::discover(path)?;

    let repo_workdir = repo
        .workdir()
        .ok_or_else(|| GitError::Custom("bare repository".to_string()))?;

    let relative_path = path.strip_prefix(repo_workdir)?;

    let index = repo.index()?;

    let relative_str = relative_path
        .to_str()
        .ok_or_else(|| GitError::Custom("invalid path encoding".to_string()))?;

    let entry = index
        .get_path(Path::new(relative_str), 0)
        .ok_or_else(|| GitError::Custom("file not staged".to_string()))?;

    let blob = repo.find_blob(entry.id)?;

    Ok(STANDARD.encode(blob.content()))
}
