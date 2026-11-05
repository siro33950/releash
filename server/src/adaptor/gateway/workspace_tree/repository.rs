use std::collections::BTreeMap;
use std::sync::Arc;

use crate::adaptor::gateway::agent_session::{
    agent_session_from_fields, read_session_context, SessionContextReadError, SessionLocation,
};
use crate::adaptor::gateway::local_event_store::node_events::{self, NodeEventRow};
#[cfg(feature = "test-support")]
use crate::adaptor::gateway::local_event_store::store::LocalEventStore;
use crate::adaptor::gateway::workflow::fact_log::{self, FactLogReadBackend};
use crate::domain::agent_session::aggregates::AgentSession;
use crate::domain::agent_session::services::session_fields_from_facts;
use crate::domain::local_event::{LocalEventQueryError, WorkflowExecutionMetadataRecord};
use crate::domain::workflow::services::fact_replay::FoldedTree;
use crate::domain::workflow::{ExecutionTreeLaunch, WorkflowError};
use crate::domain::workspace_tree::WorkspaceExecution;
use crate::domain::workspace_tree::{
    WorkspaceIdentity, WorkspacePublicRoot, WorkspaceStructureFact, WorkspaceTree,
    WorkspaceTreeNode, WorkspaceTreeProjector, WorkspaceTreeRepository,
};

#[cfg(test)]
use super::execution_reads::fold_query_error;
pub(super) use super::execution_reads::FoldedExecution;
use super::execution_reads::{
    execution_read_error as repository_error, ExecutionReads, SessionRoot, FOLD_IDLE_LIMIT,
};

/// The only concrete `WorkspaceTreeRepository` implementation.
///
/// tree ごとに fold 済みの結果を保持し、読むときに追記された事実だけを足す。
pub struct SqliteWorkspaceTreeRepository {
    reads: Arc<ExecutionReads>,
    pub processes: Option<Arc<dyn crate::domain::workflow::NodeProcessReader>>,
    pub failures: Arc<dyn crate::domain::failure::FailureRecordRepository>,
    pub fold_idle_limit: std::time::Duration,
}

impl SqliteWorkspaceTreeRepository {
    #[cfg(feature = "test-support")]
    pub fn new(
        store: Arc<LocalEventStore>,
        failures: Arc<dyn crate::domain::failure::FailureRecordRepository>,
    ) -> Arc<Self> {
        Self::with_reads(ExecutionReads::new(store), failures)
    }

    pub fn with_reads(
        reads: Arc<ExecutionReads>,
        failures: Arc<dyn crate::domain::failure::FailureRecordRepository>,
    ) -> Arc<Self> {
        Arc::new(Self {
            reads,
            processes: None,
            failures,
            fold_idle_limit: FOLD_IDLE_LIMIT,
        })
    }

    pub(super) fn fact_backend(&self) -> FactLogReadBackend {
        self.reads.backend()
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
        self.reads
            .workspaces(workspaces, self.fold_idle_limit)
            .await
    }

    async fn folded_all(
        &self,
    ) -> Result<(Vec<FoldedExecution>, Vec<(String, LocalEventQueryError)>), LocalEventQueryError>
    {
        self.reads
            .all(self.fold_idle_limit, session_root_from_row)
            .await
    }

    /// 1 tree の fold と metadata。
    pub async fn folded_tree(
        &self,
        tree_id: &str,
    ) -> Result<Option<FoldedExecution>, LocalEventQueryError> {
        self.reads.tree(tree_id, self.fold_idle_limit).await
    }

    pub(super) fn tree_nodes(
        &self,
        workspace: &str,
        folded: &FoldedTree,
        record: &WorkflowExecutionMetadataRecord,
    ) -> Result<Vec<WorkspaceTreeNode>, LocalEventQueryError> {
        super::snapshot_nodes(workspace, folded, record, &self.processes).map_err(|error| {
            match error {
                WorkflowError::Technical(failure) => LocalEventQueryError::Technical(failure),
                WorkflowError::External(message) => {
                    LocalEventQueryError::Technical(crate::domain::failure::TechnicalFailure {
                        nature: crate::domain::failure::TechnicalFailureNature::Other,
                        message,
                    })
                }
                error => invariant_query_error(error),
            }
        })
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
            executions.push(self.workspace_execution(folded, record).await?);
        }
        tree.record_executions(executions);
        self.observe_failures(&mut tree);
        Ok(tree)
    }

    async fn workspace_execution(
        &self,
        folded: &FoldedTree,
        record: &WorkflowExecutionMetadataRecord,
    ) -> Result<WorkspaceExecution, LocalEventQueryError> {
        Ok(WorkspaceExecution {
            execution_id: record.execution_id.clone(),
            launched_as: folded.root.launched_as,
            worktree_path: record.worktree_path.clone(),
            workflow_name: record.workflow_name.clone(),
            status: record.status,
            updated_at: f64::from_bits(record.updated_at_bits),
            archive: folded.aggregate.archive_record(),
            session: self.execution_session(folded).await?,
        })
    }

    fn observe_failures(&self, tree: &mut WorkspaceTree) {
        for (target, message) in super::background_failures(tree.nodes(), &*self.failures) {
            tree.observe_background_failure(&target, &message);
        }
    }

    /// Session として起動した実行木の session を、保持している事実の導出から復元する。
    async fn execution_session(
        &self,
        folded: &FoldedTree,
    ) -> Result<Option<AgentSession>, LocalEventQueryError> {
        let Some((session_id, fields)) = self.execution_session_fields(folded).await? else {
            return Ok(None);
        };
        agent_session_from_fields(&session_id, fields)
            .map(Some)
            .map_err(|error| invariant_query_error(format!("{error:?}")))
    }

    pub(super) async fn execution_session_fields(
        &self,
        folded: &FoldedTree,
    ) -> Result<
        Option<(
            String,
            crate::domain::agent_session::services::DerivedAgentSessionFields,
        )>,
        LocalEventQueryError,
    > {
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
        Ok(Some((session.session_id.clone(), fields)))
    }

    async fn session_root(
        &self,
        tree_id: &str,
    ) -> Result<Option<SessionRoot>, LocalEventQueryError> {
        if let Some(root) = self.reads.session_root(tree_id).await {
            return Ok(Some(Arc::clone(&root)));
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
        self.reads
            .record_session_root(tree_id, Arc::clone(&root))
            .await;
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
    async fn load_all_trees(
        &self,
    ) -> Result<(Vec<WorkspaceTree>, Vec<(String, WorkflowError)>), WorkflowError> {
        let (folded, read_failures) = self.folded_all().await.map_err(repository_error)?;
        let mut failures = read_failures
            .into_iter()
            .map(|(id, error)| (id, repository_error(error)))
            .collect::<Vec<_>>();
        let mut workspaces = BTreeMap::<
            String,
            (
                Vec<WorkspaceTreeNode>,
                Vec<WorkspaceExecution>,
                Vec<WorkspaceStructureFact>,
            ),
        >::new();
        for execution in folded {
            let (folded, record) = &*execution;
            let workspace = &folded.root.workspace_identity;
            let result = async {
                let nodes = self.tree_nodes(workspace, folded, record)?;
                let execution = self.workspace_execution(folded, record).await?;
                Ok::<_, LocalEventQueryError>((nodes, execution))
            }
            .await;
            match result {
                Ok((nodes, execution)) => {
                    let entry = workspaces.entry(workspace.clone()).or_default();
                    entry.0.extend(nodes);
                    entry.1.push(execution);
                    entry.2.push(execution_summary_fact(record));
                }
                Err(error) => failures.push((record.execution_id.clone(), repository_error(error))),
            }
        }
        let mut trees = Vec::with_capacity(workspaces.len());
        for (workspace, (nodes, executions, facts)) in workspaces {
            let mut tree = WorkspaceTree::restore(workspace, nodes)
                .map_err(invariant_query_error)
                .map_err(repository_error)?;
            WorkspaceTreeProjector::project(&mut tree, facts)
                .map_err(invariant_query_error)
                .map_err(repository_error)?;
            tree.record_executions(executions);
            self.observe_failures(&mut tree);
            trees.push(tree);
        }
        Ok((trees, failures))
    }

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
                .map_err(repository_error),
            );
        }
        trees
    }

    async fn load_execution_tree(
        &self,
        workspace_identity: &WorkspaceIdentity,
        execution_id: &str,
    ) -> Result<Option<WorkspaceTree>, crate::domain::workflow::WorkflowError> {
        let Some(execution) = self
            .folded_tree(execution_id)
            .await
            .map_err(repository_error)?
        else {
            return Ok(None);
        };
        if execution.0.root.workspace_identity != workspace_identity.as_str() {
            return Ok(None);
        }
        self.workspace_tree(workspace_identity.as_str(), &[execution])
            .await
            .map(Some)
            .map_err(repository_error)
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
        let workspace = workspace_identity.as_str();
        let trees = self.folded_workspace_trees(workspace).await?;
        for execution in &trees {
            let (folded, record) = &**execution;
            let nodes = self.tree_nodes(workspace, folded, record)?;
            if let Some(node) = WorkspacePublicRoot::resolve_node(&nodes, node_id) {
                return Ok(Some(node));
            }
        }
        Ok(None)
    }
}

fn session_root_from_row(row: &NodeEventRow) -> Result<SessionRoot, LocalEventQueryError> {
    let meta = fact_log::node_meta_from_row(row).map_err(invariant_query_error)?;
    let location = SessionLocation::from_meta(&meta);
    let context = crate::adaptor::gateway::agent_session::session_facts::session_context_from_row(
        row, &location,
    )
    .map_err(|error| match error {
        SessionContextReadError::Read(error) => error,
        SessionContextReadError::Corrupt(reason) => invariant_query_error(reason),
    })?;
    Ok(Arc::new((location, context)))
}

fn execution_summary_fact(execution: &WorkflowExecutionMetadataRecord) -> WorkspaceStructureFact {
    WorkspaceStructureFact::WorkflowSummaryProjected {
        execution_id: execution.execution_id.clone(),
        workflow_name: execution.workflow_name.clone(),
        status: execution.status,
        updated_at: f64::from_bits(execution.updated_at_bits),
    }
}

fn invariant_query_error(error: impl std::fmt::Display) -> LocalEventQueryError {
    let correlation_id = uuid::Uuid::new_v4().to_string();
    log::error!("Workspace indexed record invariant failure [{correlation_id}]: {error}");
    LocalEventQueryError::Corrupt { correlation_id }
}

#[cfg(test)]
#[path = "repository_test.rs"]
mod repository_tests;
