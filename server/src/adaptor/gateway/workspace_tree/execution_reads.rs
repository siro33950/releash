use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use crate::adaptor::gateway::local_event_store::{
    node_events::{self, NodeEventRow},
    LocalEventStore,
};
use crate::adaptor::gateway::workflow::fact_log::{self, FactLogReadBackend};
use crate::domain::local_event::{LocalEventQueryError, WorkflowExecutionMetadataRecord};
use crate::domain::workflow::services::fact_replay::{self, FoldedTree, TreeFold};
use crate::domain::workflow::{ExecutionTreeLaunch, WorkflowError};
/// 1 tree の fold 済みの結果と、そこから導出した metadata。
pub(super) type FoldedExecution = Arc<(FoldedTree, WorkflowExecutionMetadataRecord)>;

/// root started に記録された木の所属。
#[derive(Clone)]
pub(super) struct TreeRoot {
    pub(super) workspace_identity: String,
    pub(super) launched_as: ExecutionTreeLaunch,
    started_at_ms: i64,
    pub(super) worktree_path: String,
}

impl TreeRoot {
    fn from_row(row: &NodeEventRow) -> Result<Option<Self>, LocalEventQueryError> {
        crate::adaptor::gateway::workflow::stored_definition::read_tree_header(&row.detail)
            .map_err(|reason| fold_query_error(fact_log::FactReadError::Corrupt(reason)))
            .map(|header| {
                header.map(|header| Self {
                    workspace_identity: header.workspace_identity,
                    worktree_path: header.worktree_path,
                    launched_as: header.launched_as,
                    started_at_ms: row.timestamp_ms,
                })
            })
    }
}

#[derive(Default)]
struct HeldTree {
    /// 直近に確かめた store の最新 seq。
    head: i64,
    root: Option<TreeRoot>,
    session_root: Option<SessionRoot>,
    root_identity: Option<(i64, i64)>,
    fold: Option<HeldFold>,
}

impl HeldTree {
    fn expire(&mut self, idle: std::time::Duration) {
        if self
            .fold
            .as_ref()
            .is_some_and(|fold| fold.used_at.elapsed() > idle)
        {
            self.fold = None;
        }
    }
}

struct HeldFold {
    fold: TreeFold,
    /// ここまでの事実を fold に足した。
    read_through: i64,
    /// 終端の事実を持つ木は、started の復元の仕方が変わる。
    terminal: bool,
    view: FoldedExecution,
    /// 最後に読まれた時刻。読まれなくなった木の fold は捨てる。
    used_at: std::time::Instant,
}

/// 読まれなくなった木の fold を持ち続ける時間。一覧に出ている worktree の木は
/// 定期の読み取りで使われ続けるので、消えた worktree の木だけが捨てられる。
pub(super) const FOLD_IDLE_LIMIT: std::time::Duration = std::time::Duration::from_secs(600);

pub(super) type SessionRoot = Arc<(
    crate::adaptor::gateway::agent_session::SessionLocation,
    crate::domain::agent_session::services::SessionExecutionContext,
)>;

type SessionRootReader<'a> =
    dyn Fn(&NodeEventRow) -> Result<SessionRoot, LocalEventQueryError> + Sync + 'a;

pub struct ExecutionReads {
    pub(crate) store: Arc<LocalEventStore>,
    held: tokio::sync::Mutex<BTreeMap<String, HeldTree>>,
}
impl ExecutionReads {
    pub fn new(store: Arc<LocalEventStore>) -> Arc<Self> {
        Arc::new(Self {
            store,
            held: Default::default(),
        })
    }
    pub(super) fn backend(&self) -> FactLogReadBackend {
        FactLogReadBackend::Live(self.store.clone())
    }

    fn update_root(
        tree: &mut HeldTree,
        head: i64,
        row: Option<&NodeEventRow>,
        idle: std::time::Duration,
    ) -> Result<(), LocalEventQueryError> {
        let identity = row.map(root_identity);
        if head < tree.head || identity != tree.root_identity {
            *tree = HeldTree::default();
        }
        if tree.root.is_none() {
            tree.root = row.map(TreeRoot::from_row).transpose()?.flatten();
        }
        tree.head = head;
        tree.expire(idle);
        tree.root_identity = identity;
        Ok(())
    }

    async fn sync(
        &self,
        held: &mut BTreeMap<String, HeldTree>,
        idle: std::time::Duration,
        session_root: Option<&SessionRootReader<'_>>,
    ) -> Result<(), LocalEventQueryError> {
        let known = held
            .iter()
            .map(|(id, tree)| {
                (
                    id.clone(),
                    (
                        tree.head,
                        tree.root_identity,
                        session_root.is_some()
                            && tree.session_root.is_none()
                            && tree.root.as_ref().is_some_and(|root| {
                                root.launched_as == ExecutionTreeLaunch::Session
                            }),
                    ),
                )
            })
            .collect::<HashMap<_, _>>();
        let (heads, roots, checked) = self
            .backend()
            .run_indexed(move |connection| {
                let unavailable = |e: rusqlite::Error| {
                    crate::adaptor::gateway::local_event_store::reader::storage_unavailable(&e)
                };
                let transaction = connection.unchecked_transaction().map_err(unavailable)?;
                let heads = node_events::tree_heads(&transaction).map_err(unavailable)?;
                let identities = node_events::tree_root_identities(&transaction, "started")
                    .map_err(unavailable)?;
                let ids = heads
                    .iter()
                    .filter_map(|(id, head)| {
                        let needs_root = known.get(id).is_none_or(
                            |(previous_head, identity, missing_session)| {
                                head < previous_head
                                    || *missing_session
                                    || identities.get(id) != identity.as_ref()
                            },
                        );
                        needs_root.then_some(id.clone())
                    })
                    .collect::<Vec<_>>();
                let roots = node_events::first_root_rows_for_trees(&transaction, &ids, "started")
                    .map_err(unavailable)?;
                Ok((heads, roots, ids))
            })
            .await?;
        let checked = checked.into_iter().collect::<HashSet<_>>();
        let mut root_rows = roots
            .into_iter()
            .map(|row| (row.tree_id.clone(), row))
            .collect::<HashMap<_, _>>();
        let present = heads
            .iter()
            .map(|(id, _)| id.as_str())
            .collect::<HashSet<_>>();
        held.retain(|id, _| present.contains(id.as_str()));
        for (id, head) in heads {
            let tree = held.entry(id.clone()).or_default();
            if checked.contains(&id) {
                let row = root_rows.remove(&id);
                Self::update_root(tree, head, row.as_ref(), idle)?;
                if let (Some(read_session_root), Some(row)) = (session_root, row) {
                    if tree.session_root.is_none()
                        && tree
                            .root
                            .as_ref()
                            .is_some_and(|root| root.launched_as == ExecutionTreeLaunch::Session)
                    {
                        tree.session_root = Some(read_session_root(&row)?);
                    }
                }
            } else {
                tree.head = head;
                tree.expire(idle);
            }
        }
        Ok(())
    }

    pub(super) async fn session_root(&self, id: &str) -> Option<SessionRoot> {
        self.held.lock().await.get(id)?.session_root.clone()
    }

    pub(super) async fn record_session_root(&self, id: &str, root: SessionRoot) {
        if let Some(tree) = self.held.lock().await.get_mut(id) {
            tree.session_root = Some(root);
        }
    }

    pub(super) async fn all(
        &self,
        idle: std::time::Duration,
        session_root: impl Fn(&NodeEventRow) -> Result<SessionRoot, LocalEventQueryError> + Sync,
    ) -> Result<(Vec<FoldedExecution>, Vec<(String, LocalEventQueryError)>), LocalEventQueryError>
    {
        let mut held = self.held.lock().await;
        self.sync(&mut held, idle, Some(&session_root)).await?;
        let ids = ordered_roots(&held)
            .into_iter()
            .map(|(id, _)| id)
            .collect::<Vec<_>>();
        self.advance_partial(&mut held, &ids).await
    }

    async fn advance(
        &self,
        held: &mut BTreeMap<String, HeldTree>,
        ids: &[String],
    ) -> Result<Vec<FoldedExecution>, LocalEventQueryError> {
        let (trees, failures) = self.advance_partial(held, ids).await?;
        if let Some((_, error)) = failures.into_iter().next() {
            return Err(error);
        }
        Ok(trees)
    }

    async fn advance_partial(
        &self,
        held: &mut BTreeMap<String, HeldTree>,
        ids: &[String],
    ) -> Result<(Vec<FoldedExecution>, Vec<(String, LocalEventQueryError)>), LocalEventQueryError>
    {
        let mut failures = Vec::new();
        let after = ids
            .iter()
            .filter_map(|id| {
                let tree = &held[id];
                let read = tree.fold.as_ref().map_or(0, |fold| fold.read_through);
                (tree.fold.is_none() || read < tree.head).then_some((id.clone(), read))
            })
            .collect::<HashMap<_, _>>();
        if !after.is_empty() {
            let rows = self
                .backend()
                .run_indexed(move |connection| {
                    read_all_after(connection, &after).map_err(|e| {
                        crate::adaptor::gateway::local_event_store::reader::storage_unavailable(&e)
                    })
                })
                .await?;
            let mut grouped = group_rows(rows);
            let mut restart = HashMap::new();
            for id in ids {
                let Some(rows) = grouped.remove(id) else {
                    continue;
                };
                let tree = held.get_mut(id).expect("selected tree is held");
                if let Some(fold) = &mut tree.fold {
                    if append_rows(fold, &rows).is_none() {
                        tree.fold = None;
                        restart.insert(id.clone(), 0);
                    }
                } else {
                    match restore_fold(id, &rows) {
                        Ok(fold) => tree.fold = fold,
                        Err(error) => failures.push((id.clone(), error)),
                    }
                }
            }
            if !restart.is_empty() {
                let rows = self
                    .backend()
                    .run_indexed(move |connection| {
                        read_all_after(connection, &restart).map_err(|e| {
                            crate::adaptor::gateway::local_event_store::reader::storage_unavailable(
                                &e,
                            )
                        })
                    })
                    .await?;
                for (id, rows) in group_rows(rows) {
                    match restore_fold(&id, &rows) {
                        Ok(fold) => held.get_mut(&id).expect("restarted tree is held").fold = fold,
                        Err(error) => failures.push((id, error)),
                    }
                }
            }
        }
        Ok((
            ids.iter()
                .filter_map(|id| {
                    let fold = held.get_mut(id)?.fold.as_mut()?;
                    fold.used_at = std::time::Instant::now();
                    Some(Arc::clone(&fold.view))
                })
                .collect(),
            failures,
        ))
    }

    pub(super) async fn selected(
        &self,
        include: impl Fn(&TreeRoot) -> bool,
        idle: std::time::Duration,
    ) -> Result<Vec<FoldedExecution>, LocalEventQueryError> {
        let mut held = self.held.lock().await;
        self.sync(&mut held, idle, None).await?;
        let ids = ordered_roots(&held)
            .into_iter()
            .filter(|(_, root)| include(root))
            .map(|(id, _)| id)
            .collect::<Vec<_>>();
        self.advance(&mut held, &ids).await
    }

    pub(super) async fn workspaces(
        &self,
        workspaces: &[&str],
        idle: std::time::Duration,
    ) -> Vec<Result<Vec<FoldedExecution>, LocalEventQueryError>> {
        let mut held = self.held.lock().await;
        if let Err(error) = self.sync(&mut held, idle, None).await {
            return workspaces.iter().map(|_| Err(error.clone())).collect();
        }
        let roots = ordered_roots(&held);
        let mut results = Vec::new();
        for workspace in workspaces {
            let ids = roots
                .iter()
                .filter(|(_, root)| root.workspace_identity == *workspace)
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>();
            results.push(self.advance(&mut held, &ids).await);
        }
        results
    }

    pub(super) async fn tree(
        &self,
        id: &str,
        idle: std::time::Duration,
    ) -> Result<Option<FoldedExecution>, LocalEventQueryError> {
        let mut held = self.held.lock().await;
        let requested = id.to_string();
        let (head, row) = self
            .backend()
            .run_indexed(move |connection| {
                let unavailable = |e: rusqlite::Error| {
                    crate::adaptor::gateway::local_event_store::reader::storage_unavailable(&e)
                };
                let transaction = connection.unchecked_transaction().map_err(unavailable)?;
                let heads =
                    node_events::reference_heads(&transaction, &HashSet::from([requested.clone()]))
                        .map_err(unavailable)?;
                let row = node_events::first_root_row_of_tree(&transaction, &requested, "started")
                    .map_err(unavailable)?;
                Ok((heads.first().map(|(_, head)| *head), row))
            })
            .await?;
        let (Some(head), Some(row)) = (head, row) else {
            held.remove(id);
            return Ok(None);
        };
        Self::update_root(
            held.entry(id.to_string()).or_default(),
            head,
            Some(&row),
            idle,
        )?;
        Ok(self.advance(&mut held, &[id.to_string()]).await?.pop())
    }
}

fn group_rows(rows: Vec<NodeEventRow>) -> HashMap<String, Vec<NodeEventRow>> {
    let mut grouped = HashMap::<String, Vec<NodeEventRow>>::new();
    for row in rows {
        grouped.entry(row.tree_id.clone()).or_default().push(row);
    }
    grouped
}
fn read_all_after(
    connection: &rusqlite::Connection,
    after: &HashMap<String, i64>,
) -> Result<Vec<NodeEventRow>, rusqlite::Error> {
    if after.len() == 1 {
        let (id, seq) = after.iter().next().expect("one requested tree");
        return node_events::read_tree_after(connection, id, *seq);
    }
    let mut statement = connection.prepare(
        "SELECT e.tree_id, e.seq, e.node_execution_id, e.parent_id, e.node_name, e.kind, e.attempt, e.event_type, e.session_id, e.detail, e.timestamp
         FROM json_each(?1) AS known JOIN node_events AS e ON e.tree_id = known.key AND e.seq > known.value
         ORDER BY e.tree_id, e.seq"
    )?;
    let rows = statement.query_map(
        [serde_json::to_string(after).expect("serializable heads")],
        |row| {
            Ok(NodeEventRow {
                tree_id: row.get(0)?,
                seq: row.get(1)?,
                node_execution_id: row.get(2)?,
                parent_id: row.get(3)?,
                node_name: row.get(4)?,
                kind: row.get(5)?,
                attempt: row.get(6)?,
                event_type: row.get(7)?,
                session_id: row.get(8)?,
                detail: row.get(9)?,
                timestamp_ms: row.get(10)?,
            })
        },
    )?;
    rows.collect()
}

fn restore_fold(id: &str, rows: &[NodeEventRow]) -> Result<Option<HeldFold>, LocalEventQueryError> {
    let corrupt = |reason: String| fold_query_error(fact_log::FactReadError::Corrupt(reason));
    let records = fact_log::records_from_tree_rows(rows).map_err(corrupt)?;
    let Some((fold, folded)) = TreeFold::from_records(id, &records).map_err(corrupt)? else {
        return Ok(None);
    };
    Ok(Some(HeldFold {
        fold,
        read_through: rows.last().map_or(0, |row| row.seq),
        terminal: fact_log::terminal_fact_in_rows(rows).map_err(corrupt)?,
        view: folded_execution(folded),
        used_at: std::time::Instant::now(),
    }))
}

fn root_identity(row: &NodeEventRow) -> (i64, i64) {
    (row.seq, row.timestamp_ms)
}

fn ordered_roots(held: &BTreeMap<String, HeldTree>) -> Vec<(String, TreeRoot)> {
    let mut roots = held
        .iter()
        .filter_map(|(tree_id, tree)| Some((tree_id.clone(), tree.root.clone()?)))
        .collect::<Vec<_>>();
    roots.sort_by(|left, right| {
        (left.1.started_at_ms, &left.0).cmp(&(right.1.started_at_ms, &right.0))
    });
    roots
}

fn folded_execution(folded: FoldedTree) -> FoldedExecution {
    let model = fact_replay::derive_read_model(&folded);
    let record = fact_log::metadata_record_from_read_model(&model);
    Arc::new((folded, record))
}

/// 追記された行を fold に足す。足せない事実が含まれていれば None。
fn append_rows(held: &mut HeldFold, rows: &[NodeEventRow]) -> Option<FoldedExecution> {
    let records = fact_log::records_from_appended_rows(rows, held.terminal).ok()??;
    for record in &records {
        if held.fold.requires_restart(record) {
            return None;
        }
        held.fold.push(record).ok()?;
    }
    held.view = folded_execution(held.fold.view().ok()?);
    if let Some(row) = rows.last() {
        held.read_through = row.seq;
    }
    Some(Arc::clone(&held.view))
}

pub(super) fn fold_query_error(error: fact_log::FactReadError) -> LocalEventQueryError {
    error.into()
}

pub(super) fn execution_read_error(error: LocalEventQueryError) -> WorkflowError {
    match error {
        error @ LocalEventQueryError::StorageAccessRequired { .. } => {
            WorkflowError::Store(error.into())
        }
        LocalEventQueryError::Technical(stopped) => WorkflowError::Technical(stopped),
        error @ (LocalEventQueryError::StorageUnavailable { .. }
        | LocalEventQueryError::QueryBusy) => {
            let message = error.to_string();
            WorkflowError::storage(error, message)
        }
        error @ LocalEventQueryError::Corrupt { .. } => {
            WorkflowError::CorruptStoredState(error.to_string())
        }
        error @ LocalEventQueryError::IncompatibleStoredEvent { .. } => {
            WorkflowError::IncompatibleStoredEvent(error.to_string())
        }
        error => WorkflowError::external(error.to_string()),
    }
}

#[cfg(test)]
#[path = "execution_reads_test.rs"]
mod execution_reads_tests;
