use std::sync::Arc;

use parking_lot::{Mutex, RwLock};

use crate::adaptor::gateway::local_event_store::read_only::LocalEventReadStore;
use crate::adaptor::gateway::local_event_store::LocalEventStore;
use crate::domain::workflow::{
    IsolatedWorktreeLedgerRepository, IsolatedWorktreeLedgerSnapshot, NodeFact, NodeFactMeta,
    NodeFactRecord, WorkflowError,
};

use super::fact_log::{self, FactLogReadBackend};

#[derive(Debug, Clone)]
enum LedgerCache {
    Uninitialized,
    Ready(IsolatedWorktreeLedgerSnapshot),
}

pub(crate) struct NodeEventIsolatedWorktreeLedgerRepository {
    backend: FactLogReadBackend,
    writer: Option<Arc<LocalEventStore>>,
    cache: RwLock<LedgerCache>,
    mutation_lock: Mutex<()>,
}

impl NodeEventIsolatedWorktreeLedgerRepository {
    pub(crate) fn new(store: Arc<LocalEventStore>) -> Self {
        Self {
            backend: FactLogReadBackend::Live(store.clone()),
            writer: Some(store),
            cache: RwLock::new(LedgerCache::Uninitialized),
            mutation_lock: Mutex::new(()),
        }
    }

    pub(crate) fn new_read_only(store: Arc<LocalEventReadStore>) -> Self {
        Self {
            backend: FactLogReadBackend::ReadOnly(store),
            writer: None,
            cache: RwLock::new(LedgerCache::Uninitialized),
            mutation_lock: Mutex::new(()),
        }
    }

    fn load_durable_snapshot(&self) -> Result<IsolatedWorktreeLedgerSnapshot, String> {
        let tree_ids = fact_log::list_tree_ids(&self.backend, None)?;
        let mut snapshot = IsolatedWorktreeLedgerSnapshot::default();
        for tree_id in tree_ids {
            snapshot.merge(&self.load_tree_snapshot(&tree_id)?)?;
        }
        Ok(snapshot)
    }

    fn load_tree_snapshot(&self, tree_id: &str) -> Result<IsolatedWorktreeLedgerSnapshot, String> {
        Ok(fact_log::fold_tree_from(&self.backend, tree_id)?
            .map(|folded| folded.isolated_worktrees)
            .unwrap_or_default())
    }

    fn unavailable(reason: impl Into<String>) -> WorkflowError {
        WorkflowError::IncompatibleStoredEvent(reason.into())
    }

    fn is_worktree_fact(fact: &NodeFact) -> bool {
        matches!(
            fact,
            NodeFact::IsolatedWorktreeCreated(_)
                | NodeFact::IsolatedWorktreeReleased
                | NodeFact::IsolatedWorktreeLost
        )
    }
}

impl IsolatedWorktreeLedgerRepository for NodeEventIsolatedWorktreeLedgerRepository {
    fn snapshot(&self) -> Result<IsolatedWorktreeLedgerSnapshot, WorkflowError> {
        if let LedgerCache::Ready(snapshot) = self.cache.read().clone() {
            return Ok(snapshot);
        }
        let _guard = self.mutation_lock.lock();
        if let LedgerCache::Ready(snapshot) = self.cache.read().clone() {
            return Ok(snapshot);
        }
        let snapshot = self.load_durable_snapshot().map_err(Self::unavailable)?;
        *self.cache.write() = LedgerCache::Ready(snapshot.clone());
        Ok(snapshot)
    }

    fn snapshot_for_tree(
        &self,
        tree_id: &str,
    ) -> Result<IsolatedWorktreeLedgerSnapshot, WorkflowError> {
        self.load_tree_snapshot(tree_id).map_err(Self::unavailable)
    }

    fn append(
        &self,
        meta: &NodeFactMeta,
        fact: &NodeFact,
        timestamp_ms: i64,
    ) -> Result<(), WorkflowError> {
        if !Self::is_worktree_fact(fact) {
            return Err(WorkflowError::validation(
                "isolated worktree ledger accepts only worktree facts",
            ));
        }
        let _guard = self.mutation_lock.lock();
        let writer = self
            .writer
            .as_ref()
            .ok_or_else(|| WorkflowError::external("isolated worktree ledger is read-only"))?;
        let record = NodeFactRecord {
            meta: meta.clone(),
            seq: 0,
            timestamp_ms,
            fact: fact.clone(),
        };
        let mut tree_snapshot = self
            .load_tree_snapshot(&meta.tree_id)
            .map_err(Self::unavailable)?;
        tree_snapshot
            .apply_record(&record)
            .map_err(WorkflowError::invalid_state)?;
        fact_log::append_single_fact(writer, meta, fact, timestamp_ms)
            .map_err(WorkflowError::external)?;
        let cached = self.cache.read().clone();
        if let LedgerCache::Ready(mut snapshot) = cached {
            // durable には追記済みである。summary へ反映できない場合は summary を
            // 捨て、次回の snapshot で node_events から組み直す。
            match snapshot.apply_record(&record) {
                Ok(()) => *self.cache.write() = LedgerCache::Ready(snapshot),
                Err(reason) => {
                    log::warn!("isolated worktree ledger summary was discarded: {reason}");
                    *self.cache.write() = LedgerCache::Uninitialized;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptor::gateway::local_event_store::LocalEventStoreConfig;
    use crate::domain::provider_lifecycle::ProviderKind;
    use crate::domain::workflow::value_objects::IsolatedWorktreeCreatedFact;
    use crate::domain::workflow::SessionExecutionTreeRootFacts;

    fn meta() -> NodeFactMeta {
        SessionExecutionTreeRootFacts::new("node-1", "/repo", "/repo", ProviderKind::Codex)
            .unwrap()
            .meta
    }

    fn seed(store: &Arc<LocalEventStore>) {
        let meta = meta();
        let root_facts =
            SessionExecutionTreeRootFacts::new("node-1", "/repo", "/repo", ProviderKind::Codex)
                .unwrap();
        fact_log::append_fact_batch_for_seed(
            store,
            &root_facts.into_facts(),
            1,
            "worktree-ledger-session-root",
        )
        .unwrap();
        fact_log::append_single_fact(
            store,
            &meta,
            &NodeFact::IsolatedWorktreeCreated(IsolatedWorktreeCreatedFact {
                repository_root: "/repo".to_string(),
                worktree_path: "/repo-worktrees/.releash-isolated/node-1-a1".to_string(),
                branch: "releash/isolated/node-1-a1".to_string(),
            }),
            3,
        )
        .unwrap();
    }

    #[test]
    fn rebuild_and_append_use_node_events_as_the_only_durable_authority() {
        let root = tempfile::TempDir::new().unwrap();
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(root.path().into())).unwrap();
        seed(&store);

        let first = NodeEventIsolatedWorktreeLedgerRepository::new(store.clone());
        assert_eq!(first.snapshot().unwrap().entries().count(), 1);
        first
            .append(&meta(), &NodeFact::IsolatedWorktreeLost, 3)
            .unwrap();
        // append は durable 追記の後に、既に読み出し済みの summary へ delta を適用する。
        assert!(first
            .snapshot()
            .unwrap()
            .recovery_cause_for_node("node-1", "node-1")
            .is_some());

        let restarted = NodeEventIsolatedWorktreeLedgerRepository::new(store);
        let snapshot = restarted.snapshot().unwrap();
        let cause = snapshot
            .recovery_cause_for_node("node-1", "node-1")
            .unwrap();
        assert_eq!(
            cause.to_string(),
            "isolated worktree is missing: /repo-worktrees/.releash-isolated/node-1-a1"
        );
    }

    #[test]
    fn first_snapshot_rebuilds_the_uninitialized_cache() {
        let root = tempfile::TempDir::new().unwrap();
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(root.path().into())).unwrap();
        seed(&store);

        let repository = NodeEventIsolatedWorktreeLedgerRepository::new(store);

        assert_eq!(repository.snapshot().unwrap().entries().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn snapshot_retries_after_a_transient_read_failure() {
        let root = tempfile::TempDir::new().unwrap();
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(root.path().into())).unwrap();
        seed(&store);
        let read_store = LocalEventReadStore::open(root.path()).unwrap();
        let repository = NodeEventIsolatedWorktreeLedgerRepository::new_read_only(read_store);
        let database_path = root
            .path()
            .join(crate::adaptor::gateway::local_event_store::layout::DATABASE_FILE);
        let unavailable_path = root.path().join("temporarily-unavailable.sqlite3");

        std::fs::rename(&database_path, &unavailable_path).unwrap();
        assert!(repository.snapshot().is_err());
        std::fs::rename(&unavailable_path, &database_path).unwrap();

        assert_eq!(repository.snapshot().unwrap().entries().count(), 1);
    }
}
