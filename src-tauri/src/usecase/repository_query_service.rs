//! repository ドメインの読み取りクエリサービス（read model 生成）。
//!
//! **QueryService は Usecase ではない。** 表示・転送向けの read model（DTO）を
//! データソースから直接構築する Query 専用の協力者であり、業務手順（オーケストレーション）
//! は持たない。Entity をそのまま返す単純な読み取りは Repository の責務であり、ここには置かない。
//!
//! git2 のブロッキング呼び出しは gateway 内で同期的に行われ、非同期境界は controller 層で被せる。

use crate::domain::repository::RepositoryError;
use crate::domain::workflow::WorktreeInventoryEntry;
use std::sync::Arc;

use super::repository_dto::{BranchCardDto, WorktreeDisplayGroupsDto, WorktreeEntryDto};
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

pub fn classify_worktree_entries(repository_root: &str, entries: &mut Vec<WorktreeEntryDto>) {
    entries.retain(|entry| {
        !WorktreeInventoryEntry::new(repository_root, &entry.path, &entry.branch)
            .matches_isolated_identity_rule()
    });
}

pub fn classify_branch_cards(
    repository_root: &str,
    cards: &mut Vec<BranchCardDto>,
) -> WorktreeDisplayGroupsDto {
    cards.retain(|card| {
        !card.worktree_path.as_ref().is_some_and(|path| {
            WorktreeInventoryEntry::new(repository_root, path, &card.name)
                .matches_isolated_identity_rule()
        })
    });
    WorktreeDisplayGroupsDto {
        working_areas: cards
            .iter()
            .filter(|card| card.worktree_path.is_some())
            .cloned()
            .collect(),
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
        repository_root: &str,
    ) -> Result<Vec<BranchCardDto>, UsecaseError> {
        let mut cards = self.branch_card_query.list_branch_cards(repo_path)?;
        cards.sort_by_key(|card| !card.is_main_worktree);
        let _ = classify_branch_cards(repository_root, &mut cards);
        Ok(cards)
    }

    pub fn list_branches_with_status_for_scan(
        &self,
        repo_path: &str,
        repository_root: &str,
        current_dirty_count: usize,
    ) -> Result<Vec<BranchCardDto>, UsecaseError> {
        let mut cards = self
            .branch_card_query
            .list_branch_cards_for_scan(repo_path, current_dirty_count)?;
        cards.sort_by_key(|card| !card.is_main_worktree);
        let _ = classify_branch_cards(repository_root, &mut cards);
        Ok(cards)
    }

    pub fn classify_worktree_entries(
        &self,
        repository_root: &str,
        entries: &mut Vec<WorktreeEntryDto>,
    ) {
        classify_worktree_entries(repository_root, entries);
    }
}

#[cfg(test)]
mod repository_query_service_tests {
    use super::*;
    fn card(name: &str, path: Option<&str>) -> BranchCardDto {
        BranchCardDto {
            name: name.to_string(),
            is_main_worktree: name == "main",
            worktree_path: path.map(str::to_string),
            dirty_count: 0,
            is_merged: false,
            ahead: 0,
            behind: 0,
            has_upstream: false,
            base_ahead: 0,
        }
    }

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
        let cards = service.list_branches_with_status("/repo", "/repo").unwrap();
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
        let cards = service.list_branches_with_status("/repo", "/repo").unwrap();
        let names: Vec<&str> = cards.iter().map(|card| card.name.as_str()).collect();
        assert_eq!(names, vec!["main", "feature-a", "feature-b"]);
    }

    #[test]
    fn test_隔離worktree一覧_命名が一致する実体だけを常に非表示にする() {
        // Given
        let mut cards = vec![
            card("main", Some("/repo")),
            card(
                "releash/isolated/node-a1",
                Some("/repo-worktrees/.releash-isolated/node-a1"),
            ),
            card(
                "releash/isolated/other-a1",
                Some("/repo-worktrees/other-a1"),
            ),
        ];
        // When
        let groups = classify_branch_cards("/repo", &mut cards);
        // Then
        assert_eq!(cards.len(), 2);
        assert_eq!(groups.working_areas.len(), 2);
        assert_eq!(cards[1].name, "releash/isolated/other-a1");
    }

    #[test]
    fn test_ブランチ分類_checkoutされていない通常branchも一覧に保持する() {
        // Given
        let mut cards = vec![
            card("feature", None),
            card("main", Some("/repo")),
            card("releash/isolated/retained-a1", None),
            card(
                "releash/isolated/mismatch-a1",
                Some("/repo-worktrees/mismatch"),
            ),
            card(
                "releash/isolated/hidden-a1",
                Some("/repo-worktrees/.releash-isolated/hidden-a1"),
            ),
        ];
        let expected = cards[..4].to_vec();
        // When
        let groups = classify_branch_cards("/repo", &mut cards);
        // Then
        assert_eq!(
            serde_json::to_value(cards).unwrap(),
            serde_json::to_value(&expected).unwrap()
        );
        assert_eq!(
            serde_json::to_value(groups.working_areas).unwrap(),
            serde_json::to_value(vec![expected[1].clone(), expected[3].clone()]).unwrap()
        );
    }
}
