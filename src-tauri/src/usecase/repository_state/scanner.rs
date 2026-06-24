use std::collections::HashMap;
use std::sync::Arc;

use crate::domain::code::DiffFileEntry;
use crate::usecase::code_usecase::CodeUsecase;
use crate::usecase::repository_dto::{FileDiffStatDto, FileStatusDto};
use crate::usecase::repository_usecase::RepositoryUsecase;

use super::error::RepositoryStateError;
use super::snapshot::RepositorySnapshotParts;

pub trait RepositoryScanner: Send + Sync {
    fn scan(&self, repo_path: &str) -> Result<RepositorySnapshotParts, RepositoryStateError>;

    fn status_with_ignored(
        &self,
        repo_path: &str,
    ) -> Result<Vec<FileStatusDto>, RepositoryStateError>;

    fn prune_stale_branch_bases(
        &self,
        repo_path: &str,
        existing_branches: &[String],
    ) -> Result<(), RepositoryStateError>;
}

pub struct DefaultRepositoryScanner {
    repository: Arc<RepositoryUsecase>,
    code: Arc<CodeUsecase>,
}

impl DefaultRepositoryScanner {
    pub fn new(repository: Arc<RepositoryUsecase>, code: Arc<CodeUsecase>) -> Self {
        Self { repository, code }
    }
}

impl RepositoryScanner for DefaultRepositoryScanner {
    fn scan(&self, repo_path: &str) -> Result<RepositorySnapshotParts, RepositoryStateError> {
        let status_scan = self.repository.get_repository_status_scan(repo_path)?;
        let current_dirty_count = status_scan.dirty_count;
        let status: Vec<FileStatusDto> = status_scan.status.into_iter().map(Into::into).collect();
        let diff_stats: Vec<FileDiffStatDto> =
            status_scan.diff_stats.into_iter().map(Into::into).collect();
        let branch_cards = self
            .repository
            .list_branches_with_status_for_scan(repo_path, current_dirty_count)?;
        let diff_file_tree = self
            .code
            .build_diff_file_tree(diff_tree_entries(&status, &diff_stats));
        let staged_diff_file_tree = self
            .code
            .build_diff_file_tree(staged_diff_tree_entries(&status, &diff_stats));
        let changes_diff_file_tree = self
            .code
            .build_diff_file_tree(changes_diff_tree_entries(&status, &diff_stats));

        Ok(RepositorySnapshotParts {
            status,
            diff_stats,
            branch_cards,
            diff_file_tree,
            staged_diff_file_tree,
            changes_diff_file_tree,
            // Thresholds are defined by the later review snapshot work. This issue
            // only carries the flag through the snapshot contract.
            limited: false,
        })
    }

    fn status_with_ignored(
        &self,
        repo_path: &str,
    ) -> Result<Vec<FileStatusDto>, RepositoryStateError> {
        Ok(self
            .repository
            .get_git_status_include_ignored(repo_path)?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    fn prune_stale_branch_bases(
        &self,
        repo_path: &str,
        existing_branches: &[String],
    ) -> Result<(), RepositoryStateError> {
        Ok(self
            .repository
            .prune_stale_branch_bases(repo_path, existing_branches)?)
    }
}

fn stats_by_path(diff_stats: &[FileDiffStatDto]) -> HashMap<&str, &FileDiffStatDto> {
    diff_stats
        .iter()
        .map(|stat| (stat.path.as_str(), stat))
        .collect()
}

fn diff_tree_entries(
    status: &[FileStatusDto],
    diff_stats: &[FileDiffStatDto],
) -> Vec<DiffFileEntry> {
    let stats_by_path = stats_by_path(diff_stats);

    status
        .iter()
        .filter(|entry| entry.worktree_status != "ignored")
        .map(|entry| {
            let stat = stats_by_path.get(entry.path.as_str()).copied();
            DiffFileEntry {
                path: entry.path.clone(),
                status: diff_tree_status(entry).to_string(),
                additions: stat
                    .map(|stat| stat.index_additions + stat.wt_additions)
                    .unwrap_or(0),
                deletions: stat
                    .map(|stat| stat.index_deletions + stat.wt_deletions)
                    .unwrap_or(0),
            }
        })
        .collect()
}

fn staged_diff_tree_entries(
    status: &[FileStatusDto],
    diff_stats: &[FileDiffStatDto],
) -> Vec<DiffFileEntry> {
    let stats_by_path = stats_by_path(diff_stats);

    status
        .iter()
        .filter(|entry| entry.index_status != "none")
        .map(|entry| {
            let stat = stats_by_path.get(entry.path.as_str()).copied();
            DiffFileEntry {
                path: entry.path.clone(),
                status: entry.index_status.clone(),
                additions: stat.map(|stat| stat.index_additions).unwrap_or(0),
                deletions: stat.map(|stat| stat.index_deletions).unwrap_or(0),
            }
        })
        .collect()
}

fn changes_diff_tree_entries(
    status: &[FileStatusDto],
    diff_stats: &[FileDiffStatDto],
) -> Vec<DiffFileEntry> {
    let stats_by_path = stats_by_path(diff_stats);

    status
        .iter()
        .filter(|entry| entry.worktree_status != "none" && entry.worktree_status != "ignored")
        .map(|entry| {
            let stat = stats_by_path.get(entry.path.as_str()).copied();
            DiffFileEntry {
                path: entry.path.clone(),
                status: entry.worktree_status.clone(),
                additions: stat.map(|stat| stat.wt_additions).unwrap_or(0),
                deletions: stat.map(|stat| stat.wt_deletions).unwrap_or(0),
            }
        })
        .collect()
}

fn diff_tree_status(entry: &FileStatusDto) -> &str {
    if entry.index_status != "none" {
        entry.index_status.as_str()
    } else {
        entry.worktree_status.as_str()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_tree_entries_use_index_status_before_worktree_status() {
        let status = vec![FileStatusDto {
            path: "src/lib.rs".to_string(),
            index_status: "modified".to_string(),
            worktree_status: "deleted".to_string(),
        }];
        let stats = vec![FileDiffStatDto {
            path: "src/lib.rs".to_string(),
            index_additions: 2,
            index_deletions: 1,
            wt_additions: 3,
            wt_deletions: 4,
        }];

        let entries = diff_tree_entries(&status, &stats);

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, "modified");
        assert_eq!(entries[0].additions, 5);
        assert_eq!(entries[0].deletions, 5);
    }

    #[test]
    fn diff_tree_entries_skip_ignored_status() {
        let status = vec![FileStatusDto {
            path: "target".to_string(),
            index_status: "none".to_string(),
            worktree_status: "ignored".to_string(),
        }];

        assert!(diff_tree_entries(&status, &[]).is_empty());
    }

    #[test]
    fn staged_and_changes_entries_are_split_from_same_status_and_stats() {
        let status = vec![
            FileStatusDto {
                path: "both.rs".to_string(),
                index_status: "modified".to_string(),
                worktree_status: "deleted".to_string(),
            },
            FileStatusDto {
                path: "ignored".to_string(),
                index_status: "none".to_string(),
                worktree_status: "ignored".to_string(),
            },
        ];
        let stats = vec![FileDiffStatDto {
            path: "both.rs".to_string(),
            index_additions: 2,
            index_deletions: 1,
            wt_additions: 0,
            wt_deletions: 4,
        }];

        let staged = staged_diff_tree_entries(&status, &stats);
        let changes = changes_diff_tree_entries(&status, &stats);

        assert_eq!(staged.len(), 1);
        assert_eq!(staged[0].status, "modified");
        assert_eq!(staged[0].additions, 2);
        assert_eq!(staged[0].deletions, 1);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].status, "deleted");
        assert_eq!(changes[0].additions, 0);
        assert_eq!(changes[0].deletions, 4);
    }

    #[test]
    fn default_scanner_matches_existing_usecase_read_models_for_real_repo() {
        let (dir, repo) = crate::git::test_helpers::create_test_repo();
        crate::git::test_helpers::create_initial_commit(&repo);
        crate::git::test_helpers::add_and_commit(&repo, "staged.txt", "before\n", "add staged");
        crate::git::test_helpers::add_and_commit(&repo, "unstaged.txt", "before\n", "add unstaged");
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("feature", &head, false).unwrap();

        std::fs::write(dir.path().join("staged.txt"), "before\nafter\n").unwrap();
        {
            let mut index = repo.index().unwrap();
            index.add_path(std::path::Path::new("staged.txt")).unwrap();
            index.write().unwrap();
        }
        std::fs::write(dir.path().join("unstaged.txt"), "changed\n").unwrap();
        std::fs::write(dir.path().join("untracked.txt"), "new\n").unwrap();

        let repository = Arc::new(crate::adaptor::controller::wiring::build_repository_usecase());
        let code = Arc::new(crate::adaptor::controller::wiring::build_code_usecase());
        let scanner = DefaultRepositoryScanner::new(repository.clone(), code.clone());
        let repo_path = dir.path().to_str().unwrap();

        crate::adaptor::gateway::repository::status::reset_status_walk_count_for_tests();
        let snapshot = scanner.scan(repo_path).unwrap();
        assert_eq!(
            crate::adaptor::gateway::repository::status::status_walk_count_for_tests(),
            1
        );

        let expected_status: Vec<FileStatusDto> =
            crate::adaptor::gateway::repository::status::get_git_status(repo_path)
                .unwrap()
                .into_iter()
                .map(Into::into)
                .collect();
        let expected_diff_stats: Vec<FileDiffStatDto> =
            crate::adaptor::gateway::repository::status::get_status_diff_stats(repo_path)
                .unwrap()
                .into_iter()
                .map(Into::into)
                .collect();
        let expected_branch_cards = repository
            .list_branches_with_status_read_only(repo_path)
            .unwrap();

        assert_eq!(snapshot.status, expected_status);
        assert_eq!(snapshot.diff_stats, expected_diff_stats);
        assert_eq!(
            serde_json::to_value(&snapshot.branch_cards).unwrap(),
            serde_json::to_value(&expected_branch_cards).unwrap()
        );
        assert_eq!(
            serde_json::to_value(&snapshot.diff_file_tree).unwrap(),
            serde_json::to_value(
                code.build_diff_file_tree(diff_tree_entries(
                    &expected_status,
                    &expected_diff_stats,
                ))
            )
            .unwrap()
        );
        assert_eq!(
            serde_json::to_value(&snapshot.staged_diff_file_tree).unwrap(),
            serde_json::to_value(code.build_diff_file_tree(staged_diff_tree_entries(
                &expected_status,
                &expected_diff_stats,
            )))
            .unwrap()
        );
        assert_eq!(
            serde_json::to_value(&snapshot.changes_diff_file_tree).unwrap(),
            serde_json::to_value(code.build_diff_file_tree(changes_diff_tree_entries(
                &expected_status,
                &expected_diff_stats,
            )))
            .unwrap()
        );
    }
}
