//! staging（差分 Approve）責務の gateway 実装。git2 index 操作と `git apply --cached`
//! を封じ込める。

use git2::{ErrorCode, Repository, StatusOptions};
use std::path::Path;
use std::process::Command;

use crate::domain::code::{CodeError, StagingRepository};

pub(crate) fn git_stage(repo_path: &str, paths: Vec<String>) -> Result<(), CodeError> {
    let repo = Repository::open(repo_path)?;
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
        .ok_or_else(|| CodeError::Rule("bare repository".to_string()))?;

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

pub(crate) fn git_unstage(repo_path: &str, paths: Vec<String>) -> Result<(), CodeError> {
    let repo = Repository::open(repo_path)?;

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

pub(crate) fn git_stage_hunk(repo_path: &str, patch: &str) -> Result<(), CodeError> {
    Repository::open(repo_path)?;

    let mut child = Command::new("git")
        .args(["apply", "--cached"])
        .current_dir(repo_path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| CodeError::Rule(format!("Failed to execute git apply: {e}")))?;

    {
        use std::io::Write;
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| CodeError::Rule("Failed to open stdin for git apply".to_string()))?;
        stdin
            .write_all(patch.as_bytes())
            .map_err(|e| CodeError::Rule(format!("Failed to write patch: {e}")))?;
    }

    let output = child
        .wait_with_output()
        .map_err(|e| CodeError::Rule(format!("Failed to wait for git apply: {e}")))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(CodeError::Rule(stderr.trim().to_string()))
    }
}

pub(crate) fn git_unstage_hunk(repo_path: &str, patch: &str) -> Result<(), CodeError> {
    Repository::open(repo_path)?;

    let mut child = Command::new("git")
        .args(["apply", "--cached", "--reverse"])
        .current_dir(repo_path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| CodeError::Rule(format!("Failed to execute git apply: {e}")))?;

    {
        use std::io::Write;
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| CodeError::Rule("Failed to open stdin for git apply".to_string()))?;
        stdin
            .write_all(patch.as_bytes())
            .map_err(|e| CodeError::Rule(format!("Failed to write patch: {e}")))?;
    }

    let output = child
        .wait_with_output()
        .map_err(|e| CodeError::Rule(format!("Failed to wait for git apply: {e}")))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(CodeError::Rule(stderr.trim().to_string()))
    }
}

/// `StagingRepository` の git2 / git CLI 実装。
pub struct StagingGateway;

impl StagingRepository for StagingGateway {
    fn stage(&self, repo_path: &str, paths: Vec<String>) -> Result<(), CodeError> {
        git_stage(repo_path, paths)
    }
    fn unstage(&self, repo_path: &str, paths: Vec<String>) -> Result<(), CodeError> {
        git_unstage(repo_path, paths)
    }
    fn stage_hunk(&self, repo_path: &str, patch: &str) -> Result<(), CodeError> {
        git_stage_hunk(repo_path, patch)
    }
    fn unstage_hunk(&self, repo_path: &str, patch: &str) -> Result<(), CodeError> {
        git_unstage_hunk(repo_path, patch)
    }
}

#[cfg(test)]
mod staging_gateway_tests {
    use super::*;
    use crate::adaptor::gateway::code::diff_compute;
    use crate::adaptor::gateway::repository::status::get_git_status;
    use crate::domain::code::services::hunk as hunk_service;
    use crate::domain::code::{ChangeGroup, Hunk};
    use crate::test_support::git::*;
    use std::fs;
    use std::path::Path;

    fn index_file_content(repo: &Repository, path: &str) -> String {
        let mut index = repo.index().unwrap();
        index.read(true).unwrap();
        let entry = index.get_path(Path::new(path), 0).unwrap();
        let blob = repo.find_blob(entry.id).unwrap();
        std::str::from_utf8(blob.content()).unwrap().to_string()
    }

    fn diff_hunks_and_groups(original: &str, modified: &str) -> (Vec<Hunk>, Vec<ChangeGroup>) {
        let raw_hunks = diff_compute::diff_buffers(original, modified, Some("file.txt"));
        let hunks = hunk_service::assign_hunk_ids(&raw_hunks);
        let groups = hunk_service::compute_change_groups(&hunks);
        (hunks, groups)
    }

    fn group_patch(file_path: &str, hunks: &[Hunk], group: &ChangeGroup) -> String {
        let hunk = hunks
            .iter()
            .find(|hunk| hunk.index == group.hunk_index)
            .unwrap();
        hunk_service::generate_group_patch(file_path, hunk, group)
    }

    #[test]
    fn test_stage_特定ファイル() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        fs::write(dir.path().join("new.txt"), "hello").unwrap();

        git_stage(dir.path().to_str().unwrap(), vec!["new.txt".to_string()]).unwrap();

        let statuses = get_git_status(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].index_status, "new");
        assert_eq!(statuses[0].worktree_status, "none");
    }

    #[test]
    fn test_stage_全ファイル() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        fs::write(dir.path().join("a.txt"), "a").unwrap();
        fs::write(dir.path().join("b.txt"), "b").unwrap();

        git_stage(dir.path().to_str().unwrap(), vec![]).unwrap();

        let statuses = get_git_status(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(statuses.len(), 2);
        for s in &statuses {
            assert_eq!(s.index_status, "new");
            assert_eq!(s.worktree_status, "none");
        }
    }

    #[test]
    fn test_stage_削除ファイル() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "file.txt", "content", "add file");
        fs::remove_file(dir.path().join("file.txt")).unwrap();

        git_stage(dir.path().to_str().unwrap(), vec!["file.txt".to_string()]).unwrap();

        let statuses = get_git_status(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].index_status, "deleted");
        assert_eq!(statuses[0].worktree_status, "none");
    }

    #[test]
    fn test_stage_未追跡ファイル() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        fs::write(dir.path().join("untracked.txt"), "data").unwrap();

        let before = get_git_status(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(before[0].worktree_status, "new");

        git_stage(
            dir.path().to_str().unwrap(),
            vec!["untracked.txt".to_string()],
        )
        .unwrap();

        let after = get_git_status(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(after[0].index_status, "new");
        assert_eq!(after[0].worktree_status, "none");
    }

    #[test]
    fn test_unstage_特定ファイル() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        fs::write(dir.path().join("file.txt"), "content").unwrap();
        git_stage(dir.path().to_str().unwrap(), vec!["file.txt".to_string()]).unwrap();

        git_unstage(dir.path().to_str().unwrap(), vec!["file.txt".to_string()]).unwrap();

        let statuses = get_git_status(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].worktree_status, "new");
        assert_eq!(statuses[0].index_status, "none");
    }

    #[test]
    fn test_unstage_全ファイル() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        fs::write(dir.path().join("a.txt"), "a").unwrap();
        fs::write(dir.path().join("b.txt"), "b").unwrap();
        git_stage(dir.path().to_str().unwrap(), vec![]).unwrap();

        git_unstage(dir.path().to_str().unwrap(), vec![]).unwrap();

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
        git_stage(dir.path().to_str().unwrap(), vec!["file.txt".to_string()]).unwrap();

        let before = get_git_status(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(before[0].index_status, "new");

        git_unstage(dir.path().to_str().unwrap(), vec!["file.txt".to_string()]).unwrap();

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

        git_stage_hunk(dir.path().to_str().unwrap(), patch).unwrap();

        let statuses = get_git_status(dir.path().to_str().unwrap()).unwrap();
        assert!(statuses.iter().any(|s| s.index_status == "modified"));
    }

    #[test]
    fn test_stage_hunk_連続適用はstaged内容で再計算したpatchなら成功する() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        let original = "line1\nline2\nline3\nline4\n";
        let modified = "line1\nchanged2\nline3\nchanged4\n";
        add_and_commit(&repo, "file.txt", original, "add file");
        fs::write(dir.path().join("file.txt"), modified).unwrap();

        let (hunks, groups) =
            diff_hunks_and_groups(&index_file_content(&repo, "file.txt"), modified);
        assert_eq!(groups.len(), 2);
        let first_group = groups[0].clone();
        let second_group_id = groups[1].group_id.clone();

        let first_patch = group_patch("file.txt", &hunks, &first_group);
        git_stage_hunk(dir.path().to_str().unwrap(), &first_patch).unwrap();
        assert_eq!(
            index_file_content(&repo, "file.txt"),
            "line1\nchanged2\nline3\nline4\n"
        );

        let staged = index_file_content(&repo, "file.txt");
        let (hunks_after_stage, groups_after_stage) = diff_hunks_and_groups(&staged, modified);
        let second_group = groups_after_stage
            .iter()
            .find(|group| group.group_id == second_group_id)
            .unwrap();
        let second_patch = group_patch("file.txt", &hunks_after_stage, second_group);
        git_stage_hunk(dir.path().to_str().unwrap(), &second_patch).unwrap();

        assert_eq!(index_file_content(&repo, "file.txt"), modified);
    }

    #[test]
    fn test_unstage_hunk() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "file.txt", "line1\nline2\nline3\n", "add file");

        fs::write(dir.path().join("file.txt"), "line1\nmodified\nline3\n").unwrap();
        git_stage(dir.path().to_str().unwrap(), vec!["file.txt".to_string()]).unwrap();

        let before = get_git_status(dir.path().to_str().unwrap()).unwrap();
        assert!(before.iter().any(|s| s.index_status == "modified"));

        let patch =
            "--- a/file.txt\n+++ b/file.txt\n@@ -1,3 +1,3 @@\n line1\n-line2\n+modified\n line3\n";
        git_unstage_hunk(dir.path().to_str().unwrap(), patch).unwrap();

        let after = get_git_status(dir.path().to_str().unwrap()).unwrap();
        let file_status = after.iter().find(|s| s.path == "file.txt").unwrap();
        assert_eq!(file_status.worktree_status, "modified");
        assert_eq!(file_status.index_status, "none");
    }
}
