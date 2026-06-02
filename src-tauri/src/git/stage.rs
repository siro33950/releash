use super::error::GitError;
use git2::{ErrorCode, Repository, StatusOptions};
use std::path::Path;
use std::process::Command;

pub fn git_stage(repo_path: String, paths: Vec<String>) -> Result<(), GitError> {
    let repo = Repository::open(&repo_path)?;
    let mut index = repo.index()?;

    let targets: Vec<String> = if paths.is_empty() {
        let mut opts = StatusOptions::new();
        opts.include_untracked(true)
            .recurse_untracked_dirs(true)
            .renames_index_to_workdir(true);
        let statuses = repo.statuses(Some(&mut opts))?;
        statuses
            .iter()
            .filter_map(|entry| {
                let s = entry.status();
                if s.contains(git2::Status::WT_NEW)
                    || s.contains(git2::Status::WT_MODIFIED)
                    || s.contains(git2::Status::WT_DELETED)
                    || s.contains(git2::Status::WT_RENAMED)
                    || s.contains(git2::Status::WT_TYPECHANGE)
                {
                    entry.path().ok().map(|p| p.to_string())
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
        .ok_or_else(|| GitError::Custom("bare repository".to_string()))?;

    for p in &targets {
        let full_path = workdir.join(p);
        if full_path.exists() {
            index.add_path(Path::new(p))?;
        } else {
            index.remove_path(Path::new(p))?;
        }
    }

    index.write()?;
    Ok(())
}

pub fn git_unstage(repo_path: String, paths: Vec<String>) -> Result<(), GitError> {
    let repo = Repository::open(&repo_path)?;

    let head_result = repo.head();
    let is_unborn = matches!(&head_result, Err(e) if e.code() == ErrorCode::UnbornBranch);

    if is_unborn {
        let mut index = repo.index()?;
        if paths.is_empty() {
            index.clear()?;
        } else {
            for p in &paths {
                index.remove_path(Path::new(p))?;
            }
        }
        index.write()?;
    } else {
        let head_ref = head_result?;
        let head_obj = head_ref.peel(git2::ObjectType::Any)?;

        let targets: Vec<String> = if paths.is_empty() {
            let mut opts = StatusOptions::new();
            opts.include_untracked(true).recurse_untracked_dirs(true);
            let statuses = repo.statuses(Some(&mut opts))?;
            statuses
                .iter()
                .filter_map(|entry| {
                    let s = entry.status();
                    if s.contains(git2::Status::INDEX_NEW)
                        || s.contains(git2::Status::INDEX_MODIFIED)
                        || s.contains(git2::Status::INDEX_DELETED)
                        || s.contains(git2::Status::INDEX_RENAMED)
                    {
                        entry.path().ok().map(|p| p.to_string())
                    } else {
                        None
                    }
                })
                .collect()
        } else {
            paths
        };

        let path_specs: Vec<&str> = targets.iter().map(|s| s.as_str()).collect();
        repo.reset_default(Some(&head_obj), &path_specs)?;
    }

    Ok(())
}

pub fn git_stage_hunk(repo_path: String, patch: String) -> Result<(), GitError> {
    Repository::open(&repo_path)?;

    let mut child = Command::new("git")
        .args(["apply", "--cached"])
        .current_dir(&repo_path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| GitError::Custom(format!("Failed to execute git apply: {e}")))?;

    {
        use std::io::Write;
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| GitError::Custom("Failed to open stdin for git apply".to_string()))?;
        stdin
            .write_all(patch.as_bytes())
            .map_err(|e| GitError::Custom(format!("Failed to write patch: {e}")))?;
    }

    let output = child
        .wait_with_output()
        .map_err(|e| GitError::Custom(format!("Failed to wait for git apply: {e}")))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(GitError::Custom(stderr.trim().to_string()))
    }
}

pub fn git_unstage_hunk(repo_path: String, patch: String) -> Result<(), GitError> {
    Repository::open(&repo_path)?;

    let mut child = Command::new("git")
        .args(["apply", "--cached", "--reverse"])
        .current_dir(&repo_path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| GitError::Custom(format!("Failed to execute git apply: {e}")))?;

    {
        use std::io::Write;
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| GitError::Custom("Failed to open stdin for git apply".to_string()))?;
        stdin
            .write_all(patch.as_bytes())
            .map_err(|e| GitError::Custom(format!("Failed to write patch: {e}")))?;
    }

    let output = child
        .wait_with_output()
        .map_err(|e| GitError::Custom(format!("Failed to wait for git apply: {e}")))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(GitError::Custom(stderr.trim().to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptor::gateway::repository::status::get_git_status;
    use crate::git::test_helpers::*;
    use std::fs;

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

        let statuses = get_git_status(dir.path().to_str().unwrap()).unwrap();
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

        let statuses = get_git_status(dir.path().to_str().unwrap()).unwrap();
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

        let statuses = get_git_status(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].index_status, "deleted");
        assert_eq!(statuses[0].worktree_status, "none");
    }

    #[test]
    fn test_stage_untracked_file() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        fs::write(dir.path().join("untracked.txt"), "data").unwrap();

        let before = get_git_status(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(before[0].worktree_status, "new");

        git_stage(
            dir.path().to_str().unwrap().to_string(),
            vec!["untracked.txt".to_string()],
        )
        .unwrap();

        let after = get_git_status(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(after[0].index_status, "new");
        assert_eq!(after[0].worktree_status, "none");
    }

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

        let statuses = get_git_status(dir.path().to_str().unwrap()).unwrap();
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

        let statuses = get_git_status(dir.path().to_str().unwrap()).unwrap();
        for s in &statuses {
            assert_eq!(s.index_status, "none");
            assert_eq!(s.worktree_status, "new");
        }
    }

    #[test]
    fn test_unstage_unborn_branch() {
        let (dir, _repo) = create_test_repo();
        fs::write(dir.path().join("file.txt"), "content").unwrap();
        git_stage(
            dir.path().to_str().unwrap().to_string(),
            vec!["file.txt".to_string()],
        )
        .unwrap();

        let before = get_git_status(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(before[0].index_status, "new");

        git_unstage(
            dir.path().to_str().unwrap().to_string(),
            vec!["file.txt".to_string()],
        )
        .unwrap();

        let after = get_git_status(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(after[0].index_status, "none");
        assert_eq!(after[0].worktree_status, "new");
    }

    #[test]
    fn test_stage_hunk() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "file.txt", "line1\nline2\nline3\n", "add file");

        fs::write(dir.path().join("file.txt"), "line1\nmodified\nline3\n").unwrap();

        let patch =
            "--- a/file.txt\n+++ b/file.txt\n@@ -1,3 +1,3 @@\n line1\n-line2\n+modified\n line3\n";

        git_stage_hunk(dir.path().to_str().unwrap().to_string(), patch.to_string()).unwrap();

        let statuses = get_git_status(dir.path().to_str().unwrap()).unwrap();
        assert!(statuses.iter().any(|s| s.index_status == "modified"));
    }

    #[test]
    fn test_unstage_hunk() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "file.txt", "line1\nline2\nline3\n", "add file");

        fs::write(dir.path().join("file.txt"), "line1\nmodified\nline3\n").unwrap();
        git_stage(
            dir.path().to_str().unwrap().to_string(),
            vec!["file.txt".to_string()],
        )
        .unwrap();

        let before = get_git_status(dir.path().to_str().unwrap()).unwrap();
        assert!(before.iter().any(|s| s.index_status == "modified"));

        let patch =
            "--- a/file.txt\n+++ b/file.txt\n@@ -1,3 +1,3 @@\n line1\n-line2\n+modified\n line3\n";
        git_unstage_hunk(dir.path().to_str().unwrap().to_string(), patch.to_string()).unwrap();

        let after = get_git_status(dir.path().to_str().unwrap()).unwrap();
        let file_status = after.iter().find(|s| s.path == "file.txt").unwrap();
        assert_eq!(file_status.worktree_status, "modified");
        assert_eq!(file_status.index_status, "none");
    }
}
