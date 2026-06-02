//! status 責務の gateway 実装。git2 による作業ツリー状態取得を封じ込める。

use crate::domain::repository::{FileDiffStat, FileStatus, RepositoryError, StatusRepository};
use crate::infrastructure::git::client;
use git2::{ErrorCode, StatusOptions};
use std::collections::HashMap;

fn index_status_from_flags(status: git2::Status) -> &'static str {
    if status.contains(git2::Status::CONFLICTED) {
        "modified"
    } else if status.contains(git2::Status::INDEX_NEW) {
        "new"
    } else if status.contains(git2::Status::INDEX_MODIFIED) {
        "modified"
    } else if status.contains(git2::Status::INDEX_DELETED) {
        "deleted"
    } else if status.contains(git2::Status::INDEX_RENAMED) {
        "renamed"
    } else if status.contains(git2::Status::INDEX_TYPECHANGE) {
        "modified"
    } else {
        "none"
    }
}

fn worktree_status_from_flags(status: git2::Status) -> &'static str {
    if status.contains(git2::Status::IGNORED) {
        "ignored"
    } else if status.contains(git2::Status::CONFLICTED) {
        "modified"
    } else if status.contains(git2::Status::WT_NEW) {
        "new"
    } else if status.contains(git2::Status::WT_MODIFIED) {
        "modified"
    } else if status.contains(git2::Status::WT_DELETED) {
        "deleted"
    } else if status.contains(git2::Status::WT_RENAMED)
        || status.contains(git2::Status::WT_TYPECHANGE)
    {
        "modified"
    } else {
        "none"
    }
}

pub(crate) fn get_git_status(repo_path: &str) -> Result<Vec<FileStatus>, RepositoryError> {
    let repo = client::open(repo_path)?;

    let mut opts = StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_ignored(true);

    let statuses = repo.statuses(Some(&mut opts))?;

    let result: Vec<FileStatus> = statuses
        .iter()
        .filter_map(|entry| {
            let path = entry.path().ok()?.to_string();
            let path = path.trim_end_matches('/').to_string();
            let status = entry.status();
            let idx = index_status_from_flags(status);
            let wt = worktree_status_from_flags(status);
            if idx == "none" && wt == "none" {
                return None;
            }
            Some(FileStatus {
                path,
                index_status: idx.to_string(),
                worktree_status: wt.to_string(),
            })
        })
        .collect();

    Ok(result)
}

fn count_patch_lines(diff: &git2::Diff, idx: usize) -> (u32, u32) {
    let patch = match git2::Patch::from_diff(diff, idx) {
        Ok(Some(p)) => p,
        _ => return (0, 0),
    };
    let mut adds = 0u32;
    let mut dels = 0u32;
    for h in 0..patch.num_hunks() {
        let lines = match patch.num_lines_in_hunk(h) {
            Ok(n) => n,
            Err(_) => continue,
        };
        for l in 0..lines {
            if let Ok(line) = patch.line_in_hunk(h, l) {
                match line.origin() {
                    '+' => adds += 1,
                    '-' => dels += 1,
                    _ => {}
                }
            }
        }
    }
    (adds, dels)
}

fn collect_diff_stats(diff: &git2::Diff) -> HashMap<String, (u32, u32)> {
    let mut map = HashMap::new();
    let num_deltas = diff.deltas().len();
    for i in 0..num_deltas {
        let delta = diff.get_delta(i).unwrap();
        let path = delta
            .new_file()
            .path()
            .or_else(|| delta.old_file().path())
            .map(|p| p.to_string_lossy().to_string());
        if let Some(path) = path {
            if delta.new_file().is_binary() || delta.old_file().is_binary() {
                map.insert(path, (0, 0));
            } else {
                let (adds, dels) = count_patch_lines(diff, i);
                map.insert(path, (adds, dels));
            }
        }
    }
    map
}

pub(crate) fn get_status_diff_stats(repo_path: &str) -> Result<Vec<FileDiffStat>, RepositoryError> {
    let repo = client::open(repo_path)?;

    // HEAD tree (may not exist for unborn branch)
    let head_tree = match repo.head() {
        Ok(head) => Some(head.peel_to_tree()?),
        Err(err) if err.code() == ErrorCode::UnbornBranch => None,
        Err(err) => return Err(err.into()),
    };

    // Index diff stats: HEAD → index (staged changes)
    let index_diff = repo.diff_tree_to_index(head_tree.as_ref(), None, None)?;
    let index_stats = collect_diff_stats(&index_diff);

    // Worktree diff stats: index → worktree (unstaged changes)
    let mut wt_opts = git2::DiffOptions::new();
    wt_opts
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .show_untracked_content(true);
    let wt_diff = repo.diff_index_to_workdir(None, Some(&mut wt_opts))?;
    let wt_stats = collect_diff_stats(&wt_diff);

    // Merge all paths
    let mut all_paths = std::collections::BTreeSet::new();
    for path in index_stats.keys() {
        all_paths.insert(path.clone());
    }
    for path in wt_stats.keys() {
        all_paths.insert(path.clone());
    }

    let result = all_paths
        .into_iter()
        .map(|path| {
            let (ia, id) = index_stats.get(&path).copied().unwrap_or((0, 0));
            let (wa, wd) = wt_stats.get(&path).copied().unwrap_or((0, 0));
            FileDiffStat {
                path,
                index_additions: ia,
                index_deletions: id,
                wt_additions: wa,
                wt_deletions: wd,
            }
        })
        .collect();

    Ok(result)
}

/// `StatusRepository` の git2 実装。
pub struct StatusGateway;

impl StatusRepository for StatusGateway {
    fn status(&self, repo_path: &str) -> Result<Vec<FileStatus>, RepositoryError> {
        get_git_status(repo_path)
    }
    fn diff_stats(&self, repo_path: &str) -> Result<Vec<FileDiffStat>, RepositoryError> {
        get_status_diff_stats(repo_path)
    }
}

#[cfg(test)]
mod status_gateway_tests {
    use super::*;
    use crate::git::test_helpers::*;
    use std::fs;
    use std::path::Path;

    #[test]
    fn test_状態取得_未追跡ファイル() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        fs::write(dir.path().join("new_file.txt"), "hello").unwrap();

        let result = get_git_status(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].path, "new_file.txt");
        assert_eq!(result[0].worktree_status, "new");
        assert_eq!(result[0].index_status, "none");
    }

    #[test]
    fn test_状態取得_ステージ済み() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        fs::write(dir.path().join("staged.txt"), "content").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("staged.txt")).unwrap();
        index.write().unwrap();

        let result = get_git_status(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].path, "staged.txt");
        assert_eq!(result[0].index_status, "new");
    }

    #[test]
    fn test_状態取得_変更済み() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "file.txt", "original", "add file");

        fs::write(dir.path().join("file.txt"), "modified content").unwrap();

        let result = get_git_status(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].path, "file.txt");
        assert_eq!(result[0].worktree_status, "modified");
    }

    #[test]
    fn test_状態取得_空リポジトリ() {
        let (dir, _repo) = create_test_repo();

        let result = get_git_status(dir.path().to_str().unwrap()).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_状態取得_無視ファイル() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        fs::write(dir.path().join(".gitignore"), "ignored.txt\nbuild/\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new(".gitignore")).unwrap();
        index.write().unwrap();
        let sig = git2::Signature::now("Test User", "test@example.com").unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let parent = repo.head().unwrap().peel_to_commit().unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "add gitignore", &tree, &[&parent])
            .unwrap();

        fs::write(dir.path().join("ignored.txt"), "should be ignored").unwrap();
        fs::create_dir(dir.path().join("build")).unwrap();
        fs::write(dir.path().join("build").join("output.js"), "built").unwrap();

        let result = get_git_status(dir.path().to_str().unwrap()).unwrap();

        let ignored_file = result.iter().find(|e| e.path == "ignored.txt");
        assert!(
            ignored_file.is_some(),
            "ignored.txt should appear in status"
        );
        assert_eq!(ignored_file.unwrap().worktree_status, "ignored");
        assert_eq!(ignored_file.unwrap().index_status, "none");

        let ignored_dir = result.iter().find(|e| e.path == "build");
        assert!(ignored_dir.is_some(), "build dir should appear in status");
        assert_eq!(ignored_dir.unwrap().worktree_status, "ignored");
    }

    #[test]
    fn test_差分統計_ステージ済み新規ファイル() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        fs::write(dir.path().join("new.txt"), "line1\nline2\nline3\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("new.txt")).unwrap();
        index.write().unwrap();

        let stats = get_status_diff_stats(dir.path().to_str().unwrap()).unwrap();
        let file = stats.iter().find(|s| s.path == "new.txt").unwrap();
        assert_eq!(file.index_additions, 3);
        assert_eq!(file.index_deletions, 0);
        assert_eq!(file.wt_additions, 0);
        assert_eq!(file.wt_deletions, 0);
    }

    #[test]
    fn test_差分統計_作業ツリー変更() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "file.txt", "original\n", "add file");

        fs::write(dir.path().join("file.txt"), "original\nmodified\n").unwrap();

        let stats = get_status_diff_stats(dir.path().to_str().unwrap()).unwrap();
        let file = stats.iter().find(|s| s.path == "file.txt").unwrap();
        assert_eq!(file.index_additions, 0);
        assert_eq!(file.index_deletions, 0);
        assert_eq!(file.wt_additions, 1);
        assert_eq!(file.wt_deletions, 0);
    }

    #[test]
    fn test_差分統計_ステージと作業ツリー両方() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "file.txt", "line1\nline2\n", "add file");

        fs::write(dir.path().join("file.txt"), "line1\nline2\nline3\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("file.txt")).unwrap();
        index.write().unwrap();

        fs::write(dir.path().join("file.txt"), "line1\nline2\nline3\nline4\n").unwrap();

        let stats = get_status_diff_stats(dir.path().to_str().unwrap()).unwrap();
        let file = stats.iter().find(|s| s.path == "file.txt").unwrap();
        assert_eq!(file.index_additions, 1); // line3 staged
        assert_eq!(file.index_deletions, 0);
        assert_eq!(file.wt_additions, 1); // line4 unstaged
        assert_eq!(file.wt_deletions, 0);
    }

    #[test]
    fn test_差分統計_未追跡新規ファイル() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        fs::write(dir.path().join("new.txt"), "line1\nline2\n").unwrap();

        let stats = get_status_diff_stats(dir.path().to_str().unwrap()).unwrap();
        let file = stats.iter().find(|s| s.path == "new.txt").unwrap();
        assert_eq!(file.index_additions, 0);
        assert_eq!(file.index_deletions, 0);
        assert_eq!(file.wt_additions, 2);
        assert_eq!(file.wt_deletions, 0);
    }

    #[test]
    fn test_差分統計_空() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        let stats = get_status_diff_stats(dir.path().to_str().unwrap()).unwrap();
        assert!(stats.is_empty());
    }
}
