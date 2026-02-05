use git2::{BranchType, Repository};
use std::path::Path;

#[tauri::command]
pub fn get_file_at_ref(file_path: String, git_ref: String) -> Result<String, String> {
    let path = Path::new(&file_path);
    let repo = Repository::discover(path).map_err(|e| e.message().to_string())?;

    let repo_workdir = repo
        .workdir()
        .ok_or_else(|| "bare repository".to_string())?;

    let relative_path = path.strip_prefix(repo_workdir).map_err(|e| e.to_string())?;

    let obj = repo
        .revparse_single(&git_ref)
        .map_err(|e| e.message().to_string())?;

    let commit = obj.peel_to_commit().map_err(|e| e.message().to_string())?;

    let tree = commit.tree().map_err(|e| e.message().to_string())?;

    let entry = tree
        .get_path(relative_path)
        .map_err(|e| e.message().to_string())?;

    let blob = repo
        .find_blob(entry.id())
        .map_err(|e| e.message().to_string())?;

    let content = std::str::from_utf8(blob.content())
        .map_err(|e| e.to_string())?
        .to_string();

    Ok(content)
}

#[tauri::command]
pub fn get_staged_content(file_path: String) -> Result<String, String> {
    let path = Path::new(&file_path);
    let repo = Repository::discover(path).map_err(|e| e.message().to_string())?;

    let repo_workdir = repo
        .workdir()
        .ok_or_else(|| "bare repository".to_string())?;

    let relative_path = path.strip_prefix(repo_workdir).map_err(|e| e.to_string())?;

    let index = repo.index().map_err(|e| e.message().to_string())?;

    let relative_str = relative_path
        .to_str()
        .ok_or_else(|| "invalid path encoding".to_string())?;

    let entry = index
        .get_path(Path::new(relative_str), 0)
        .ok_or_else(|| "file not staged".to_string())?;

    let blob = repo
        .find_blob(entry.id)
        .map_err(|e| e.message().to_string())?;

    let content = std::str::from_utf8(blob.content())
        .map_err(|e| e.to_string())?
        .to_string();

    Ok(content)
}

#[tauri::command]
pub fn list_branches(file_path: String) -> Result<Vec<String>, String> {
    let path = Path::new(&file_path);
    let repo = Repository::discover(path).map_err(|e| e.message().to_string())?;

    let branches = repo
        .branches(Some(BranchType::Local))
        .map_err(|e| e.message().to_string())?;

    let mut names = Vec::new();
    for branch in branches {
        let (branch, _) = branch.map_err(|e| e.message().to_string())?;
        if let Some(name) = branch.name().map_err(|e| e.message().to_string())? {
            names.push(name.to_string());
        }
    }

    Ok(names)
}

#[tauri::command]
pub fn get_repo_git_dir(file_path: String) -> Result<String, String> {
    let path = Path::new(&file_path);
    let repo = Repository::discover(path).map_err(|e| e.message().to_string())?;

    repo.path()
        .to_str()
        .ok_or_else(|| "invalid path encoding".to_string())
        .map(|s| s.to_string())
}
