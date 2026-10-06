use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use crate::adaptor::gateway::agent_session::{
    agent_session_from_fields, read_session_context, SessionContextReadError, SessionLocation,
};
use crate::adaptor::gateway::local_event_store::node_events::{self, NodeEventRow};
use crate::adaptor::gateway::local_event_store::read_only::LocalEventReadStore;
use crate::adaptor::gateway::local_event_store::store::LocalEventStore;
use crate::adaptor::gateway::workflow::fact_log::{self, FactLogReadBackend};
use crate::domain::agent_session::aggregates::AgentSession;
use crate::domain::agent_session::services::{session_fields_from_facts, SessionExecutionContext};
use crate::domain::local_event::{LocalEventQueryError, WorkflowExecutionMetadataRecord};
use crate::domain::workflow::services::fact_replay::{self, FoldedTree, TreeFold};
use crate::domain::workflow::{ExecutionTreeLaunch, WorkflowError};
use crate::domain::workspace_tree::WorkspaceExecution;
use crate::domain::workspace_tree::{
    RuntimeSnapshotNodeProjection, WorkspaceIdentity, WorkspacePublicRoot, WorkspaceStructureFact,
    WorkspaceTree, WorkspaceTreeNode, WorkspaceTreeProjector, WorkspaceTreeRepository,
};

#[derive(Clone)]
enum WorkspaceSqliteBackend {
    Live(Arc<LocalEventStore>),
    ReadOnly(Arc<LocalEventReadStore>),
}

/// 1 tree の fold 済みの結果と、そこから導出した metadata。
pub(super) type FoldedExecution = Arc<(FoldedTree, WorkflowExecutionMetadataRecord)>;

type SessionRoot = Arc<(SessionLocation, SessionExecutionContext)>;

/// root started に記録された木の所属。
#[derive(Clone)]
pub(super) struct TreeRoot {
    pub(super) workspace_identity: String,
    pub(super) worktree_path: String,
    pub(super) launched_as: ExecutionTreeLaunch,
    started_at_ms: i64,
}

#[derive(Default)]
struct HeldTree {
    /// 直近に確かめた store の最新 seq。
    head: i64,
    root: Option<TreeRoot>,
    /// root を探した時点の最新 seq。root が無い木は、事実が足されたら探し直す。
    root_checked_at: i64,
    fold: Option<HeldFold>,
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
const FOLD_IDLE_LIMIT: std::time::Duration = std::time::Duration::from_secs(600);

/// The only concrete `WorkspaceTreeRepository` implementation.
///
/// tree ごとに fold 済みの結果を保持し、読むときに追記された事実だけを足す。
pub struct SqliteWorkspaceTreeRepository {
    backend: WorkspaceSqliteBackend,
    pub processes: Option<Arc<dyn crate::domain::workflow::NodeProcessReader>>,
    pub fold_idle_limit: std::time::Duration,
    // ponytail: 全 tree で 1 つの排他。読み取りが競合するなら tree ごとに分ける。
    held: tokio::sync::Mutex<BTreeMap<String, HeldTree>>,
    /// Session として起動した木の root の位置と実行の文脈。root started から決まり、変わらない。
    session_roots: parking_lot::Mutex<HashMap<String, SessionRoot>>,
}

impl SqliteWorkspaceTreeRepository {
    pub fn new(store: Arc<LocalEventStore>) -> Arc<Self> {
        Arc::new(Self {
            backend: WorkspaceSqliteBackend::Live(store),
            processes: None,
            fold_idle_limit: FOLD_IDLE_LIMIT,
            held: Default::default(),
            session_roots: Default::default(),
        })
    }

    pub fn new_read_only(store: Arc<LocalEventReadStore>) -> Arc<Self> {
        Arc::new(Self {
            backend: WorkspaceSqliteBackend::ReadOnly(store),
            processes: None,
            fold_idle_limit: FOLD_IDLE_LIMIT,
            held: Default::default(),
            session_roots: Default::default(),
        })
    }

    pub(super) fn fact_backend(&self) -> FactLogReadBackend {
        match &self.backend {
            WorkspaceSqliteBackend::Live(store) => FactLogReadBackend::Live(Arc::clone(store)),
            WorkspaceSqliteBackend::ReadOnly(store) => {
                FactLogReadBackend::ReadOnly(Arc::clone(store))
            }
        }
    }

    /// store にある tree とその最新 seq を確かめ、保持している一覧を合わせる。
    async fn sync(
        &self,
        held: &mut BTreeMap<String, HeldTree>,
    ) -> Result<(), LocalEventQueryError> {
        let known = held
            .iter()
            .map(|(tree_id, tree)| (tree_id.clone(), (tree.root.is_some(), tree.root_checked_at)))
            .collect::<HashMap<_, _>>();
        let (heads, roots) = self
            .fact_backend()
            .run_indexed(move |connection| {
                let unavailable = |error: rusqlite::Error| {
                    crate::adaptor::gateway::local_event_store::reader::storage_unavailable(&error)
                };
                let heads = node_events::tree_heads(connection).map_err(unavailable)?;
                let mut roots = Vec::new();
                for (tree_id, head) in &heads {
                    let root_is_current = known
                        .get(tree_id)
                        .is_some_and(|(has_root, checked_at)| *has_root || checked_at == head);
                    if !root_is_current {
                        roots.push((
                            tree_id.clone(),
                            node_events::first_root_row_of_tree(connection, tree_id, "started")
                                .map_err(unavailable)?,
                        ));
                    }
                }
                Ok((heads, roots))
            })
            .await?;
        let present = heads
            .iter()
            .map(|(tree_id, _)| tree_id.as_str())
            .collect::<HashSet<_>>();
        held.retain(|tree_id, _| present.contains(tree_id.as_str()));
        self.session_roots
            .lock()
            .retain(|tree_id, _| present.contains(tree_id.as_str()));
        for (tree_id, head) in heads {
            let tree = held.entry(tree_id).or_default();
            if head < tree.head {
                // 削除された tree と同じ識別子で作り直された。
                *tree = HeldTree::default();
            }
            tree.head = head;
            if tree
                .fold
                .as_ref()
                .is_some_and(|fold| fold.used_at.elapsed() > self.fold_idle_limit)
            {
                tree.fold = None;
            }
        }
        for (tree_id, row) in roots {
            let tree = held.entry(tree_id).or_default();
            tree.root_checked_at = tree.head;
            tree.root = match row {
                Some(row) => {
                    crate::adaptor::gateway::workflow::stored_definition::read_tree_header(
                        &row.detail,
                    )
                    .map_err(|reason| fold_query_error(fact_log::FactReadError::Corrupt(reason)))?
                    .map(|header| TreeRoot {
                        workspace_identity: header.workspace_identity,
                        worktree_path: header.worktree_path,
                        launched_as: header.launched_as,
                        started_at_ms: row.timestamp_ms,
                    })
                }
                None => None,
            };
        }
        Ok(())
    }

    /// 保持している fold を store の最新まで進めて返す。木が存在しなければ None。
    async fn catch_up(
        &self,
        tree_id: &str,
        tree: &mut HeldTree,
    ) -> Result<Option<FoldedExecution>, LocalEventQueryError> {
        let backend = self.fact_backend();
        if let Some(held) = &mut tree.fold {
            held.used_at = std::time::Instant::now();
            if held.read_through >= tree.head {
                return Ok(Some(Arc::clone(&held.view)));
            }
            let (requested, after) = (tree_id.to_string(), held.read_through);
            let rows = backend
                .run_indexed(move |connection| {
                    node_events::read_tree_after(connection, &requested, after).map_err(|error| {
                        crate::adaptor::gateway::local_event_store::reader::storage_unavailable(
                            &error,
                        )
                    })
                })
                .await?;
            if let Some(view) = append_rows(held, &rows) {
                return Ok(Some(view));
            }
            // 追記として足せない事実だったので、最初から読み直す。
            tree.fold = None;
        }
        let requested = tree_id.to_string();
        let rows = backend
            .run_indexed(move |connection| {
                node_events::read_tree(connection, &requested).map_err(|error| {
                    crate::adaptor::gateway::local_event_store::reader::storage_unavailable(&error)
                })
            })
            .await?;
        let corrupt = |reason: String| fold_query_error(fact_log::FactReadError::Corrupt(reason));
        let records = fact_log::records_from_tree_rows(&rows).map_err(corrupt)?;
        let Some((fold, folded)) = TreeFold::from_records(tree_id, &records).map_err(corrupt)?
        else {
            return Ok(None);
        };
        let view = folded_execution(folded);
        tree.fold = Some(HeldFold {
            fold,
            read_through: rows.last().map_or(0, |row| row.seq),
            terminal: fact_log::terminal_fact_in_rows(&rows).map_err(corrupt)?,
            view: Arc::clone(&view),
            used_at: std::time::Instant::now(),
        });
        Ok(Some(view))
    }

    /// 保持している木の所属（root started の追記順）。
    pub(super) async fn tree_roots(&self) -> Result<Vec<(String, TreeRoot)>, LocalEventQueryError> {
        let mut held = self.held.lock().await;
        self.sync(&mut held).await?;
        Ok(ordered_roots(&held))
    }

    /// workspace identity が一致する全実行木の fold と metadata。
    pub async fn folded_workspace_trees(
        &self,
        workspace: &str,
    ) -> Result<Vec<FoldedExecution>, LocalEventQueryError> {
        self.folded_workspaces(&[workspace])
            .await
            .pop()
            .expect("one result per requested workspace")
    }

    /// 複数の workspace をまとめて読む。store の確認は 1 回だけ行う。
    pub async fn folded_workspaces(
        &self,
        workspaces: &[&str],
    ) -> Vec<Result<Vec<FoldedExecution>, LocalEventQueryError>> {
        let mut held = self.held.lock().await;
        if let Err(error) = self.sync(&mut held).await {
            return workspaces.iter().map(|_| Err(error.clone())).collect();
        }
        let roots = ordered_roots(&held);
        let mut results = Vec::with_capacity(workspaces.len());
        for workspace in workspaces {
            let mut trees = Ok(Vec::new());
            for (tree_id, root) in &roots {
                if root.workspace_identity != *workspace {
                    continue;
                }
                let tree = held.get_mut(tree_id).expect("root belongs to a held tree");
                match self.catch_up(tree_id, tree).await {
                    Ok(Some(folded)) => {
                        if let Ok(trees) = &mut trees {
                            trees.push(folded);
                        }
                    }
                    Ok(None) => {}
                    Err(error) => {
                        trees = Err(error);
                        break;
                    }
                }
            }
            results.push(trees);
        }
        results
    }

    /// 1 tree の fold と metadata。
    pub async fn folded_tree(
        &self,
        tree_id: &str,
    ) -> Result<Option<FoldedExecution>, LocalEventQueryError> {
        let mut held = self.held.lock().await;
        self.sync(&mut held).await?;
        match held.get_mut(tree_id) {
            Some(tree) => self.catch_up(tree_id, tree).await,
            None => Ok(None),
        }
    }

    fn tree_nodes(
        &self,
        workspace: &str,
        folded: &FoldedTree,
        record: &WorkflowExecutionMetadataRecord,
    ) -> Result<Vec<WorkspaceTreeNode>, LocalEventQueryError> {
        let mut process_presences = std::collections::HashMap::new();
        if let Some(processes) = &self.processes {
            for node in &folded.aggregate.node_executions {
                process_presences.insert(
                    node.id.clone(),
                    processes
                        .presence(workspace, &node.id, node.kind, node.session_id.as_deref())
                        .map_err(|error| invariant_query_error(error.to_string()))?,
                );
            }
        }
        let delegate_waiting_node_ids = folded
            .aggregate
            .node_executions
            .iter()
            .filter(|node| folded.aggregate.delegate_waits_for_child(&node.id))
            .map(|node| node.id.clone())
            .collect();
        crate::domain::workspace_tree::runtime_snapshot_nodes(RuntimeSnapshotNodeProjection {
            process_presences: &process_presences,
            execution_id: &folded.aggregate.id,
            workflow_name: &folded.aggregate.workflow_name,
            workspace_identity: workspace,
            workflow_definition: folded.aggregate.workflow.as_ref(),
            node_executions: &folded.aggregate.node_executions,
            retry_predecessors: &folded.aggregate.retry_predecessors,
            delegate_waiting_node_ids: &delegate_waiting_node_ids,
            execution_active: folded.aggregate.is_active(),
            started_at: folded.aggregate.started_at,
            updated_at: folded.aggregate.updated_at,
            execution: record,

            session_activities: &folded.session_activities,
            session_display_names: &folded.session_display_names,
        })
        .map_err(invariant_query_error)
    }

    /// Workspace の実行木を、実行ごとの状態と一緒に組み立てる。
    async fn workspace_tree(
        &self,
        workspace: &str,
        trees: &[FoldedExecution],
    ) -> Result<WorkspaceTree, LocalEventQueryError> {
        let mut tree = self
            .workspace_tree_from_folded(workspace, trees)?
            .unwrap_or_else(|| WorkspaceTree::empty(workspace));
        let mut executions = Vec::with_capacity(trees.len());
        for execution in trees {
            let (folded, record) = &**execution;
            executions.push(WorkspaceExecution {
                execution_id: record.execution_id.clone(),
                launched_as: folded.root.launched_as,
                worktree_path: record.worktree_path.clone(),
                workflow_name: record.workflow_name.clone(),
                status: record.status,
                updated_at: f64::from_bits(record.updated_at_bits),
                archive: folded.aggregate.archive_record(),
                session: self.execution_session(folded).await?,
            });
        }
        tree.record_executions(executions);
        Ok(tree)
    }

    /// Session として起動した実行木の session を、保持している事実の導出から復元する。
    async fn execution_session(
        &self,
        folded: &FoldedTree,
    ) -> Result<Option<AgentSession>, LocalEventQueryError> {
        if folded.root.launched_as != ExecutionTreeLaunch::Session {
            return Ok(None);
        }
        let tree_id = folded.aggregate.id.as_str();
        let Some(root) = self.session_root(tree_id).await? else {
            return Ok(None);
        };
        let (location, context) = &*root;
        let Some(session) = folded.sessions.get(&location.node_execution_id) else {
            return Ok(None);
        };
        let fields = session_fields_from_facts(
            session.facts.clone(),
            context,
            &location.tree_id,
            &location.node_execution_id,
            &session.session_id,
        )
        .map_err(|error| invariant_query_error(format!("{error:?}")))?;
        agent_session_from_fields(&session.session_id, fields)
            .map(Some)
            .map_err(|error| invariant_query_error(format!("{error:?}")))
    }

    async fn session_root(
        &self,
        tree_id: &str,
    ) -> Result<Option<SessionRoot>, LocalEventQueryError> {
        if let Some(root) = self.session_roots.lock().get(tree_id) {
            return Ok(Some(Arc::clone(root)));
        }
        let backend = self.fact_backend();
        let requested = tree_id.to_string();
        let Some(row) = backend
            .run_indexed(move |connection| {
                node_events::first_row_of_tree(connection, &requested).map_err(|error| {
                    crate::adaptor::gateway::local_event_store::reader::storage_unavailable(&error)
                })
            })
            .await?
        else {
            return Ok(None);
        };
        let location = SessionLocation {
            attempt: u32::try_from(row.attempt)
                .map_err(|_| invariant_query_error("session root attempt is invalid"))?,
            tree_id: row.tree_id,
            node_execution_id: row.node_execution_id,
            parent_id: row.parent_id,
            node_name: row.node_name,
        };
        let context =
            read_session_context(&backend, &location)
                .await
                .map_err(|error| match error {
                    SessionContextReadError::Read(error) => error,
                    SessionContextReadError::Corrupt(reason) => invariant_query_error(reason),
                })?;
        let root = Arc::new((location, context));
        self.session_roots
            .lock()
            .insert(tree_id.to_string(), Arc::clone(&root));
        Ok(Some(root))
    }

    pub fn workspace_tree_from_folded(
        &self,
        workspace: &str,
        trees: &[FoldedExecution],
    ) -> Result<Option<WorkspaceTree>, LocalEventQueryError> {
        if trees.is_empty() {
            return Ok(None);
        }
        let mut nodes = Vec::new();
        let mut facts = Vec::new();
        for execution in trees {
            let (folded, record) = &**execution;
            nodes.extend(self.tree_nodes(workspace, folded, record)?);
            facts.push(execution_summary_fact(record));
        }
        let mut tree =
            WorkspaceTree::restore(workspace.to_string(), nodes).map_err(invariant_query_error)?;
        WorkspaceTreeProjector::project(&mut tree, facts).map_err(invariant_query_error)?;
        Ok(Some(tree))
    }
}

#[async_trait::async_trait]
impl WorkspaceTreeRepository for SqliteWorkspaceTreeRepository {
    async fn load_trees(
        &self,
        workspace_identities: &[WorkspaceIdentity],
    ) -> Vec<Result<WorkspaceTree, WorkflowError>> {
        let workspaces = workspace_identities
            .iter()
            .map(WorkspaceIdentity::as_str)
            .collect::<Vec<_>>();
        let folded = self.folded_workspaces(&workspaces).await;
        let mut trees = Vec::with_capacity(workspaces.len());
        for (workspace, folded) in workspaces.iter().zip(folded) {
            trees.push(
                match folded {
                    Ok(folded) => self.workspace_tree(workspace, &folded).await,
                    Err(error) => Err(error),
                }
                .map_err(super::query_service::query_error),
            );
        }
        trees
    }

    async fn load_node_by_session_id(
        &self,
        workspace_identity: &WorkspaceIdentity,
        session_id: &str,
    ) -> Result<Option<WorkspaceTreeNode>, LocalEventQueryError> {
        let workspace = workspace_identity.as_str().to_string();
        let backend = self.fact_backend();
        let Some((tree_id, node_execution_id)) =
            fact_log::find_session_attachment(&backend, session_id)
                .await
                .map_err(LocalEventQueryError::from)?
        else {
            return Ok(None);
        };
        let Some(execution) = self.folded_tree(&tree_id).await? else {
            return Ok(None);
        };
        let (folded, record) = &*execution;
        if folded.root.workspace_identity != workspace {
            return Ok(None);
        }
        let nodes = self.tree_nodes(&workspace, folded, record)?;
        Ok(nodes
            .iter()
            .find(|node| {
                node.node_execution_id.as_deref() == Some(node_execution_id.as_str())
                    && node.session_id.as_deref() == Some(session_id)
            })
            .map(|node| {
                let mut selected = node.clone();
                selected.id = WorkspacePublicRoot::for_node(&nodes, &node.id)
                    .map_or_else(|| node.id.clone(), |root| root.public_id().to_string());
                selected
            }))
    }

    async fn load_node(
        &self,
        workspace_identity: &WorkspaceIdentity,
        node_id: &str,
    ) -> Result<Option<WorkspaceTreeNode>, LocalEventQueryError> {
        let workspace = workspace_identity.as_str().to_string();
        let trees = self.folded_workspace_trees(&workspace).await?;
        for execution in &trees {
            let (folded, record) = &**execution;
            let nodes = self.tree_nodes(&workspace, folded, record)?;
            if folded.aggregate.id == node_id {
                if let Some(mut node) =
                    WorkspacePublicRoot::for_execution(&nodes, &folded.aggregate.id)
                        .map(|root| root.node().clone())
                {
                    node.id = node_id.to_string();
                    return Ok(Some(node));
                }
            }
            if let Some(node) = nodes.iter().find(|node| node.id == node_id).cloned() {
                return Ok(Some(node));
            }
        }
        Ok(None)
    }

    async fn load_node_by_node_execution_id(
        &self,
        node_execution_id: &str,
    ) -> Result<Option<WorkspaceTreeNode>, LocalEventQueryError> {
        let backend = self.fact_backend();
        let Some(tree_id) = backend
            .tree_id_for_node(node_execution_id)
            .await
            .map_err(fold_query_error)?
        else {
            return Ok(None);
        };
        let Some(execution) = self.folded_tree(&tree_id).await? else {
            return Ok(None);
        };
        let (folded, record) = &*execution;
        let workspace = folded.root.workspace_identity.clone();
        Ok(self
            .tree_nodes(&workspace, folded, record)?
            .into_iter()
            .find(|node| node.node_execution_id.as_deref() == Some(node_execution_id)))
    }
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

fn execution_summary_fact(execution: &WorkflowExecutionMetadataRecord) -> WorkspaceStructureFact {
    WorkspaceStructureFact::WorkflowSummaryProjected {
        execution_id: execution.execution_id.clone(),
        workflow_name: execution.workflow_name.clone(),
        status: execution.status,
        updated_at: f64::from_bits(execution.updated_at_bits),
    }
}

pub(super) fn fold_query_error(error: fact_log::FactReadError) -> LocalEventQueryError {
    error.into()
}

fn invariant_query_error(error: impl std::fmt::Display) -> LocalEventQueryError {
    let correlation_id = uuid::Uuid::new_v4().to_string();
    log::error!("Workspace indexed record invariant failure [{correlation_id}]: {error}");
    LocalEventQueryError::Corrupt { correlation_id }
}

#[cfg(test)]
#[path = "repository_test.rs"]
mod repository_tests;
