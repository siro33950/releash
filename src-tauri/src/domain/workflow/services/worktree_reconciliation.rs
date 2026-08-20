use crate::domain::workflow::{
    IsolatedWorktreeIdentity, IsolatedWorktreeLedgerEntry, IsolatedWorktreeLedgerSnapshot,
    IsolatedWorktreeLifecycle, IsolatedWorktreeRecoveryCause, RepositoryWorktreeInventory,
    WorktreeInventoryEntry, WorktreeManagementKind,
};

/// 隔離 worktree を所有する Node attempt の実行状態。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsolatedWorktreeOwnerLifecycle {
    /// 再開対象になり得る。
    Active,
    /// 実行が終了している。
    Ended,
    /// 実行木に所有 Node が存在しない。
    Unknown,
    /// 実行木を読み取れず、所有 Node の状態を判定できない。
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IsolatedWorktreeOwnerState {
    pub identity: IsolatedWorktreeIdentity,
    pub lifecycle: IsolatedWorktreeOwnerLifecycle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeClassification {
    pub inventory: WorktreeInventoryEntry,
    pub management_kind: WorktreeManagementKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IsolatedWorktreeLoss {
    pub entry: IsolatedWorktreeLedgerEntry,
    pub cause: IsolatedWorktreeRecoveryCause,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorktreeReconciliation {
    pub classifications: Vec<WorktreeClassification>,
    pub losses: Vec<IsolatedWorktreeLoss>,
}

pub fn reconcile_worktrees(
    ledger: &IsolatedWorktreeLedgerSnapshot,
    owner_states: &[IsolatedWorktreeOwnerState],
    inventory: &RepositoryWorktreeInventory,
) -> WorktreeReconciliation {
    let mut result = WorktreeReconciliation::default();
    for worktree in &inventory.worktrees {
        let management_kind =
            match ledger.entry_for_path(&worktree.repository_root, &worktree.worktree_path) {
                Some(entry) => classify_tracked(entry, owner_states),
                None if worktree.matches_isolated_identity_rule() => {
                    WorktreeManagementKind::UntrackedCleanupCandidate
                }
                None => WorktreeManagementKind::WorkingArea,
            };
        result.classifications.push(WorktreeClassification {
            inventory: worktree.clone(),
            management_kind,
        });
    }

    for entry in ledger.entries() {
        if entry.repository_root != inventory.repository_root {
            continue;
        }
        if entry.lifecycle != IsolatedWorktreeLifecycle::Created {
            continue;
        }
        let exists = inventory.worktrees.iter().any(|worktree| {
            worktree.repository_root == entry.repository_root
                && worktree.worktree_path == entry.worktree_path
        });
        // 所有 Node の状態を復元できない entry では喪失を確定させない。
        if owner_lifecycle(entry, owner_states) == IsolatedWorktreeOwnerLifecycle::Active && !exists
        {
            result.losses.push(IsolatedWorktreeLoss {
                entry: entry.clone(),
                cause: IsolatedWorktreeRecoveryCause::new(&entry.worktree_path),
            });
        }
    }
    result
}

pub fn classify_inventory_without_ledger(
    inventory: &RepositoryWorktreeInventory,
) -> Vec<WorktreeClassification> {
    inventory
        .worktrees
        .iter()
        .map(|worktree| WorktreeClassification {
            inventory: worktree.clone(),
            management_kind: if worktree.matches_isolated_identity_rule() {
                WorktreeManagementKind::UntrackedCleanupCandidate
            } else {
                WorktreeManagementKind::WorkingArea
            },
        })
        .collect()
}

fn classify_tracked(
    entry: &IsolatedWorktreeLedgerEntry,
    owner_states: &[IsolatedWorktreeOwnerState],
) -> WorktreeManagementKind {
    // 解放済みと喪失記録済みの②は、実体が残っていても所有 Node の再開には使われない。
    if entry.lifecycle != IsolatedWorktreeLifecycle::Created {
        return WorktreeManagementKind::CleanupCandidate;
    }
    match owner_lifecycle(entry, owner_states) {
        IsolatedWorktreeOwnerLifecycle::Active => WorktreeManagementKind::IsolatedOwned,
        // 所有 Node が終了した②と、実行木に所有 Node が無い②は、どちらも
        // 再開対象になり得ないため人間へ掃除候補として提示する。
        IsolatedWorktreeOwnerLifecycle::Ended | IsolatedWorktreeOwnerLifecycle::Unknown => {
            WorktreeManagementKind::CleanupCandidate
        }
        // 所有 Node の状態を判定できない間は、掃除候補として提示しない。
        IsolatedWorktreeOwnerLifecycle::Unavailable => WorktreeManagementKind::IsolatedOwned,
    }
}

fn owner_lifecycle(
    entry: &IsolatedWorktreeLedgerEntry,
    owner_states: &[IsolatedWorktreeOwnerState],
) -> IsolatedWorktreeOwnerLifecycle {
    let identity = entry.identity();
    owner_states
        .iter()
        .find(|owner| owner.identity == identity)
        .map(|owner| owner.lifecycle)
        .unwrap_or(IsolatedWorktreeOwnerLifecycle::Unknown)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::value_objects::IsolatedWorktreeCreatedFact;
    use crate::domain::workflow::{NodeFact, NodeFactMeta, NodeFactRecord, NodeKindName};

    fn meta(id: &str, attempt: u32) -> NodeFactMeta {
        NodeFactMeta {
            tree_id: "tree-1".to_string(),
            node_execution_id: id.to_string(),
            parent_id: None,
            node_name: "work".to_string(),
            kind: NodeKindName::Command,
            attempt,
        }
    }

    fn record(meta: NodeFactMeta, fact: NodeFact, seq: i64) -> NodeFactRecord {
        NodeFactRecord {
            meta,
            seq,
            timestamp_ms: seq * 1_000,
            fact,
        }
    }

    fn created(meta: NodeFactMeta) -> NodeFactRecord {
        let token = meta.node_execution_id.clone();
        record(
            meta,
            NodeFact::IsolatedWorktreeCreated(IsolatedWorktreeCreatedFact {
                repository_root: "/projects/repo".to_string(),
                worktree_path: format!("/projects/repo-worktrees/.releash-isolated/{token}-a1"),
                branch: format!("releash/isolated/{token}-a1"),
            }),
            1,
        )
    }

    fn inventory(id: &str) -> WorktreeInventoryEntry {
        WorktreeInventoryEntry::new(
            "/projects/repo",
            format!("/projects/repo-worktrees/.releash-isolated/{id}-a1"),
            format!("releash/isolated/{id}-a1"),
        )
    }

    fn owner(id: &str, lifecycle: IsolatedWorktreeOwnerLifecycle) -> IsolatedWorktreeOwnerState {
        IsolatedWorktreeOwnerState {
            identity: IsolatedWorktreeIdentity::from_meta(&meta(id, 1)),
            lifecycle,
        }
    }

    #[test]
    fn active_owner_with_missing_worktree_emits_one_loss_until_lost_is_recorded() {
        let owner_meta = meta("node-1", 1);
        let ledger =
            IsolatedWorktreeLedgerSnapshot::from_records(&[created(owner_meta.clone())]).unwrap();
        let owners = [owner("node-1", IsolatedWorktreeOwnerLifecycle::Active)];
        let empty_inventory = RepositoryWorktreeInventory::new("/projects/repo", Vec::new());
        let first = reconcile_worktrees(&ledger, &owners, &empty_inventory);
        assert_eq!(first.losses.len(), 1);
        assert_eq!(
            first.losses[0].cause.to_string(),
            "isolated worktree is missing: /projects/repo-worktrees/.releash-isolated/node-1-a1"
        );

        let mut records = vec![created(owner_meta.clone())];
        records.push(record(owner_meta, NodeFact::IsolatedWorktreeLost, 2));
        let lost = IsolatedWorktreeLedgerSnapshot::from_records(&records).unwrap();
        let second = reconcile_worktrees(&lost, &owners, &empty_inventory);
        assert!(second.losses.is_empty());
    }

    #[test]
    fn unknown_owner_is_a_cleanup_candidate_and_never_confirms_a_loss() {
        let owner_meta = meta("node-1", 1);
        let ledger = IsolatedWorktreeLedgerSnapshot::from_records(&[created(owner_meta)]).unwrap();

        let missing = reconcile_worktrees(
            &ledger,
            &[],
            &RepositoryWorktreeInventory::new("/projects/repo", Vec::new()),
        );
        assert!(missing.losses.is_empty());

        let present = reconcile_worktrees(
            &ledger,
            &[],
            &RepositoryWorktreeInventory::new("/projects/repo", vec![inventory("node-1")]),
        );
        assert_eq!(
            present.classifications[0].management_kind,
            WorktreeManagementKind::CleanupCandidate
        );
        assert!(present.losses.is_empty());
    }

    #[test]
    fn a_recovered_worktree_of_a_lost_entry_stays_a_cleanup_candidate() {
        let owner_meta = meta("node-1", 1);
        let mut records = vec![created(owner_meta.clone())];
        records.push(record(owner_meta, NodeFact::IsolatedWorktreeLost, 2));
        let ledger = IsolatedWorktreeLedgerSnapshot::from_records(&records).unwrap();

        let actual = reconcile_worktrees(
            &ledger,
            &[owner("node-1", IsolatedWorktreeOwnerLifecycle::Active)],
            &RepositoryWorktreeInventory::new("/projects/repo", vec![inventory("node-1")]),
        );

        assert_eq!(
            actual.classifications[0].management_kind,
            WorktreeManagementKind::CleanupCandidate
        );
    }

    #[test]
    fn an_unreadable_owner_is_not_offered_as_a_cleanup_candidate() {
        let owner_meta = meta("node-1", 1);
        let ledger = IsolatedWorktreeLedgerSnapshot::from_records(&[created(owner_meta)]).unwrap();

        let actual = reconcile_worktrees(
            &ledger,
            &[owner("node-1", IsolatedWorktreeOwnerLifecycle::Unavailable)],
            &RepositoryWorktreeInventory::new("/projects/repo", vec![inventory("node-1")]),
        );

        assert_eq!(
            actual.classifications[0].management_kind,
            WorktreeManagementKind::IsolatedOwned
        );
        assert!(actual.losses.is_empty());
    }

    #[test]
    fn tracked_inventory_is_hidden_or_cleanup_candidate_from_owner_lifecycle() {
        let active_meta = meta("active", 1);
        let ended_meta = meta("ended", 1);
        let released_meta = meta("released", 1);
        let mut records = vec![
            created(active_meta),
            created(ended_meta),
            created(released_meta.clone()),
        ];
        records.push(record(released_meta, NodeFact::IsolatedWorktreeReleased, 4));
        let ledger = IsolatedWorktreeLedgerSnapshot::from_records(&records).unwrap();
        let owners = [
            owner("active", IsolatedWorktreeOwnerLifecycle::Active),
            owner("ended", IsolatedWorktreeOwnerLifecycle::Ended),
        ];
        let actual = reconcile_worktrees(
            &ledger,
            &owners,
            &RepositoryWorktreeInventory::new(
                "/projects/repo",
                vec![
                    inventory("active"),
                    inventory("ended"),
                    inventory("released"),
                ],
            ),
        );
        assert_eq!(
            actual
                .classifications
                .iter()
                .map(|item| item.management_kind)
                .collect::<Vec<_>>(),
            vec![
                WorktreeManagementKind::IsolatedOwned,
                WorktreeManagementKind::CleanupCandidate,
                WorktreeManagementKind::CleanupCandidate,
            ]
        );
    }

    #[test]
    fn inventory_without_ledger_uses_only_the_complete_defensive_rule() {
        let ordinary = WorktreeInventoryEntry::new(
            "/projects/repo",
            "/projects/repo-worktrees/feature",
            "feature",
        );
        let classified = classify_inventory_without_ledger(&RepositoryWorktreeInventory::new(
            "/projects/repo",
            vec![inventory("orphan"), ordinary],
        ));
        assert_eq!(
            classified
                .iter()
                .map(|item| item.management_kind)
                .collect::<Vec<_>>(),
            vec![
                WorktreeManagementKind::UntrackedCleanupCandidate,
                WorktreeManagementKind::WorkingArea,
            ]
        );
    }
}
