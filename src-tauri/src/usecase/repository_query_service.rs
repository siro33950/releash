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
use super::worktree_operation::WorktreeOperations;

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
    pub(crate) worktree_operations: Arc<WorktreeOperations>,
}

impl RepositoryQueryService {
    pub(crate) fn new(
        branch_card_query: Arc<dyn BranchCardQuery>,
        worktree_operations: Arc<WorktreeOperations>,
    ) -> Self {
        Self {
            branch_card_query,
            worktree_operations,
        }
    }

    pub(crate) fn include_deleting_worktrees(
        &self,
        repository_root: &str,
        cards: &mut Vec<BranchCardDto>,
    ) {
        self.worktree_operations
            .for_each_deleting_worktree(|worktree| {
                if worktree.repository_root != repository_root {
                    return;
                }
                let path = &worktree.path;
                if let Some(card) = cards.iter_mut().find(|card| {
                    card.worktree_path.as_deref() == Some(path)
                        || worktree.branch.as_deref() == Some(card.name.as_str())
                }) {
                    card.worktree_path.get_or_insert_with(|| path.clone());
                    card.is_deleting = true;
                } else {
                    cards.push(BranchCardDto {
                        name: worktree.branch.clone().unwrap_or_else(|| path.clone()),
                        worktree_path: Some(path.clone()),
                        is_main_worktree: false,
                        is_deleting: true,
                        dirty_count: 0,
                        is_merged: false,
                        ahead: 0,
                        behind: 0,
                        has_upstream: false,
                        base_ahead: 0,
                    });
                }
            });
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
            is_deleting: false,
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
                is_deleting: false,
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
        let service = RepositoryQueryService::new(Arc::new(FakeBranchCards), Default::default());
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
                    is_deleting: false,
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
                    is_deleting: false,
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
                    is_deleting: false,
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
        let service =
            RepositoryQueryService::new(Arc::new(FakeUnsortedBranchCards), Default::default());
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
    #[tokio::test]
    async fn test_worktree削除表示_管理情報が消えても対象だけ保持しguard終了で消す() {
        // Given
        let operations = Arc::new(WorktreeOperations::default());
        let mut deletion = operations.delete("/repo-worktrees/feature").await.unwrap();
        let service = RepositoryQueryService::new(Arc::new(FakeBranchCards), operations.clone());
        deletion
            .accept(
                crate::domain::repository::worktree_operation::WorktreeDeletionTarget {
                    repository_root: "/repo".into(),
                    path: "/repo-worktrees/feature".into(),
                    branch: Some("feature".into()),
                },
            )
            .unwrap();
        for path in [
            Some("/repo-worktrees/feature"),
            Some("/alias/feature"),
            None,
        ] {
            let mut cards = vec![card("main", Some("/repo")), card("feature", path)];
            // When
            service.include_deleting_worktrees("/repo", &mut cards);
            service.include_deleting_worktrees("/repo", &mut cards);
            // Then
            assert_eq!(cards.len(), 2);
            assert!(!cards[0].is_deleting);
            assert!(cards[1].is_deleting);
            assert_eq!(
                cards[1].worktree_path.as_deref(),
                path.or(Some("/repo-worktrees/feature"))
            );
            assert_eq!(
                classify_branch_cards("/repo", &mut cards)
                    .working_areas
                    .len(),
                2
            );
        }
        let mut other = Vec::new();
        service.include_deleting_worktrees("/other", &mut other);
        assert!(other.is_empty());
        let mut missing = Vec::new();
        service.include_deleting_worktrees("/repo", &mut missing);
        assert_eq!(missing.len(), 1);
        assert!(missing[0].is_deleting);
        drop(deletion);
        let mut finished = vec![card("feature", None)];
        service.include_deleting_worktrees("/repo", &mut finished);
        assert!(!finished[0].is_deleting);
        assert!(finished[0].worktree_path.is_none());
        assert!(operations.mutate("/repo-worktrees/feature").is_ok());
    }

    #[tokio::test]
    async fn test_worktree削除表示_ブランチ名が不明でもパスごとに対象を保持する() {
        // Given
        let operations = Arc::new(WorktreeOperations::default());
        let service = RepositoryQueryService::new(Arc::new(FakeBranchCards), operations.clone());
        let mut deletions = Vec::new();
        for path in ["/repo-worktrees/one", "/repo-worktrees/two"] {
            let mut deletion = operations.delete(path).await.unwrap();
            deletion
                .accept(
                    crate::domain::repository::worktree_operation::WorktreeDeletionTarget {
                        repository_root: "/repo".into(),
                        path: path.into(),
                        branch: None,
                    },
                )
                .unwrap();
            deletions.push(deletion);
        }
        let mut cards = vec![
            card("unknown", Some("/repo-worktrees/other")),
            card("feature", Some("/repo-worktrees/one")),
        ];
        // When
        service.include_deleting_worktrees("/repo", &mut cards);
        service.include_deleting_worktrees("/repo", &mut cards);
        // Then
        assert_eq!(cards.len(), 3);
        assert!(!cards[0].is_deleting);
        assert_eq!(cards[1].name, "feature");
        assert!(cards[1].is_deleting);
        assert_eq!(cards[2].name, "/repo-worktrees/two");
        assert_eq!(
            cards[2].worktree_path.as_deref(),
            Some("/repo-worktrees/two")
        );
        assert!(cards[2].is_deleting);
        drop(deletions);
        let mut finished = Vec::new();
        service.include_deleting_worktrees("/repo", &mut finished);
        assert!(finished.is_empty());
    }
}
