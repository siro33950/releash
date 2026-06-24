//! repository ドメインの読み取りクエリサービス（read model 生成）。
//!
//! **QueryService は Usecase ではない。** 表示・転送向けの read model（DTO）を
//! データソースから直接構築する Query 専用の協力者であり、業務手順（オーケストレーション）
//! は持たない。Entity をそのまま返す単純な読み取りは Repository の責務であり、ここには置かない。
//!
//! git2 のブロッキング呼び出しは gateway 内で同期的に行われ、非同期境界は controller 層で被せる。

use std::sync::Arc;

use crate::domain::repository::RepositoryError;

use super::repository_dto::BranchCardDto;
use super::repository_error::UsecaseError;

/// ブランチカード一覧の読み取りポート（Query 側）。
///
/// gateway 実装がデータソース（git2）から read model（`BranchCardDto`）を
/// 中間 Entity を介さず直接組み立てる。
pub trait BranchCardQuery: Send + Sync {
    fn list_branch_cards(&self, repo_path: &str) -> Result<Vec<BranchCardDto>, RepositoryError>;

    fn list_branch_cards_for_scan(
        &self,
        repo_path: &str,
        current_dirty_count: usize,
    ) -> Result<Vec<BranchCardDto>, RepositoryError> {
        let _ = current_dirty_count;
        self.list_branch_cards(repo_path)
    }
}

/// read model（`BranchCardDto`）を構築する読み取りクエリサービス。
/// Usecase から呼ばれる協力者であり、entity 用 Repository には依存しない。
#[derive(Clone)]
pub struct RepositoryQueryService {
    branch_card_query: Arc<dyn BranchCardQuery>,
}

impl RepositoryQueryService {
    pub fn new(branch_card_query: Arc<dyn BranchCardQuery>) -> Self {
        Self { branch_card_query }
    }

    #[cfg(test)]
    pub fn list_branches_with_status(
        &self,
        repo_path: &str,
    ) -> Result<Vec<BranchCardDto>, UsecaseError> {
        let mut cards = self.branch_card_query.list_branch_cards(repo_path)?;
        cards.sort_by_key(|card| !card.is_main_worktree);
        Ok(cards)
    }

    pub fn list_branches_with_status_for_scan(
        &self,
        repo_path: &str,
        current_dirty_count: usize,
    ) -> Result<Vec<BranchCardDto>, UsecaseError> {
        let mut cards = self
            .branch_card_query
            .list_branch_cards_for_scan(repo_path, current_dirty_count)?;
        cards.sort_by_key(|card| !card.is_main_worktree);
        Ok(cards)
    }
}

#[cfg(test)]
mod repository_query_service_tests {
    use super::*;

    struct FakeBranchCards;

    impl BranchCardQuery for FakeBranchCards {
        fn list_branch_cards(
            &self,
            _repo_path: &str,
        ) -> Result<Vec<BranchCardDto>, RepositoryError> {
            Ok(vec![BranchCardDto {
                name: "main".to_string(),
                is_main_worktree: true,
                worktree_path: Some("/repo".to_string()),
                dirty_count: 0,
                is_merged: false,
                ahead: 0,
                behind: 0,
                has_upstream: false,
                base_ahead: 0,
            }])
        }
    }

    #[test]
    fn test_ブランチカード一覧を委譲する() {
        let service = RepositoryQueryService::new(Arc::new(FakeBranchCards));
        let cards = service.list_branches_with_status("/repo").unwrap();
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].name, "main");
        assert!(cards[0].is_main_worktree);
    }

    struct FakeUnsortedBranchCards;

    impl BranchCardQuery for FakeUnsortedBranchCards {
        fn list_branch_cards(
            &self,
            _repo_path: &str,
        ) -> Result<Vec<BranchCardDto>, RepositoryError> {
            Ok(vec![
                BranchCardDto {
                    name: "feature-a".to_string(),
                    is_main_worktree: false,
                    worktree_path: Some("/repo-worktrees/feature-a".to_string()),
                    dirty_count: 0,
                    is_merged: false,
                    ahead: 0,
                    behind: 0,
                    has_upstream: false,
                    base_ahead: 0,
                },
                BranchCardDto {
                    name: "main".to_string(),
                    is_main_worktree: true,
                    worktree_path: Some("/repo".to_string()),
                    dirty_count: 0,
                    is_merged: false,
                    ahead: 0,
                    behind: 0,
                    has_upstream: false,
                    base_ahead: 0,
                },
                BranchCardDto {
                    name: "feature-b".to_string(),
                    is_main_worktree: false,
                    worktree_path: Some("/repo-worktrees/feature-b".to_string()),
                    dirty_count: 0,
                    is_merged: false,
                    ahead: 0,
                    behind: 0,
                    has_upstream: false,
                    base_ahead: 0,
                },
            ])
        }
    }

    #[test]
    fn test_main_worktreeを先頭に正規化する() {
        let service = RepositoryQueryService::new(Arc::new(FakeUnsortedBranchCards));
        let cards = service.list_branches_with_status("/repo").unwrap();
        let names: Vec<&str> = cards.iter().map(|card| card.name.as_str()).collect();
        assert_eq!(names, vec!["main", "feature-a", "feature-b"]);
    }
}
