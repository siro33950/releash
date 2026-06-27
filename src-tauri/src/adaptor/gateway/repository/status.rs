//! status 責務の gateway 実装。git2 による作業ツリー状態取得を封じ込める。

use crate::domain::repository::{
    FileDiffStat, FileStatus, RepositoryError, RepositoryStatusScan, StatusRepository,
};
use crate::infrastructure::git::client;
use git2::{ErrorCode, Repository, StatusOptions};
use std::collections::HashMap;

#[cfg(test)]
thread_local! {
    static STATUS_WALK_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn reset_status_walk_count_for_tests() {
    STATUS_WALK_COUNT.with(|count| count.set(0));
}

#[cfg(test)]
pub(crate) fn status_walk_count_for_tests() -> usize {
    STATUS_WALK_COUNT.with(|count| count.get())
}

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

#[cfg(test)]
pub(crate) fn get_git_status(repo_path: &str) -> Result<Vec<FileStatus>, RepositoryError> {
    get_git_status_with_options(repo_path, false)
}

pub(crate) fn get_git_status_with_options(
    repo_path: &str,
    include_ignored: bool,
) -> Result<Vec<FileStatus>, RepositoryError> {
    let result = crate::other::telemetry::measure_result(
        crate::other::telemetry::HotPath::GitStatusScan,
        || get_git_status_inner(repo_path, include_ignored),
    );
    if result.is_ok() {
        crate::other::telemetry::record_first_repo_snapshot_ready();
    }
    result
}

fn get_git_status_inner(
    repo_path: &str,
    include_ignored: bool,
) -> Result<Vec<FileStatus>, RepositoryError> {
    let repo = client::open(repo_path)?;
    collect_git_status(&repo, include_ignored)
}

fn collect_git_status(
    repo: &Repository,
    include_ignored: bool,
) -> Result<Vec<FileStatus>, RepositoryError> {
    let mut opts = StatusOptions::new();
    opts.include_untracked(true).recurse_untracked_dirs(true);
    if include_ignored {
        opts.include_ignored(true);
    }

    #[cfg(test)]
    STATUS_WALK_COUNT.with(|count| count.set(count.get() + 1));
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

#[cfg(test)]
pub(crate) fn get_status_diff_stats(repo_path: &str) -> Result<Vec<FileDiffStat>, RepositoryError> {
    crate::other::telemetry::measure_result(crate::other::telemetry::HotPath::DiffStats, || {
        get_status_diff_stats_inner(repo_path)
    })
}

#[cfg(test)]
fn get_status_diff_stats_inner(repo_path: &str) -> Result<Vec<FileDiffStat>, RepositoryError> {
    let repo = client::open(repo_path)?;
    collect_status_diff_stats(&repo)
}

fn collect_status_diff_stats(repo: &Repository) -> Result<Vec<FileDiffStat>, RepositoryError> {
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

pub(crate) fn get_repository_status_scan(
    repo_path: &str,
) -> Result<RepositoryStatusScan, RepositoryError> {
    let result = crate::other::telemetry::measure_result(
        crate::other::telemetry::HotPath::GitStatusScan,
        || get_repository_status_scan_inner(repo_path),
    );
    if result.is_ok() {
        crate::other::telemetry::record_first_repo_snapshot_ready();
    }
    result
}

fn get_repository_status_scan_inner(
    repo_path: &str,
) -> Result<RepositoryStatusScan, RepositoryError> {
    let repo = client::open(repo_path)?;
    let status = collect_git_status(&repo, false)?;
    let dirty_count = status
        .iter()
        .filter(|entry| entry.worktree_status != "ignored")
        .count();
    let diff_stats = crate::other::telemetry::measure_result(
        crate::other::telemetry::HotPath::DiffStats,
        || collect_status_diff_stats(&repo),
    )?;

    Ok(RepositoryStatusScan {
        status,
        diff_stats,
        dirty_count,
    })
}

/// `StatusRepository` の git2 実装。
pub struct StatusGateway;

impl StatusRepository for StatusGateway {
    fn status_with_options(
        &self,
        repo_path: &str,
        include_ignored: bool,
    ) -> Result<Vec<FileStatus>, RepositoryError> {
        get_git_status_with_options(repo_path, include_ignored)
    }
    fn status_scan(&self, repo_path: &str) -> Result<RepositoryStatusScan, RepositoryError> {
        get_repository_status_scan(repo_path)
    }
}

#[cfg(test)]
mod status_gateway_tests {
    use super::*;
    use crate::test_support::git::*;
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
    fn first_repo_snapshot_records_only_first_successful_status_scan() {
        let _guard = crate::other::telemetry::lock_test_telemetry();
        crate::other::telemetry::reset_test_metrics();
        crate::other::telemetry::set_performance_configured(true);
        crate::other::telemetry::set_performance_enabled(true);
        crate::other::telemetry::set_startup_origin(
            std::time::Instant::now() - std::time::Duration::from_millis(20),
        );

        let invalid = tempfile::TempDir::new().unwrap();
        assert!(get_git_status(invalid.path().to_str().unwrap()).is_err());
        assert!(!crate::other::telemetry::first_repo_snapshot_recorded_for_tests());
        assert!(crate::other::telemetry::test_metric_records()
            .iter()
            .all(|record| record.name != "releash.startup.duration_ms"));

        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        get_git_status(dir.path().to_str().unwrap()).unwrap();
        get_git_status(dir.path().to_str().unwrap()).unwrap();

        let startup_records: Vec<_> = crate::other::telemetry::test_metric_records()
            .into_iter()
            .filter(|record| record.name == "releash.startup.duration_ms")
            .collect();
        assert_eq!(startup_records.len(), 1);
        assert!(startup_records[0].value >= 20.0);
        assert!(startup_records[0].attributes.iter().any(|(key, value)| {
            key == "releash.operation" && value == "startup.first_repo_snapshot_ready"
        }));
        crate::other::telemetry::reset_test_metrics();
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
    fn test_状態取得_無視ファイルはdefaultで除外する() {
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

        assert!(
            result.iter().all(|e| e.worktree_status != "ignored"),
            "ignored entries should not appear in default status"
        );
        assert!(result.iter().all(|e| e.path != "ignored.txt"));
        assert!(result.iter().all(|e| e.path != "build"));
    }

    #[test]
    fn test_状態取得_無視ファイルはopt_inで含める() {
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

        let result = get_git_status_with_options(dir.path().to_str().unwrap(), true).unwrap();

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
    fn status_gateway_status_with_options_include_ignored_returns_ignored_entries() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        fs::write(dir.path().join(".gitignore"), "ignored.txt\n").unwrap();
        fs::write(dir.path().join("ignored.txt"), "should be ignored").unwrap();

        let gateway = StatusGateway;
        let result = <StatusGateway as StatusRepository>::status_with_options(
            &gateway,
            dir.path().to_str().unwrap(),
            true,
        )
        .unwrap();

        assert!(result
            .iter()
            .any(|entry| entry.path == "ignored.txt" && entry.worktree_status == "ignored"));
    }

    #[test]
    fn repository_status_scan_matches_legacy_status_and_diff_stats_with_one_status_walk() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "staged.txt", "before\n", "add staged");
        add_and_commit(&repo, "changed.txt", "before\n", "add changed");

        fs::write(dir.path().join("staged.txt"), "before\nafter\n").unwrap();
        {
            let mut index = repo.index().unwrap();
            index.add_path(Path::new("staged.txt")).unwrap();
            index.write().unwrap();
        }
        fs::write(dir.path().join("changed.txt"), "changed\n").unwrap();
        fs::write(dir.path().join("untracked.txt"), "new\n").unwrap();

        let repo_path = dir.path().to_str().unwrap();
        let expected_status = get_git_status(repo_path).unwrap();
        let expected_diff_stats = get_status_diff_stats(repo_path).unwrap();

        reset_status_walk_count_for_tests();
        let scan = get_repository_status_scan(repo_path).unwrap();

        assert_eq!(scan.status, expected_status);
        assert_eq!(scan.diff_stats, expected_diff_stats);
        assert_eq!(scan.dirty_count, expected_status.len());
        assert_eq!(status_walk_count_for_tests(), 1);
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
