use std::sync::Arc;

use crate::usecase::code_usecase::CodeUsecase;
use crate::usecase::repository_dto::{FileDiffStatDto, FileStatusDto};
use crate::usecase::repository_state::scanner::{
    changes_diff_tree_entries, diff_tree_entries, staged_diff_tree_entries, RepositoryScanner,
};
use crate::usecase::repository_state::snapshot::RepositorySnapshotParts;
use crate::usecase::repository_state::RepositoryStateError;
use crate::usecase::repository_usecase::RepositoryUsecase;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_scanner_matches_existing_usecase_read_models_for_real_repo() {
        let (dir, repo) = crate::test_support::git::create_test_repo();
        crate::test_support::git::create_initial_commit(&repo);
        crate::test_support::git::add_and_commit(&repo, "staged.txt", "before\n", "add staged");
        crate::test_support::git::add_and_commit(&repo, "unstaged.txt", "before\n", "add unstaged");
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

    #[tokio::test]
    async fn default_scanner_prunes_stale_branch_bases_after_committed_cold_start_scan() {
        let (dir, repo) = crate::test_support::git::create_test_repo();
        crate::test_support::git::create_initial_commit(&repo);
        let branch_name = repo.head().unwrap().shorthand().unwrap().to_string();
        {
            let mut config = repo.config().unwrap();
            config
                .set_str(&format!("branch.{branch_name}.releash-base"), "main")
                .unwrap();
            config
                .set_str("branch.deleted.releash-base", "main")
                .unwrap();
        }

        let repository = Arc::new(crate::adaptor::controller::wiring::build_repository_usecase());
        let code = Arc::new(crate::adaptor::controller::wiring::build_code_usecase());
        let scanner = Arc::new(DefaultRepositoryScanner::new(repository, code));
        let state = crate::usecase::repository_state::worktree::WorktreeState::new(
            dir.path().to_str().unwrap().to_string(),
            scanner,
            Arc::new(crate::usecase::repository_state::worktree::NoopRepositoryStateNotifier),
            Arc::new(
                crate::usecase::repository_state::runtime::tests_support::TestRepositoryStateWorkerRuntime,
            ),
            std::time::Duration::ZERO,
        );

        state.invalidate(crate::usecase::repository_state::worker::InvalidateReason::initial());
        let mut ready = false;
        for _ in 0..100 {
            let snapshot = state.snapshot_for_read();
            if snapshot.version >= 1 && !snapshot.flags.loading {
                ready = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(ready, "timed out waiting for initial repository scan");

        let config = repo.config().unwrap();
        assert_eq!(
            config
                .get_string(&format!("branch.{branch_name}.releash-base"))
                .unwrap(),
            "main"
        );
        assert!(config.get_string("branch.deleted.releash-base").is_err());
    }
}
