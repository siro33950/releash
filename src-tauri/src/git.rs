use git2::{build::CheckoutBuilder, BranchType, ErrorCode, Repository, Sort, StatusOptions};
use serde::Serialize;
use std::path::Path;
use std::process::Command;

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

#[derive(Serialize)]
pub struct GitFileStatus {
    pub path: String,
    pub index_status: String,
    pub worktree_status: String,
}

fn index_status_from_flags(status: git2::Status) -> &'static str {
    if status.contains(git2::Status::INDEX_NEW) {
        "new"
    } else if status.contains(git2::Status::INDEX_MODIFIED) {
        "modified"
    } else if status.contains(git2::Status::INDEX_DELETED) {
        "deleted"
    } else if status.contains(git2::Status::INDEX_RENAMED) {
        "renamed"
    } else {
        "none"
    }
}

fn worktree_status_from_flags(status: git2::Status) -> &'static str {
    if status.contains(git2::Status::WT_NEW) {
        "new"
    } else if status.contains(git2::Status::WT_MODIFIED) {
        "modified"
    } else if status.contains(git2::Status::WT_DELETED) {
        "deleted"
    } else {
        "none"
    }
}

#[tauri::command]
pub fn get_git_status(repo_path: String) -> Result<Vec<GitFileStatus>, String> {
    let repo = Repository::open(&repo_path).map_err(|e| e.message().to_string())?;

    let mut opts = StatusOptions::new();
    opts.include_untracked(true).recurse_untracked_dirs(true);

    let statuses = repo
        .statuses(Some(&mut opts))
        .map_err(|e| e.message().to_string())?;

    let result: Vec<GitFileStatus> = statuses
        .iter()
        .filter_map(|entry| {
            let path = entry.path()?.to_string();
            let status = entry.status();
            let idx = index_status_from_flags(status);
            let wt = worktree_status_from_flags(status);
            if idx == "none" && wt == "none" {
                return None;
            }
            Some(GitFileStatus {
                path,
                index_status: idx.to_string(),
                worktree_status: wt.to_string(),
            })
        })
        .collect();

    Ok(result)
}

#[derive(Serialize)]
pub struct CommitInfo {
    pub hash: String,
    pub short_hash: String,
    pub message: String,
    pub author_name: String,
    pub author_email: String,
    pub timestamp: i64,
}

#[tauri::command]
pub fn get_git_log(repo_path: String, limit: Option<usize>) -> Result<Vec<CommitInfo>, String> {
    let repo = Repository::open(&repo_path).map_err(|e| e.message().to_string())?;
    let limit = limit.unwrap_or(50);

    let head = match repo.head() {
        Ok(h) => h,
        Err(e) if e.code() == git2::ErrorCode::UnbornBranch => return Ok(Vec::new()),
        Err(e) => return Err(e.message().to_string()),
    };

    let mut revwalk = repo.revwalk().map_err(|e| e.message().to_string())?;
    revwalk
        .push(head.target().ok_or("HEAD has no target")?)
        .map_err(|e| e.message().to_string())?;
    revwalk
        .set_sorting(Sort::TIME)
        .map_err(|e| e.message().to_string())?;

    let mut commits = Vec::new();
    for oid in revwalk {
        if commits.len() >= limit {
            break;
        }
        let oid = oid.map_err(|e| e.message().to_string())?;
        let commit = repo.find_commit(oid).map_err(|e| e.message().to_string())?;
        let hash = oid.to_string();
        let short_hash = hash[..7.min(hash.len())].to_string();
        commits.push(CommitInfo {
            hash,
            short_hash,
            message: commit.message().unwrap_or("").to_string(),
            author_name: commit.author().name().unwrap_or("").to_string(),
            author_email: commit.author().email().unwrap_or("").to_string(),
            timestamp: commit.time().seconds(),
        });
    }

    Ok(commits)
}

#[tauri::command]
pub fn get_current_branch(repo_path: String) -> Result<String, String> {
    let repo = Repository::open(&repo_path).map_err(|e| e.message().to_string())?;

    let head = match repo.head() {
        Ok(h) => h,
        Err(e) if e.code() == git2::ErrorCode::UnbornBranch => {
            return Ok("(no commits)".to_string())
        }
        Err(e) => return Err(e.message().to_string()),
    };

    if head.is_branch() {
        Ok(head.shorthand().unwrap_or("HEAD").to_string())
    } else {
        let oid = head.target().ok_or("HEAD has no target")?;
        let short = &oid.to_string()[..7];
        Ok(format!("({short})"))
    }
}

#[tauri::command]
pub fn git_stage(repo_path: String, paths: Vec<String>) -> Result<(), String> {
    let repo = Repository::open(&repo_path).map_err(|e| e.message().to_string())?;
    let mut index = repo.index().map_err(|e| e.message().to_string())?;

    let targets: Vec<String> = if paths.is_empty() {
        let mut opts = StatusOptions::new();
        opts.include_untracked(true).recurse_untracked_dirs(true);
        let statuses = repo
            .statuses(Some(&mut opts))
            .map_err(|e| e.message().to_string())?;
        statuses
            .iter()
            .filter_map(|entry| {
                let s = entry.status();
                if s.contains(git2::Status::WT_NEW)
                    || s.contains(git2::Status::WT_MODIFIED)
                    || s.contains(git2::Status::WT_DELETED)
                {
                    entry.path().map(|p| p.to_string())
                } else {
                    None
                }
            })
            .collect()
    } else {
        paths
    };

    let workdir = repo
        .workdir()
        .ok_or_else(|| "bare repository".to_string())?;

    for p in &targets {
        let full_path = workdir.join(p);
        if full_path.exists() {
            index
                .add_path(Path::new(p))
                .map_err(|e| e.message().to_string())?;
        } else {
            index
                .remove_path(Path::new(p))
                .map_err(|e| e.message().to_string())?;
        }
    }

    index.write().map_err(|e| e.message().to_string())?;
    Ok(())
}

#[tauri::command]
pub fn git_unstage(repo_path: String, paths: Vec<String>) -> Result<(), String> {
    let repo = Repository::open(&repo_path).map_err(|e| e.message().to_string())?;

    let head_result = repo.head();
    let is_unborn = matches!(&head_result, Err(e) if e.code() == ErrorCode::UnbornBranch);

    if is_unborn {
        let mut index = repo.index().map_err(|e| e.message().to_string())?;
        if paths.is_empty() {
            index.clear().map_err(|e| e.message().to_string())?;
        } else {
            for p in &paths {
                index
                    .remove_path(Path::new(p))
                    .map_err(|e| e.message().to_string())?;
            }
        }
        index.write().map_err(|e| e.message().to_string())?;
    } else {
        let head_ref = head_result.map_err(|e| e.message().to_string())?;
        let head_obj = head_ref
            .peel(git2::ObjectType::Any)
            .map_err(|e| e.message().to_string())?;

        let targets: Vec<String> = if paths.is_empty() {
            let mut opts = StatusOptions::new();
            opts.include_untracked(true).recurse_untracked_dirs(true);
            let statuses = repo
                .statuses(Some(&mut opts))
                .map_err(|e| e.message().to_string())?;
            statuses
                .iter()
                .filter_map(|entry| {
                    let s = entry.status();
                    if s.contains(git2::Status::INDEX_NEW)
                        || s.contains(git2::Status::INDEX_MODIFIED)
                        || s.contains(git2::Status::INDEX_DELETED)
                        || s.contains(git2::Status::INDEX_RENAMED)
                    {
                        entry.path().map(|p| p.to_string())
                    } else {
                        None
                    }
                })
                .collect()
        } else {
            paths
        };

        let path_specs: Vec<&str> = targets.iter().map(|s| s.as_str()).collect();
        repo.reset_default(Some(&head_obj), &path_specs)
            .map_err(|e| e.message().to_string())?;
    }

    Ok(())
}

#[tauri::command]
pub fn git_commit(repo_path: String, message: String) -> Result<String, String> {
    let repo = Repository::open(&repo_path).map_err(|e| e.message().to_string())?;
    let sig = repo.signature().map_err(|e| e.message().to_string())?;

    let mut index = repo.index().map_err(|e| e.message().to_string())?;
    let tree_id = index.write_tree().map_err(|e| e.message().to_string())?;
    let tree = repo
        .find_tree(tree_id)
        .map_err(|e| e.message().to_string())?;

    let parents = match repo.head() {
        Ok(head_ref) => {
            let commit = head_ref
                .peel_to_commit()
                .map_err(|e| e.message().to_string())?;
            vec![commit]
        }
        Err(e) if e.code() == ErrorCode::UnbornBranch => vec![],
        Err(e) => return Err(e.message().to_string()),
    };

    let parent_refs: Vec<&git2::Commit> = parents.iter().collect();
    let oid = repo
        .commit(Some("HEAD"), &sig, &sig, &message, &tree, &parent_refs)
        .map_err(|e| e.message().to_string())?;

    Ok(oid.to_string())
}

#[tauri::command]
pub fn git_push(repo_path: String) -> Result<String, String> {
    let output = Command::new("git")
        .args(["push", "-u", "origin", "HEAD"])
        .current_dir(&repo_path)
        .output()
        .map_err(|e| format!("Failed to execute git push: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if output.status.success() {
        Ok(format!("{stdout}{stderr}").trim().to_string())
    } else {
        Err(stderr.trim().to_string())
    }
}

#[tauri::command]
pub fn git_create_branch(repo_path: String, branch_name: String) -> Result<(), String> {
    let repo = Repository::open(&repo_path).map_err(|e| e.message().to_string())?;

    let head = repo.head().map_err(|e| e.message().to_string())?;
    let commit = head.peel_to_commit().map_err(|e| e.message().to_string())?;

    repo.branch(&branch_name, &commit, false)
        .map_err(|e| e.message().to_string())?;

    repo.set_head(&format!("refs/heads/{branch_name}"))
        .map_err(|e| e.message().to_string())?;

    repo.checkout_head(Some(CheckoutBuilder::new().safe()))
        .map_err(|e| e.message().to_string())?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::{Repository, Signature};
    use std::fs;
    use tempfile::TempDir;

    fn create_test_repo() -> (TempDir, Repository) {
        let dir = TempDir::new().unwrap();
        let repo = Repository::init(dir.path()).unwrap();

        let mut config = repo.config().unwrap();
        config.set_str("user.name", "Test User").unwrap();
        config.set_str("user.email", "test@example.com").unwrap();

        (dir, repo)
    }

    fn create_initial_commit(repo: &Repository) -> git2::Oid {
        let sig = Signature::now("Test User", "test@example.com").unwrap();
        let tree_id = repo.index().unwrap().write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "initial commit", &tree, &[])
            .unwrap()
    }

    fn add_and_commit(repo: &Repository, path: &str, content: &str, message: &str) -> git2::Oid {
        let workdir = repo.workdir().unwrap();
        fs::write(workdir.join(path), content).unwrap();

        let mut index = repo.index().unwrap();
        index.add_path(Path::new(path)).unwrap();
        index.write().unwrap();

        let sig = Signature::now("Test User", "test@example.com").unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let parent = repo.head().unwrap().peel_to_commit().unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &[&parent])
            .unwrap()
    }

    #[test]
    fn test_get_git_status_untracked() {
        let (dir, _repo) = create_test_repo();
        create_initial_commit(&_repo);
        fs::write(dir.path().join("new_file.txt"), "hello").unwrap();

        let result = get_git_status(dir.path().to_str().unwrap().to_string()).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].path, "new_file.txt");
        assert_eq!(result[0].worktree_status, "new");
        assert_eq!(result[0].index_status, "none");
    }

    #[test]
    fn test_get_git_status_staged() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        fs::write(dir.path().join("staged.txt"), "content").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("staged.txt")).unwrap();
        index.write().unwrap();

        let result = get_git_status(dir.path().to_str().unwrap().to_string()).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].path, "staged.txt");
        assert_eq!(result[0].index_status, "new");
    }

    #[test]
    fn test_get_git_status_modified() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "file.txt", "original", "add file");

        fs::write(dir.path().join("file.txt"), "modified content").unwrap();

        let result = get_git_status(dir.path().to_str().unwrap().to_string()).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].path, "file.txt");
        assert_eq!(result[0].worktree_status, "modified");
    }

    #[test]
    fn test_get_git_status_empty_repo() {
        let (dir, _repo) = create_test_repo();

        let result = get_git_status(dir.path().to_str().unwrap().to_string()).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_get_git_log() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "a.txt", "a", "second commit");
        add_and_commit(&repo, "b.txt", "b", "third commit");

        let result = get_git_log(dir.path().to_str().unwrap().to_string(), None).unwrap();
        assert_eq!(result.len(), 3);

        let messages: Vec<&str> = result.iter().map(|c| c.message.as_str()).collect();
        assert!(messages.contains(&"initial commit"));
        assert!(messages.contains(&"second commit"));
        assert!(messages.contains(&"third commit"));

        assert_eq!(result[0].author_name, "Test User");
        assert_eq!(result[0].short_hash.len(), 7);
    }

    #[test]
    fn test_get_git_log_with_limit() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "a.txt", "a", "second");

        let result = get_git_log(dir.path().to_str().unwrap().to_string(), Some(1)).unwrap();
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_get_git_log_empty_repo() {
        let (dir, _repo) = create_test_repo();

        let result = get_git_log(dir.path().to_str().unwrap().to_string(), None).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_get_current_branch() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        let result = get_current_branch(dir.path().to_str().unwrap().to_string()).unwrap();
        assert!(result == "main" || result == "master");
    }

    #[test]
    fn test_get_current_branch_empty_repo() {
        let (dir, _repo) = create_test_repo();

        let result = get_current_branch(dir.path().to_str().unwrap().to_string()).unwrap();
        assert_eq!(result, "(no commits)");
    }

    #[test]
    fn test_get_current_branch_detached_head() {
        let (dir, repo) = create_test_repo();
        let oid = create_initial_commit(&repo);

        repo.set_head_detached(oid).unwrap();

        let result = get_current_branch(dir.path().to_str().unwrap().to_string()).unwrap();
        assert!(result.starts_with('('));
        assert!(result.ends_with(')'));
        assert_eq!(result.len(), 9); // "(1234567)"
    }

    // --- git_stage tests ---

    #[test]
    fn test_stage_specific_file() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        fs::write(dir.path().join("new.txt"), "hello").unwrap();

        git_stage(
            dir.path().to_str().unwrap().to_string(),
            vec!["new.txt".to_string()],
        )
        .unwrap();

        let statuses = get_git_status(dir.path().to_str().unwrap().to_string()).unwrap();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].index_status, "new");
        assert_eq!(statuses[0].worktree_status, "none");
    }

    #[test]
    fn test_stage_all_files() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        fs::write(dir.path().join("a.txt"), "a").unwrap();
        fs::write(dir.path().join("b.txt"), "b").unwrap();

        git_stage(dir.path().to_str().unwrap().to_string(), vec![]).unwrap();

        let statuses = get_git_status(dir.path().to_str().unwrap().to_string()).unwrap();
        assert_eq!(statuses.len(), 2);
        for s in &statuses {
            assert_eq!(s.index_status, "new");
            assert_eq!(s.worktree_status, "none");
        }
    }

    #[test]
    fn test_stage_deleted_file() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "file.txt", "content", "add file");
        fs::remove_file(dir.path().join("file.txt")).unwrap();

        git_stage(
            dir.path().to_str().unwrap().to_string(),
            vec!["file.txt".to_string()],
        )
        .unwrap();

        let statuses = get_git_status(dir.path().to_str().unwrap().to_string()).unwrap();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].index_status, "deleted");
    }

    #[test]
    fn test_stage_untracked_file() {
        let (dir, _repo) = create_test_repo();
        create_initial_commit(&_repo);
        fs::write(dir.path().join("untracked.txt"), "data").unwrap();

        let before = get_git_status(dir.path().to_str().unwrap().to_string()).unwrap();
        assert_eq!(before[0].worktree_status, "new");

        git_stage(
            dir.path().to_str().unwrap().to_string(),
            vec!["untracked.txt".to_string()],
        )
        .unwrap();

        let after = get_git_status(dir.path().to_str().unwrap().to_string()).unwrap();
        assert_eq!(after[0].index_status, "new");
        assert_eq!(after[0].worktree_status, "none");
    }

    // --- git_unstage tests ---

    #[test]
    fn test_unstage_specific_file() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        fs::write(dir.path().join("file.txt"), "content").unwrap();
        git_stage(
            dir.path().to_str().unwrap().to_string(),
            vec!["file.txt".to_string()],
        )
        .unwrap();

        git_unstage(
            dir.path().to_str().unwrap().to_string(),
            vec!["file.txt".to_string()],
        )
        .unwrap();

        let statuses = get_git_status(dir.path().to_str().unwrap().to_string()).unwrap();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].worktree_status, "new");
        assert_eq!(statuses[0].index_status, "none");
    }

    #[test]
    fn test_unstage_all_files() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        fs::write(dir.path().join("a.txt"), "a").unwrap();
        fs::write(dir.path().join("b.txt"), "b").unwrap();
        git_stage(dir.path().to_str().unwrap().to_string(), vec![]).unwrap();

        git_unstage(dir.path().to_str().unwrap().to_string(), vec![]).unwrap();

        let statuses = get_git_status(dir.path().to_str().unwrap().to_string()).unwrap();
        for s in &statuses {
            assert_eq!(s.index_status, "none");
            assert_eq!(s.worktree_status, "new");
        }
    }

    #[test]
    fn test_unstage_unborn_branch() {
        let (dir, _repo) = create_test_repo();
        // No initial commit — unborn branch
        fs::write(dir.path().join("file.txt"), "content").unwrap();
        git_stage(
            dir.path().to_str().unwrap().to_string(),
            vec!["file.txt".to_string()],
        )
        .unwrap();

        // Verify it was staged
        let before = get_git_status(dir.path().to_str().unwrap().to_string()).unwrap();
        assert_eq!(before[0].index_status, "new");

        git_unstage(
            dir.path().to_str().unwrap().to_string(),
            vec!["file.txt".to_string()],
        )
        .unwrap();

        let after = get_git_status(dir.path().to_str().unwrap().to_string()).unwrap();
        assert_eq!(after[0].index_status, "none");
        assert_eq!(after[0].worktree_status, "new");
    }

    // --- git_commit tests ---

    #[test]
    fn test_commit_normal() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        fs::write(dir.path().join("file.txt"), "content").unwrap();
        git_stage(
            dir.path().to_str().unwrap().to_string(),
            vec!["file.txt".to_string()],
        )
        .unwrap();

        let hash = git_commit(
            dir.path().to_str().unwrap().to_string(),
            "test commit".to_string(),
        )
        .unwrap();
        assert_eq!(hash.len(), 40);

        let log = get_git_log(dir.path().to_str().unwrap().to_string(), Some(1)).unwrap();
        assert_eq!(log[0].message, "test commit");
    }

    #[test]
    fn test_commit_initial() {
        let (dir, _repo) = create_test_repo();
        // No initial commit
        fs::write(dir.path().join("file.txt"), "content").unwrap();
        git_stage(
            dir.path().to_str().unwrap().to_string(),
            vec!["file.txt".to_string()],
        )
        .unwrap();

        let hash = git_commit(
            dir.path().to_str().unwrap().to_string(),
            "first commit".to_string(),
        )
        .unwrap();
        assert_eq!(hash.len(), 40);

        let log = get_git_log(dir.path().to_str().unwrap().to_string(), None).unwrap();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].message, "first commit");
    }

    // --- git_create_branch tests ---

    #[test]
    fn test_create_branch() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        git_create_branch(
            dir.path().to_str().unwrap().to_string(),
            "feature".to_string(),
        )
        .unwrap();

        let branch = get_current_branch(dir.path().to_str().unwrap().to_string()).unwrap();
        assert_eq!(branch, "feature");
    }

    #[test]
    fn test_create_branch_already_exists() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        git_create_branch(
            dir.path().to_str().unwrap().to_string(),
            "feature".to_string(),
        )
        .unwrap();

        let result = git_create_branch(
            dir.path().to_str().unwrap().to_string(),
            "feature".to_string(),
        );
        assert!(result.is_err());
    }
}
