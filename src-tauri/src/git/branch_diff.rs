use super::error::GitError;
use git2::Repository;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffStats {
    pub additions: u32,
    pub deletions: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangedFile {
    pub path: String,
    pub old_path: Option<String>,
    pub status: String,
    pub binary: bool,
    pub stats: DiffStats,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchDiffSummary {
    pub base_branch: String,
    pub changed_files: Vec<ChangedFile>,
    pub stats: DiffStats,
}

/// Returns a summary of files changed between the merge-base of the current branch
/// and the working tree (including staged changes, including untracked files).
/// Used by the Source Control panel to show a flat file list in "branch-base" diff mode.
///
/// For an unborn branch (initial commit not yet created), returns an empty summary
/// so callers can display a consistent empty state instead of propagating an error.
pub fn get_branch_diff_summary(
    repo_path: &str,
    base_branch: Option<&str>,
) -> Result<BranchDiffSummary, GitError> {
    let repo = Repository::open(repo_path)?;
    if is_unborn_branch(&repo)? {
        return Ok(BranchDiffSummary {
            base_branch: String::new(),
            changed_files: Vec::new(),
            stats: DiffStats {
                additions: 0,
                deletions: 0,
            },
        });
    }
    let (base_ref, base_commit) = find_base_commit(&repo, base_branch)?;
    let base_tree = base_commit.tree()?;

    let mut opts = git2::DiffOptions::new();
    opts.include_untracked(true);
    opts.recurse_untracked_dirs(true);
    opts.show_untracked_content(true);

    let diff = repo.diff_tree_to_workdir_with_index(Some(&base_tree), Some(&mut opts))?;
    let mut find_opts = git2::DiffFindOptions::new();
    find_opts.renames(true).copies(true);
    let mut diff = diff;
    diff.find_similar(Some(&mut find_opts))?;

    let stats = diff.stats()?;
    let total_additions = stats.insertions() as u32;
    let total_deletions = stats.deletions() as u32;

    let num_deltas = diff.deltas().len();
    let mut changed_files: Vec<ChangedFile> = Vec::with_capacity(num_deltas);

    for i in 0..num_deltas {
        let delta = diff
            .get_delta(i)
            .ok_or_else(|| GitError::Custom(format!("invalid delta index: {i}")))?;

        let new_path = delta
            .new_file()
            .path()
            .map(|p| p.to_string_lossy().to_string());
        let old_path = delta
            .old_file()
            .path()
            .map(|p| p.to_string_lossy().to_string());
        let path = new_path
            .clone()
            .or_else(|| old_path.clone())
            .ok_or_else(|| GitError::Custom(format!("delta {i} has no file path")))?;

        let status = match delta.status() {
            git2::Delta::Added | git2::Delta::Untracked => "added",
            git2::Delta::Deleted => "deleted",
            git2::Delta::Modified => "modified",
            git2::Delta::Renamed => "renamed",
            git2::Delta::Copied => "copied",
            git2::Delta::Typechange => "modified",
            _ => "modified",
        };

        let binary = delta.new_file().is_binary() || delta.old_file().is_binary();

        let (additions, deletions) = if binary {
            (0u32, 0u32)
        } else if let Some(patch) = git2::Patch::from_diff(&diff, i)? {
            let mut adds = 0u32;
            let mut dels = 0u32;
            for h in 0..patch.num_hunks() {
                let lines = patch.num_lines_in_hunk(h)?;
                for l in 0..lines {
                    let line = patch.line_in_hunk(h, l)?;
                    match line.origin() {
                        '+' => adds += 1,
                        '-' => dels += 1,
                        _ => {}
                    }
                }
            }
            (adds, dels)
        } else {
            (0u32, 0u32)
        };

        let old_path_opt = match status {
            "renamed" | "copied" => old_path,
            _ => None,
        };

        changed_files.push(ChangedFile {
            path,
            old_path: old_path_opt,
            status: status.to_string(),
            binary,
            stats: DiffStats {
                additions,
                deletions,
            },
        });
    }

    Ok(BranchDiffSummary {
        base_branch: base_ref,
        changed_files,
        stats: DiffStats {
            additions: total_additions,
            deletions: total_deletions,
        },
    })
}

/// Returns true when the repository has no commits yet (HEAD points to an unborn branch).
fn is_unborn_branch(repo: &Repository) -> Result<bool, GitError> {
    match repo.head() {
        Ok(_) => Ok(false),
        Err(e) if e.code() == git2::ErrorCode::UnbornBranch => Ok(true),
        Err(e) => Err(GitError::from(e)),
    }
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

        let branch_name = head.shorthand()?;

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

    // --- Tests for get_branch_diff_summary ---

    #[test]
    fn test_get_branch_diff_summary_single_modified_file() {
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

        let summary = get_branch_diff_summary(&repo_path_str(&repo), None).unwrap();
        assert_eq!(summary.changed_files.len(), 1);
        assert_eq!(summary.changed_files[0].path, "file.txt");
        assert_eq!(summary.changed_files[0].status, "modified");
        assert!(!summary.changed_files[0].binary);
    }

    #[test]
    fn test_get_branch_diff_summary_added_file() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "existing.txt", "content\n", "add existing");

        setup_feature_branch(&repo);
        add_and_commit(&repo, "new.txt", "new\n", "add new file");

        let summary = get_branch_diff_summary(&repo_path_str(&repo), None).unwrap();
        let added = summary
            .changed_files
            .iter()
            .find(|f| f.path == "new.txt")
            .expect("new.txt should be in diff");
        assert_eq!(added.status, "added");
    }

    #[test]
    fn test_get_branch_diff_summary_no_changes() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "file.txt", "content\n", "add file");

        setup_feature_branch(&repo);
        let summary = get_branch_diff_summary(&repo_path_str(&repo), None).unwrap();
        assert!(summary.changed_files.is_empty());
        assert_eq!(summary.stats.additions, 0);
        assert_eq!(summary.stats.deletions, 0);
    }

    #[test]
    fn test_get_branch_diff_summary_includes_untracked_file() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "existing.txt", "content\n", "add existing");

        setup_feature_branch(&repo);
        // Create an untracked file (no index add, no commit)
        let workdir = repo.workdir().unwrap();
        std::fs::write(workdir.join("untracked.txt"), "hello\n").unwrap();

        let summary = get_branch_diff_summary(&repo_path_str(&repo), None).unwrap();
        let untracked = summary
            .changed_files
            .iter()
            .find(|f| f.path == "untracked.txt")
            .expect("untracked.txt should be included in the branch diff");
        assert_eq!(untracked.status, "added");
    }

    #[test]
    fn test_get_branch_diff_summary_unborn_branch_returns_empty() {
        let (_dir, repo) = create_test_repo();
        // No initial commit: HEAD points to an unborn branch
        let summary = get_branch_diff_summary(&repo_path_str(&repo), None).unwrap();
        assert!(summary.changed_files.is_empty());
        assert_eq!(summary.stats.additions, 0);
        assert_eq!(summary.stats.deletions, 0);
        assert_eq!(summary.base_branch, "");
    }
}
