//! repository ドメインの読み取りクエリサービス（read model 生成）。
//!
//! **QueryService は Usecase ではない。** 表示・転送向けの read model（DTO）を
//! データソースから直接構築する Query 専用の協力者であり、業務手順（オーケストレーション）
//! は持たない。Entity をそのまま返す単純な読み取りは Repository の責務であり、ここには置かない。
//!
//! git2 のブロッキング呼び出しは gateway 内で同期的に行われ、非同期境界は controller 層で被せる。

use std::collections::BTreeSet;
use std::sync::Arc;

use crate::domain::repository::RepositoryError;
use crate::domain::workflow::services::worktree_reconciliation::{
    classify_inventory_without_ledger, reconcile_worktrees, IsolatedWorktreeOwnerLifecycle,
    IsolatedWorktreeOwnerState, WorktreeClassification,
};
#[cfg(test)]
use crate::domain::workflow::WorkflowError;
use crate::domain::workflow::{
    IsolatedWorktreeIdentity, IsolatedWorktreeLedgerRepository, IsolatedWorktreeLedgerSnapshot,
    IsolatedWorktreeLifecycle, RepositoryWorktreeInventory, WorkflowExecutionId,
    WorktreeInventoryEntry,
};
use crate::usecase::workflow::ports::WorkflowExecutionProjectionRepository;

use super::repository_dto::{BranchCardDto, WorktreeEntryDto};
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

#[derive(Clone)]
pub struct WorktreeClassificationQuery {
    ledger: Arc<dyn IsolatedWorktreeLedgerRepository>,
    executions: Arc<dyn WorkflowExecutionProjectionRepository>,
}

impl WorktreeClassificationQuery {
    pub fn new(
        ledger: Arc<dyn IsolatedWorktreeLedgerRepository>,
        executions: Arc<dyn WorkflowExecutionProjectionRepository>,
    ) -> Self {
        Self { ledger, executions }
    }

    #[cfg(test)]
    pub(crate) fn empty() -> Self {
        Self::new(Arc::new(EmptyLedger), Arc::new(EmptyExecutionProjection))
    }

    fn classify(&self, inventory: &RepositoryWorktreeInventory) -> Vec<WorktreeClassification> {
        // 台帳そのものを読めない場合だけ、canonical naming rule への defensive fallback へ倒す（R-011）。
        let Ok(snapshot) = self.ledger.snapshot() else {
            return classify_inventory_without_ledger(inventory);
        };
        reconcile_worktrees(&snapshot, &self.owner_states(&snapshot), inventory).classifications
    }

    /// 実行木から復元できた所有 Node の状態だけを返す。復元できない entry の
    /// 実体は domain 側で所有者不明として扱われる。
    fn owner_states(
        &self,
        ledger: &IsolatedWorktreeLedgerSnapshot,
    ) -> Vec<IsolatedWorktreeOwnerState> {
        let tree_ids = ledger
            .entries()
            .filter(|entry| entry.lifecycle != IsolatedWorktreeLifecycle::Released)
            .map(|entry| entry.owner.tree_id.clone())
            .collect::<BTreeSet<_>>();
        let mut states = Vec::new();
        for tree_id in tree_ids {
            let Some(execution) = WorkflowExecutionId::new(tree_id.clone())
                .ok()
                .and_then(|execution_id| self.executions.get_execution(&execution_id).ok())
                .flatten()
            else {
                continue;
            };
            for entry in ledger.entries().filter(|entry| {
                entry.owner.tree_id == tree_id
                    && entry.lifecycle != IsolatedWorktreeLifecycle::Released
            }) {
                let identity = IsolatedWorktreeIdentity::from_meta(&entry.owner);
                let Some(node) = execution.node_executions.iter().find(|node| {
                    node.id == identity.node_execution_id && node.attempt == identity.attempt
                }) else {
                    continue;
                };
                states.push(IsolatedWorktreeOwnerState {
                    identity,
                    lifecycle: if node.status.is_active() {
                        IsolatedWorktreeOwnerLifecycle::Active
                    } else {
                        IsolatedWorktreeOwnerLifecycle::Ended
                    },
                });
            }
        }
        states
    }

    pub fn classify_worktree_entries(
        &self,
        repository_root: &str,
        entries: &mut [WorktreeEntryDto],
    ) {
        let inventory = RepositoryWorktreeInventory::new(
            repository_root,
            entries
                .iter()
                .map(|entry| {
                    WorktreeInventoryEntry::new(repository_root, &entry.path, &entry.branch)
                })
                .collect(),
        );
        let classifications = self.classify(&inventory);
        for (entry, classification) in entries.iter_mut().zip(classifications) {
            entry.management_kind = classification.management_kind.as_public_str().to_string();
        }
    }

    pub fn classify_branch_cards(&self, repository_root: &str, cards: &mut [BranchCardDto]) {
        let inventory = RepositoryWorktreeInventory::new(
            repository_root,
            cards
                .iter()
                .filter_map(|card| {
                    card.worktree_path
                        .as_ref()
                        .map(|path| WorktreeInventoryEntry::new(repository_root, path, &card.name))
                })
                .collect(),
        );
        for (card, classification) in cards
            .iter_mut()
            .filter(|card| card.worktree_path.is_some())
            .zip(self.classify(&inventory))
        {
            card.management_kind = Some(classification.management_kind.as_public_str().to_string());
        }
    }
}

#[cfg(test)]
struct EmptyLedger;

#[cfg(test)]
impl IsolatedWorktreeLedgerRepository for EmptyLedger {
    fn snapshot(&self) -> Result<IsolatedWorktreeLedgerSnapshot, WorkflowError> {
        Ok(IsolatedWorktreeLedgerSnapshot::default())
    }

    fn snapshot_for_tree(
        &self,
        _tree_id: &str,
    ) -> Result<IsolatedWorktreeLedgerSnapshot, WorkflowError> {
        Ok(IsolatedWorktreeLedgerSnapshot::default())
    }

    fn append(
        &self,
        _meta: &crate::domain::workflow::NodeFactMeta,
        _fact: &crate::domain::workflow::NodeFact,
        _timestamp_ms: i64,
    ) -> Result<(), WorkflowError> {
        unreachable!()
    }
}

#[cfg(test)]
struct EmptyExecutionProjection;

#[cfg(test)]
impl WorkflowExecutionProjectionRepository for EmptyExecutionProjection {
    fn get_execution(
        &self,
        _execution_id: &WorkflowExecutionId,
    ) -> Result<Option<crate::domain::workflow::WorkflowExecution>, WorkflowError> {
        Ok(None)
    }
}

/// read model（`BranchCardDto`）を構築する読み取りクエリサービス。
/// Usecase から呼ばれる協力者であり、entity 用 Repository には依存しない。
#[derive(Clone)]
pub struct RepositoryQueryService {
    branch_card_query: Arc<dyn BranchCardQuery>,
    worktree_classification: WorktreeClassificationQuery,
}

impl RepositoryQueryService {
    pub fn new(
        branch_card_query: Arc<dyn BranchCardQuery>,
        worktree_classification: WorktreeClassificationQuery,
    ) -> Self {
        Self {
            branch_card_query,
            worktree_classification,
        }
    }

    #[cfg(test)]
    pub fn list_branches_with_status(
        &self,
        repo_path: &str,
        repository_root: &str,
    ) -> Result<Vec<BranchCardDto>, UsecaseError> {
        let mut cards = self.branch_card_query.list_branch_cards(repo_path)?;
        cards.sort_by_key(|card| !card.is_main_worktree);
        self.worktree_classification
            .classify_branch_cards(repository_root, &mut cards);
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
        self.worktree_classification
            .classify_branch_cards(repository_root, &mut cards);
        Ok(cards)
    }

    pub fn classify_worktree_entries(
        &self,
        repository_root: &str,
        entries: &mut [WorktreeEntryDto],
    ) {
        self.worktree_classification
            .classify_worktree_entries(repository_root, entries);
    }
}

#[cfg(test)]
mod repository_query_service_tests {
    use super::*;
    use crate::domain::workflow::value_objects::IsolatedWorktreeCreatedFact;
    use crate::domain::workflow::{
        ExecutionOrigin, ExecutionStatus, IsolatedWorktreeLedgerSnapshot,
        NodeCompletionSignalState, NodeExecution, NodeExecutionStatus, NodeFact, NodeFactMeta,
        NodeFactRecord, NodeKindName, TokenUsage, WorkflowError, WorkflowExecution,
    };

    const TREE_ID: &str = "00000000-0000-4000-8000-000000000001";

    #[derive(Clone)]
    struct FakeLedger {
        snapshot: Result<IsolatedWorktreeLedgerSnapshot, WorkflowError>,
    }

    impl IsolatedWorktreeLedgerRepository for FakeLedger {
        fn snapshot(&self) -> Result<IsolatedWorktreeLedgerSnapshot, WorkflowError> {
            self.snapshot.clone()
        }

        fn snapshot_for_tree(
            &self,
            _tree_id: &str,
        ) -> Result<IsolatedWorktreeLedgerSnapshot, WorkflowError> {
            self.snapshot.clone()
        }

        fn append(
            &self,
            _meta: &NodeFactMeta,
            _fact: &NodeFact,
            _timestamp_ms: i64,
        ) -> Result<(), WorkflowError> {
            unreachable!()
        }
    }

    struct FakeExecutionProjection {
        nodes: Vec<(&'static str, NodeExecutionStatus)>,
    }

    impl WorkflowExecutionProjectionRepository for FakeExecutionProjection {
        fn get_execution(
            &self,
            execution_id: &WorkflowExecutionId,
        ) -> Result<Option<WorkflowExecution>, WorkflowError> {
            Ok(Some(WorkflowExecution {
                id: execution_id.to_string(),
                workflow_name: "test".to_string(),
                status: ExecutionStatus::Running,
                current_node: None,
                created_from: ExecutionOrigin::DesktopUi,
                worktree_path: "/repo".to_string(),
                started_at: 0.0,
                updated_at: 0.0,
                completed_at: None,
                error_reason: None,
                interruption_reason: None,
                resume_from_node: None,
                total_token_usage: TokenUsage::default(),
                node_executions: self
                    .nodes
                    .iter()
                    .map(|(id, status)| NodeExecution {
                        id: (*id).to_string(),
                        execution_id: execution_id.to_string(),
                        node_name: "work".to_string(),
                        kind: NodeKindName::Command,
                        attempt: 1,
                        status: *status,
                        session_id: None,
                        display_command: None,
                        result_summary: None,
                        artifact: None,
                        token_usage: None,
                        failure: None,
                        parent: None,
                        completion_signals: NodeCompletionSignalState::Pending,
                        started_at: 0.0,
                        completed_at: None,
                    })
                    .collect(),
                artifacts: Vec::new(),
                fanouts: Vec::new(),
                approval_target: None,
            }))
        }
    }

    fn meta(node_execution_id: &str) -> NodeFactMeta {
        NodeFactMeta {
            tree_id: TREE_ID.to_string(),
            node_execution_id: node_execution_id.to_string(),
            parent_id: None,
            node_name: "work".to_string(),
            kind: NodeKindName::Command,
            attempt: 1,
        }
    }

    fn created(node_execution_id: &str, seq: i64) -> NodeFactRecord {
        NodeFactRecord {
            meta: meta(node_execution_id),
            seq,
            timestamp_ms: seq,
            fact: NodeFact::IsolatedWorktreeCreated(IsolatedWorktreeCreatedFact {
                repository_root: "/repo".to_string(),
                worktree_path: format!("/repo-worktrees/.releash-isolated/{node_execution_id}-a1"),
                branch: format!("releash/isolated/{node_execution_id}-a1"),
            }),
        }
    }

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
            management_kind: None,
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
                management_kind: None,
            }])
        }
    }

    #[test]
    fn test_ブランチカード一覧を委譲する() {
        let service = RepositoryQueryService::new(
            Arc::new(FakeBranchCards),
            WorktreeClassificationQuery::empty(),
        );
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
                    management_kind: None,
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
                    management_kind: None,
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
                    management_kind: None,
                },
            ])
        }
    }

    #[test]
    fn test_main_worktreeを先頭に正規化する() {
        let service = RepositoryQueryService::new(
            Arc::new(FakeUnsortedBranchCards),
            WorktreeClassificationQuery::empty(),
        );
        let cards = service.list_branches_with_status("/repo", "/repo").unwrap();
        let names: Vec<&str> = cards.iter().map(|card| card.name.as_str()).collect();
        assert_eq!(names, vec!["main", "feature-a", "feature-b"]);
    }

    #[test]
    fn ledger_and_owner_state_classify_branch_cards_without_changing_order() {
        let mut records = vec![created("active", 1), created("ended", 2)];
        let released = created("released", 3);
        records.push(released.clone());
        records.push(NodeFactRecord {
            meta: released.meta,
            seq: 4,
            timestamp_ms: 4,
            fact: NodeFact::IsolatedWorktreeReleased,
        });
        let ledger = FakeLedger {
            snapshot: Ok(IsolatedWorktreeLedgerSnapshot::from_records(&records).unwrap()),
        };
        let classifier = WorktreeClassificationQuery::new(
            Arc::new(ledger),
            Arc::new(FakeExecutionProjection {
                nodes: vec![
                    ("active", NodeExecutionStatus::Running),
                    ("ended", NodeExecutionStatus::Succeeded),
                ],
            }),
        );
        let mut cards = vec![
            card(
                "releash/isolated/active-a1",
                Some("/repo-worktrees/.releash-isolated/active-a1"),
            ),
            card(
                "releash/isolated/ended-a1",
                Some("/repo-worktrees/.releash-isolated/ended-a1"),
            ),
            card(
                "releash/isolated/released-a1",
                Some("/repo-worktrees/.releash-isolated/released-a1"),
            ),
            card("feature", Some("/repo-worktrees/feature")),
            card(
                "releash/isolated/orphan-a1",
                Some("/repo-worktrees/.releash-isolated/orphan-a1"),
            ),
            card("branch-without-worktree", None),
        ];

        classifier.classify_branch_cards("/repo", &mut cards);

        assert_eq!(
            cards
                .iter()
                .map(|card| card.management_kind.as_deref())
                .collect::<Vec<_>>(),
            vec![
                Some("isolated_owned"),
                Some("cleanup_candidate"),
                Some("cleanup_candidate"),
                Some("working_area"),
                Some("untracked_cleanup_candidate"),
                None,
            ]
        );
    }

    #[test]
    fn owner_absent_from_the_execution_tree_only_marks_its_own_worktree_as_cleanup() {
        let ledger = FakeLedger {
            snapshot: Ok(IsolatedWorktreeLedgerSnapshot::from_records(&[
                created("active", 1),
                created("unknown-owner", 2),
            ])
            .unwrap()),
        };
        let classifier = WorktreeClassificationQuery::new(
            Arc::new(ledger),
            Arc::new(FakeExecutionProjection {
                nodes: vec![("active", NodeExecutionStatus::Running)],
            }),
        );
        let mut cards = vec![
            card(
                "releash/isolated/active-a1",
                Some("/repo-worktrees/.releash-isolated/active-a1"),
            ),
            card(
                "releash/isolated/unknown-owner-a1",
                Some("/repo-worktrees/.releash-isolated/unknown-owner-a1"),
            ),
            card("feature", Some("/repo-worktrees/feature")),
        ];

        classifier.classify_branch_cards("/repo", &mut cards);

        assert_eq!(
            cards
                .iter()
                .map(|card| card.management_kind.as_deref())
                .collect::<Vec<_>>(),
            vec![
                Some("isolated_owned"),
                Some("cleanup_candidate"),
                Some("working_area"),
            ]
        );
    }

    #[test]
    fn ledger_read_failure_uses_only_the_complete_path_and_branch_fallback() {
        let classifier = WorktreeClassificationQuery::new(
            Arc::new(FakeLedger {
                snapshot: Err(WorkflowError::external("ledger unavailable")),
            }),
            Arc::new(EmptyExecutionProjection),
        );
        let mut cards = vec![
            card("feature", Some("/repo-worktrees/feature")),
            card(
                "releash/isolated/orphan-a1",
                Some("/repo-worktrees/.releash-isolated/orphan-a1"),
            ),
            card(
                "releash/isolated/path-only-a1",
                Some("/repo-worktrees/not-the-isolated-directory/path-only-a1"),
            ),
        ];

        classifier.classify_branch_cards("/repo", &mut cards);

        assert_eq!(
            cards
                .iter()
                .map(|card| card.management_kind.as_deref())
                .collect::<Vec<_>>(),
            vec![
                Some("working_area"),
                Some("untracked_cleanup_candidate"),
                Some("working_area"),
            ]
        );
    }
}
