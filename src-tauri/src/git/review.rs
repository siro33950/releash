use std::collections::HashMap;

use super::error::GitError;
use git2::Repository;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct ReviewDiff {
    pub base_ref: String,
    pub changed_files: Vec<ChangedFile>,
    pub stats: DiffStats,
}

#[derive(Debug, Serialize)]
pub struct FileStats {
    pub additions: usize,
    pub deletions: usize,
}

#[derive(Debug, Serialize)]
pub struct ChangedFile {
    pub path: String,
    pub old_path: Option<String>,
    pub status: String,
    pub binary: bool,
    pub stats: FileStats,
    pub hunks: Vec<ReviewHunk>,
    pub truncated: bool,
}

#[derive(Debug, Serialize)]
pub struct ReviewHunk {
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub header: String,
    pub lines: Vec<ReviewLine>,
}

#[derive(Debug, Serialize)]
pub struct ReviewLine {
    pub origin: char,
    pub content: String,
    pub old_lineno: Option<u32>,
    pub new_lineno: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct DiffStats {
    pub files_changed: usize,
    pub insertions: usize,
    pub deletions: usize,
}

pub fn get_review_diff(
    repo_path: &str,
    base_branch: Option<&str>,
    paths: Option<&[String]>,
    max_lines_per_file: Option<usize>,
) -> Result<ReviewDiff, GitError> {
    let repo = Repository::open(repo_path)?;
    let (base_ref, base_commit) = find_base_commit(&repo, base_branch)?;
    let base_tree = base_commit.tree()?;

    let mut opts = git2::DiffOptions::new();
    opts.include_untracked(false);

    let diff = repo.diff_tree_to_workdir_with_index(Some(&base_tree), Some(&mut opts))?;

    let diff_stats = diff.stats()?;
    let stats = DiffStats {
        files_changed: diff_stats.files_changed(),
        insertions: diff_stats.insertions(),
        deletions: diff_stats.deletions(),
    };

    let mut changed_files: Vec<ChangedFile> = Vec::new();
    let num_deltas = diff.deltas().len();

    for i in 0..num_deltas {
        let delta = diff
            .get_delta(i)
            .ok_or_else(|| GitError::Custom(format!("invalid delta index: {i}")))?;

        let path = delta
            .new_file()
            .path()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        let old_path = if delta.status() == git2::Delta::Renamed {
            delta
                .old_file()
                .path()
                .map(|p| p.to_string_lossy().to_string())
        } else {
            None
        };
        let status = match delta.status() {
            git2::Delta::Added => "added",
            git2::Delta::Deleted => "deleted",
            git2::Delta::Modified => "modified",
            git2::Delta::Renamed => "renamed",
            git2::Delta::Copied => "copied",
            _ => "modified",
        };

        let include_hunks = paths
            .map(|ps| ps.iter().any(|p| p == &path))
            .unwrap_or(false);

        // In detail mode, skip files not in the requested paths
        if paths.is_some() && !include_hunks {
            continue;
        }

        let (binary, file_stats, hunks, truncated) = match git2::Patch::from_diff(&diff, i)? {
            Some(patch) => {
                let (_, additions, deletions) = patch.line_stats()?;
                let file_stats = FileStats {
                    additions,
                    deletions,
                };

                let total_diff_lines = additions + deletions;
                let should_truncate =
                    include_hunks && max_lines_per_file.is_some_and(|max| total_diff_lines > max);

                let (hunks, truncated) = if should_truncate {
                    // Truncated: include hunk headers (line ranges) but omit line content
                    let mut hunks = Vec::new();
                    for h in 0..patch.num_hunks() {
                        let (hunk, _) = patch.hunk(h)?;
                        let header = String::from_utf8_lossy(hunk.header())
                            .trim_end()
                            .to_string();
                        hunks.push(ReviewHunk {
                            old_start: hunk.old_start(),
                            old_lines: hunk.old_lines(),
                            new_start: hunk.new_start(),
                            new_lines: hunk.new_lines(),
                            header,
                            lines: Vec::new(),
                        });
                    }
                    (hunks, true)
                } else if include_hunks {
                    let mut hunks = Vec::new();
                    for h in 0..patch.num_hunks() {
                        let (hunk, num_lines) = patch.hunk(h)?;
                        let header = String::from_utf8_lossy(hunk.header())
                            .trim_end()
                            .to_string();

                        let mut lines = Vec::new();
                        for l in 0..num_lines {
                            let line = patch.line_in_hunk(h, l)?;
                            lines.push(ReviewLine {
                                origin: line.origin(),
                                content: String::from_utf8_lossy(line.content()).to_string(),
                                old_lineno: line.old_lineno(),
                                new_lineno: line.new_lineno(),
                            });
                        }

                        hunks.push(ReviewHunk {
                            old_start: hunk.old_start(),
                            old_lines: hunk.old_lines(),
                            new_start: hunk.new_start(),
                            new_lines: hunk.new_lines(),
                            header,
                            lines,
                        });
                    }
                    (hunks, false)
                } else {
                    (Vec::new(), false)
                };
                (false, file_stats, hunks, truncated)
            }
            None => (
                true,
                FileStats {
                    additions: 0,
                    deletions: 0,
                },
                Vec::new(),
                false,
            ),
        };

        changed_files.push(ChangedFile {
            path,
            old_path,
            status: status.to_string(),
            binary,
            stats: file_stats,
            hunks,
            truncated,
        });
    }

    Ok(ReviewDiff {
        base_ref,
        changed_files,
        stats,
    })
}

/// Returns a map of file paths to their hunk ranges (new_start, new_lines).
/// This is a lightweight alternative to `get_review_diff` that only extracts
/// hunk metadata without line content.
pub fn get_hunk_ranges(
    repo_path: &str,
    base_branch: Option<&str>,
) -> Result<HashMap<String, Vec<(u32, u32)>>, GitError> {
    let repo = Repository::open(repo_path)?;
    let (_base_ref, base_commit) = find_base_commit(&repo, base_branch)?;
    let base_tree = base_commit.tree()?;

    let mut opts = git2::DiffOptions::new();
    opts.include_untracked(false);

    let diff = repo.diff_tree_to_workdir_with_index(Some(&base_tree), Some(&mut opts))?;

    let mut result: HashMap<String, Vec<(u32, u32)>> = HashMap::new();
    let num_deltas = diff.deltas().len();

    for i in 0..num_deltas {
        let delta = diff
            .get_delta(i)
            .ok_or_else(|| GitError::Custom(format!("invalid delta index: {i}")))?;

        let path = delta
            .new_file()
            .path()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();

        if let Some(patch) = git2::Patch::from_diff(&diff, i)? {
            let mut ranges = Vec::new();
            for h in 0..patch.num_hunks() {
                let (hunk, _) = patch.hunk(h)?;
                ranges.push((hunk.new_start(), hunk.new_lines()));
            }
            if !ranges.is_empty() {
                result.insert(path, ranges);
            }
        }
    }

    Ok(result)
}

/// Check if a 1-based line number falls within any of the given hunk ranges.
/// Each range is (new_start, new_lines) where new_start is 1-based.
pub fn is_line_in_hunk_ranges(line_1based: u32, ranges: &[(u32, u32)]) -> bool {
    ranges
        .iter()
        .any(|&(start, lines)| line_1based >= start && line_1based < start + lines)
}

pub(crate) fn find_base_commit<'a>(
    repo: &'a Repository,
    base_branch: Option<&str>,
) -> Result<(String, git2::Commit<'a>), GitError> {
    let head = repo.head().map_err(|e| {
        if e.code() == git2::ErrorCode::UnbornBranch {
            GitError::Custom("unborn branch: no commits yet".to_string())
        } else {
            GitError::from(e)
        }
    })?;

    let current_oid = head
        .target()
        .ok_or_else(|| GitError::Custom("HEAD has no target".to_string()))?;

    let base_branch_name = if let Some(base) = base_branch {
        Some(base.to_string())
    } else {
        if !head.is_branch() {
            let commit = repo.find_commit(current_oid)?;
            return Ok(("HEAD".to_string(), commit));
        }

        let branch_name = head
            .shorthand()
            .ok_or_else(|| GitError::Custom("HEAD has no shorthand".to_string()))?;

        let config = repo.config().ok();
        super::config::resolve_branch_base(repo, config.as_ref(), branch_name)
    };

    let base_branch_name = match base_branch_name {
        Some(name) => name,
        None => {
            let commit = repo.find_commit(current_oid)?;
            return Ok(("HEAD".to_string(), commit));
        }
    };

    let base_ref = format!("refs/heads/{base_branch_name}");
    let base_oid = match repo.revparse_single(&base_ref) {
        Ok(obj) => obj.peel_to_commit().map(|c| c.id()),
        Err(_) => {
            let remote_ref = format!("refs/remotes/origin/{base_branch_name}");
            repo.revparse_single(&remote_ref)
                .map_err(|_| {
                    GitError::Custom(format!("base branch '{base_branch_name}' not found"))
                })?
                .peel_to_commit()
                .map(|c| c.id())
        }
    }
    .map_err(|e| GitError::Custom(format!("failed to resolve base branch: {e}")))?;

    let merge_base_oid = repo.merge_base(current_oid, base_oid)?;
    let commit = repo.find_commit(merge_base_oid)?;

    Ok((base_branch_name, commit))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::test_helpers::*;
    use git2::build::CheckoutBuilder;

    fn repo_path_str(repo: &Repository) -> String {
        repo.workdir().unwrap().to_str().unwrap().to_string()
    }

    fn setup_feature_branch(repo: &Repository) {
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("feature", &head, false).unwrap();
        repo.set_head("refs/heads/feature").unwrap();
        repo.checkout_head(Some(CheckoutBuilder::new().force()))
            .unwrap();
    }

    #[test]
    fn test_review_diff_modified_file() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "hello.txt", "base content\n", "add hello.txt");

        setup_feature_branch(&repo);
        add_and_commit(&repo, "hello.txt", "modified content\n", "modify hello.txt");

        let paths = vec!["hello.txt".to_string()];
        let result = get_review_diff(&repo_path_str(&repo), None, Some(&paths), None).unwrap();
        assert_eq!(result.changed_files.len(), 1);
        assert_eq!(result.changed_files[0].path, "hello.txt");
        assert_eq!(result.changed_files[0].status, "modified");
        assert!(!result.changed_files[0].binary);
        assert!(!result.changed_files[0].hunks.is_empty());
        assert_eq!(result.stats.files_changed, 1);
    }

    #[test]
    fn test_review_diff_added_file() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "existing.txt", "content\n", "add existing");

        setup_feature_branch(&repo);
        add_and_commit(&repo, "new_file.txt", "new content\n", "add new file");

        let result = get_review_diff(&repo_path_str(&repo), None, None, None).unwrap();
        assert_eq!(result.changed_files.len(), 1);
        assert_eq!(result.changed_files[0].path, "new_file.txt");
        assert_eq!(result.changed_files[0].status, "added");
    }

    #[test]
    fn test_review_diff_deleted_file() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "to_delete.txt", "content\n", "add file");

        setup_feature_branch(&repo);

        // Delete the file and stage the deletion
        let workdir = repo.workdir().unwrap();
        std::fs::remove_file(workdir.join("to_delete.txt")).unwrap();
        let mut index = repo.index().unwrap();
        index
            .remove(std::path::Path::new("to_delete.txt"), 0)
            .unwrap();
        index.write().unwrap();
        let sig = git2::Signature::now("Test User", "test@example.com").unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let parent = repo.head().unwrap().peel_to_commit().unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "delete file", &tree, &[&parent])
            .unwrap();

        let result = get_review_diff(&repo_path_str(&repo), None, None, None).unwrap();
        assert_eq!(result.changed_files.len(), 1);
        assert_eq!(result.changed_files[0].path, "to_delete.txt");
        assert_eq!(result.changed_files[0].status, "deleted");
    }

    #[test]
    fn test_review_diff_with_explicit_base_branch() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        // Get the default branch name
        let default_branch = repo.head().unwrap().shorthand().unwrap().to_string();

        add_and_commit(&repo, "file.txt", "base\n", "add file");

        setup_feature_branch(&repo);
        add_and_commit(&repo, "file.txt", "changed\n", "modify");

        let result =
            get_review_diff(&repo_path_str(&repo), Some(&default_branch), None, None).unwrap();
        assert_eq!(result.changed_files.len(), 1);
        assert_eq!(result.base_ref, default_branch);
    }

    #[test]
    fn test_review_diff_multiple_files() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "a.txt", "aaa\n", "add a");
        add_and_commit(&repo, "b.txt", "bbb\n", "add b");

        setup_feature_branch(&repo);
        add_and_commit(&repo, "a.txt", "modified a\n", "modify a");
        add_and_commit(&repo, "c.txt", "new c\n", "add c");

        let result = get_review_diff(&repo_path_str(&repo), None, None, None).unwrap();
        assert_eq!(result.changed_files.len(), 2);
        assert_eq!(result.stats.files_changed, 2);
    }

    #[test]
    fn test_review_diff_hunk_lines() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "file.txt", "line1\nline2\nline3\n", "add file");

        setup_feature_branch(&repo);
        add_and_commit(
            &repo,
            "file.txt",
            "line1\nmodified\nline3\n",
            "modify line2",
        );

        let paths = vec!["file.txt".to_string()];
        let result = get_review_diff(&repo_path_str(&repo), None, Some(&paths), None).unwrap();
        let file = &result.changed_files[0];
        assert!(!file.hunks.is_empty());

        let hunk = &file.hunks[0];
        assert!(!hunk.lines.is_empty());

        // Verify line origins exist
        let has_addition = hunk.lines.iter().any(|l| l.origin == '+');
        let has_deletion = hunk.lines.iter().any(|l| l.origin == '-');
        assert!(has_addition);
        assert!(has_deletion);
    }

    #[test]
    fn test_review_diff_detached_head() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        let oid = add_and_commit(&repo, "file.txt", "content\n", "add file");
        repo.set_head_detached(oid).unwrap();

        let result = get_review_diff(&repo_path_str(&repo), None, None, None).unwrap();
        assert_eq!(result.base_ref, "HEAD");
        assert!(result.changed_files.is_empty());
    }

    #[test]
    fn test_review_diff_unborn_branch() {
        let (_dir, repo) = create_test_repo();
        let result = get_review_diff(&repo_path_str(&repo), None, None, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unborn branch"));
    }

    // --- New tests for summary/detail modes ---

    #[test]
    fn test_review_diff_summary_has_no_hunks() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "file.txt", "base\n", "add file");

        setup_feature_branch(&repo);
        add_and_commit(&repo, "file.txt", "changed\n", "modify");

        let result = get_review_diff(&repo_path_str(&repo), None, None, None).unwrap();
        assert_eq!(result.changed_files.len(), 1);
        assert!(result.changed_files[0].hunks.is_empty());
    }

    #[test]
    fn test_review_diff_summary_has_file_stats() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "file.txt", "line1\nline2\n", "add file");

        setup_feature_branch(&repo);
        add_and_commit(&repo, "file.txt", "line1\nmodified\nnew_line\n", "modify");

        let result = get_review_diff(&repo_path_str(&repo), None, None, None).unwrap();
        let file = &result.changed_files[0];
        assert_eq!(file.stats.additions, 2);
        assert_eq!(file.stats.deletions, 1);
    }

    #[test]
    fn test_review_diff_detail_filters_by_path() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "a.txt", "aaa\n", "add a");
        add_and_commit(&repo, "b.txt", "bbb\n", "add b");

        setup_feature_branch(&repo);
        add_and_commit(&repo, "a.txt", "modified a\n", "modify a");
        add_and_commit(&repo, "b.txt", "modified b\n", "modify b");

        let paths = vec!["a.txt".to_string()];
        let result = get_review_diff(&repo_path_str(&repo), None, Some(&paths), None).unwrap();
        assert_eq!(result.changed_files.len(), 1);
        assert_eq!(result.changed_files[0].path, "a.txt");
        // stats still reflects the full diff
        assert_eq!(result.stats.files_changed, 2);
    }

    #[test]
    fn test_review_diff_detail_includes_hunks() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "file.txt", "base\n", "add file");

        setup_feature_branch(&repo);
        add_and_commit(&repo, "file.txt", "changed\n", "modify");

        let paths = vec!["file.txt".to_string()];
        let result = get_review_diff(&repo_path_str(&repo), None, Some(&paths), None).unwrap();
        assert_eq!(result.changed_files.len(), 1);
        assert!(!result.changed_files[0].hunks.is_empty());
    }

    #[test]
    fn test_review_diff_detail_nonexistent_path() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "file.txt", "base\n", "add file");

        setup_feature_branch(&repo);
        add_and_commit(&repo, "file.txt", "changed\n", "modify");

        let paths = vec!["nonexistent.txt".to_string()];
        let result = get_review_diff(&repo_path_str(&repo), None, Some(&paths), None).unwrap();
        assert!(result.changed_files.is_empty());
    }

    // --- Tests for get_hunk_ranges ---

    #[test]
    fn test_get_hunk_ranges_single_file() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(
            &repo,
            "file.txt",
            "line1\nline2\nline3\nline4\nline5\n",
            "add file",
        );

        setup_feature_branch(&repo);
        add_and_commit(
            &repo,
            "file.txt",
            "line1\nmodified\nline3\nline4\nline5\n",
            "modify line2",
        );

        let ranges = get_hunk_ranges(&repo_path_str(&repo), None).unwrap();
        assert!(ranges.contains_key("file.txt"));
        let file_ranges = &ranges["file.txt"];
        assert!(!file_ranges.is_empty());
        // The hunk should cover line 2 area
        let (start, _lines) = file_ranges[0];
        assert!(start >= 1);
    }

    #[test]
    fn test_get_hunk_ranges_multiple_files() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "a.txt", "aaa\n", "add a");
        add_and_commit(&repo, "b.txt", "bbb\n", "add b");

        setup_feature_branch(&repo);
        add_and_commit(&repo, "a.txt", "modified a\n", "modify a");
        add_and_commit(&repo, "b.txt", "modified b\n", "modify b");

        let ranges = get_hunk_ranges(&repo_path_str(&repo), None).unwrap();
        assert!(ranges.contains_key("a.txt"));
        assert!(ranges.contains_key("b.txt"));
    }

    #[test]
    fn test_get_hunk_ranges_no_changes() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "file.txt", "content\n", "add file");

        setup_feature_branch(&repo);
        // No changes on feature branch
        let ranges = get_hunk_ranges(&repo_path_str(&repo), None).unwrap();
        assert!(ranges.is_empty());
    }

    // --- Tests for is_line_in_hunk_ranges ---

    #[test]
    fn test_is_line_in_hunk_ranges_inside() {
        let ranges = vec![(10, 5)]; // lines 10..14
        assert!(is_line_in_hunk_ranges(10, &ranges));
        assert!(is_line_in_hunk_ranges(14, &ranges));
    }

    #[test]
    fn test_is_line_in_hunk_ranges_outside() {
        let ranges = vec![(10, 5)]; // lines 10..14
        assert!(!is_line_in_hunk_ranges(9, &ranges));
        assert!(!is_line_in_hunk_ranges(15, &ranges));
    }

    #[test]
    fn test_is_line_in_hunk_ranges_multiple_ranges() {
        let ranges = vec![(5, 3), (20, 2)]; // lines 5..7 and 20..21
        assert!(is_line_in_hunk_ranges(5, &ranges));
        assert!(is_line_in_hunk_ranges(7, &ranges));
        assert!(!is_line_in_hunk_ranges(8, &ranges));
        assert!(is_line_in_hunk_ranges(20, &ranges));
        assert!(is_line_in_hunk_ranges(21, &ranges));
        assert!(!is_line_in_hunk_ranges(22, &ranges));
    }

    #[test]
    fn test_is_line_in_hunk_ranges_empty() {
        assert!(!is_line_in_hunk_ranges(1, &[]));
    }

    #[test]
    fn test_is_line_in_hunk_ranges_single_line_hunk() {
        let ranges = vec![(5, 1)]; // only line 5
        assert!(!is_line_in_hunk_ranges(4, &ranges));
        assert!(is_line_in_hunk_ranges(5, &ranges));
        assert!(!is_line_in_hunk_ranges(6, &ranges));
    }

    // --- Tests for max_lines_per_file truncation ---

    #[test]
    fn test_review_diff_truncates_large_file() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "file.txt", "line1\nline2\nline3\n", "add file");

        setup_feature_branch(&repo);
        add_and_commit(
            &repo,
            "file.txt",
            "changed1\nchanged2\nchanged3\n",
            "modify all lines",
        );

        // Set max_lines_per_file=1 so the file (3 additions + 3 deletions = 6) exceeds it
        let paths = vec!["file.txt".to_string()];
        let result = get_review_diff(&repo_path_str(&repo), None, Some(&paths), Some(1)).unwrap();
        assert_eq!(result.changed_files.len(), 1);
        let file = &result.changed_files[0];
        assert!(file.truncated);
        // Hunk headers are preserved (line ranges), but lines are empty
        assert!(!file.hunks.is_empty());
        assert!(file.hunks[0].lines.is_empty());
        assert!(file.hunks[0].new_start > 0);
        // stats should still be present
        assert!(file.stats.additions > 0 || file.stats.deletions > 0);
    }

    #[test]
    fn test_review_diff_no_truncation_when_under_limit() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "file.txt", "line1\nline2\n", "add file");

        setup_feature_branch(&repo);
        add_and_commit(&repo, "file.txt", "line1\nchanged\n", "modify line2");

        // 1 addition + 1 deletion = 2, limit is 100
        let paths = vec!["file.txt".to_string()];
        let result = get_review_diff(&repo_path_str(&repo), None, Some(&paths), Some(100)).unwrap();
        assert_eq!(result.changed_files.len(), 1);
        let file = &result.changed_files[0];
        assert!(!file.truncated);
        assert!(!file.hunks.is_empty());
    }

    #[test]
    fn test_review_diff_no_truncation_when_none() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "file.txt", "line1\nline2\nline3\n", "add file");

        setup_feature_branch(&repo);
        add_and_commit(
            &repo,
            "file.txt",
            "changed1\nchanged2\nchanged3\n",
            "modify all lines",
        );

        // max_lines_per_file=None means no limit
        let paths = vec!["file.txt".to_string()];
        let result = get_review_diff(&repo_path_str(&repo), None, Some(&paths), None).unwrap();
        assert_eq!(result.changed_files.len(), 1);
        let file = &result.changed_files[0];
        assert!(!file.truncated);
        assert!(!file.hunks.is_empty());
    }

    #[test]
    fn test_review_diff_summary_mode_not_truncated() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "file.txt", "line1\n", "add file");

        setup_feature_branch(&repo);
        add_and_commit(&repo, "file.txt", "changed\n", "modify");

        // Summary mode (paths=None) should never truncate
        let result = get_review_diff(&repo_path_str(&repo), None, None, Some(1)).unwrap();
        assert_eq!(result.changed_files.len(), 1);
        assert!(!result.changed_files[0].truncated);
    }
}
