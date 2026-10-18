use std::sync::Arc;

use super::SqliteWorkspaceTreeRepository;
use crate::domain::workflow::{
    ExecutionStatusFilter, ExecutionTreeLaunch, WorkflowError, WorkflowExecutionSummary,
};
use crate::domain::workspace_tree::{
    WorkspaceIdentity, WorkspaceNodeKind, WorkspacePublicRoot, WorkspaceTreeNode,
};
use crate::usecase::workflow::{
    WorkspaceCommandNodeContentDto, WorkspaceCommandResultDto, WorkspaceNodeCapabilitiesDto,
    WorkspaceNodeContentDto, WorkspaceNodeDetailDto, WorkspaceSessionNodeContentDto,
};
use crate::usecase::workspace_tree::WorkspaceQueryService;

#[cfg(feature = "test-support")]
type ReferenceSearchObserver = Arc<dyn Fn() + Send + Sync>;

pub struct SqliteWorkspaceQueryService {
    repository: Arc<SqliteWorkspaceTreeRepository>,
    #[cfg(feature = "test-support")]
    reference_search_observer: parking_lot::Mutex<Option<ReferenceSearchObserver>>,
    references: Arc<parking_lot::Mutex<ReferenceIndex>>,
}

#[derive(Default)]
struct ReferenceIndex {
    workspace: Option<String>,
    trees: std::collections::HashMap<String, ReferenceTree>,
    aliases: std::collections::HashMap<String, (String, String)>,
    standalone: StandaloneReferences,
}

#[derive(Default)]
pub struct StandaloneReferences {
    heads: std::collections::HashMap<String, i64>,
    aliases: std::collections::HashMap<String, String>,
}

impl StandaloneReferences {
    pub fn owner(&self, alias: &str) -> Option<&str> {
        self.aliases.get(alias).map(String::as_str)
    }

    pub fn update(&mut self, connection: &rusqlite::Connection) -> Result<(), rusqlite::Error> {
        use crate::adaptor::gateway::local_event_store::node_events;
        let heads: std::collections::HashMap<_, _> =
            node_events::tree_heads(connection)?.into_iter().collect();
        self.aliases.retain(|_, tree| {
            heads.get(tree).is_some_and(|head| {
                self.heads
                    .get(tree)
                    .is_some_and(|previous| head >= previous)
            })
        });
        for (tree, head) in &heads {
            let after = self
                .heads
                .get(tree)
                .copied()
                .filter(|previous| previous <= head)
                .unwrap_or_default();
            if *head == after {
                continue;
            }
            for name in node_events::standalone_roots_after(connection, tree, after)? {
                let alias =
                    crate::domain::workspace_tree::entities::standalone_reference(tree, &name);
                self.aliases.insert(alias, tree.clone());
            }
        }
        self.heads = heads;
        Ok(())
    }
}

impl ReferenceIndex {
    fn update(
        &mut self,
        connection: &rusqlite::Connection,
        heads: &[(String, i64)],
        workspace: &str,
    ) -> Result<(), rusqlite::Error> {
        use crate::adaptor::gateway::{local_event_store::node_events, workflow::fact_log};
        let live: std::collections::HashSet<_> = heads.iter().map(|(id, _)| id.as_str()).collect();
        self.trees.retain(|id, _| live.contains(id.as_str()));
        self.aliases
            .retain(|_, (tree, _)| live.contains(tree.as_str()));
        for (tree, head) in heads {
            let held = self.trees.entry(tree.clone()).or_default();
            if *head < held.seq {
                *held = ReferenceTree::default();
                self.aliases.retain(|_, (owner, _)| owner != tree);
            }
            if *head == held.seq {
                continue;
            }
            for row in node_events::reference_starts_after(connection, tree, held.seq)? {
                let result = fact_log::record_from_row(&row).and_then(|record| {
                    if let Some(record) = record {
                        if let crate::domain::workflow::NodeFact::Started(started) = &record.fact {
                            if let Some(root) = &started.root {
                                if !crate::domain::workflow::TreeRootFact::valid_worktree_path(
                                    &root.worktree_path,
                                ) {
                                    return Err(format!(
                                        "tree {} has an empty root workspace",
                                        row.tree_id
                                    ));
                                }
                                held.workspace = Some(root.worktree_path.clone());
                            } else if held.workspace.is_none() && record.meta.parent_id.is_none() {
                                return Err(format!("tree {} has no root header", row.tree_id));
                            }
                        }
                        if held
                            .workspace
                            .as_deref()
                            .is_some_and(|owner| owner != workspace)
                        {
                            return Ok(());
                        }
                        if held.errors.is_empty() {
                            held.ids
                                .record(&record)
                                .map_err(|error| error.to_string())?;
                        }
                    }
                    Ok(())
                });
                if held
                    .workspace
                    .as_deref()
                    .is_some_and(|owner| owner != workspace)
                {
                    break;
                }
                if let Err(error) = result {
                    held.errors
                        .insert(row.node_execution_id.clone(), error.clone());
                    if row.parent_id.is_none() {
                        if let Ok(Some(header)) =
                            crate::adaptor::gateway::workflow::stored_definition::read_tree_header(
                                &row.detail,
                            )
                        {
                            if crate::domain::workflow::TreeRootFact::valid_worktree_path(
                                &header.worktree_path,
                            ) {
                                held.workspace = Some(header.worktree_path);
                            }
                        }
                        if held
                            .workspace
                            .as_deref()
                            .is_some_and(|owner| owner != workspace)
                        {
                            break;
                        }
                        if let Err(reason) = fact_log::node_meta_from_row(&row).and_then(|meta| {
                            held.ids
                                .record_start_identity(&meta, None)
                                .map_err(|error| error.to_string())
                        }) {
                            held.errors.insert(
                                row.node_execution_id.clone(),
                                format!("{error}: {reason}"),
                            );
                        }
                    }
                }
                if let Some(alias) = held.ids.identities.get(&row.node_execution_id) {
                    self.aliases
                        .insert(alias.clone(), (tree.clone(), row.node_execution_id));
                }
            }
            held.seq = *head;
        }
        self.trees.retain(|_, held| {
            held.workspace
                .as_deref()
                .is_none_or(|owner| owner == workspace)
        });
        self.aliases
            .retain(|_, (tree, _)| self.trees.contains_key(tree));
        Ok(())
    }

    fn resolve(
        &self,
        connection: &rusqlite::Connection,
        workspace: &str,
        id: &str,
        owner: Option<&str>,
    ) -> Result<Option<crate::domain::workflow::NodeFactRecord>, WorkflowError> {
        use crate::adaptor::gateway::{local_event_store::node_events, workflow::fact_log};
        let storage = |error| {
            WorkflowError::from(fact_log::FactReadError::Query(
                crate::adaptor::gateway::local_event_store::reader::storage_unavailable(&error),
            ))
        };
        let direct =
            node_events::latest_row_for_node_with_event_types(connection, id, &["started"])
                .map_err(storage)?;
        let direct = match direct {
            Some(row) => Some(row),
            None => node_events::first_row_of_tree(connection, id).map_err(storage)?,
        };
        let matched = direct
            .map(|row| (row.tree_id, row.node_execution_id))
            .or_else(|| self.aliases.get(id).cloned());
        let tree = matched.as_ref().map(|(tree, _)| tree.as_str()).or(owner);
        let Some(tree) = tree else { return Ok(None) };
        let Some(held) = self.trees.get(tree) else {
            return Ok(None);
        };
        if let Some(owner) = held.workspace.as_deref() {
            if owner != workspace {
                return Ok(None);
            }
        } else if let Some(error) = held.errors.values().next() {
            return Err(WorkflowError::CorruptStoredState(error.clone()));
        } else {
            return Ok(None);
        }
        let node = if tree == id {
            held.ids.public_root().map(str::to_string)
        } else {
            matched.as_ref().map(|(_, node)| node.clone())
        };
        let Some(node) = node else {
            if let Some(error) = held.errors.values().next() {
                return Err(WorkflowError::CorruptStoredState(error.clone()));
            }
            return Ok(None);
        };
        if let Some(error) = held.errors.get(&node) {
            return Err(WorkflowError::CorruptStoredState(error.clone()));
        }
        let Some(start) =
            node_events::latest_row_for_node_with_event_types(connection, &node, &["started"])
                .map_err(storage)?
        else {
            return Ok(None);
        };
        let meta =
            fact_log::node_meta_from_row(&start).map_err(WorkflowError::CorruptStoredState)?;
        if meta.kind != crate::domain::workflow::NodeKindName::Session {
            return Ok(None);
        }
        if tree == id {
            if let Some(error) = held.errors.values().next() {
                return Err(WorkflowError::CorruptStoredState(error.clone()));
            }
        }
        node_events::latest_row_for_node_with_event_types(connection, &node, &["session_attached"])
            .map_err(storage)?
            .map(|row| fact_log::record_from_row(&row).map_err(WorkflowError::CorruptStoredState))
            .transpose()
            .map(Option::flatten)
    }
}

#[derive(Default)]
struct ReferenceTree {
    seq: i64,
    workspace: Option<String>,
    ids: crate::domain::workspace_tree::entities::NodeIdentityIndex,
    errors: std::collections::HashMap<String, String>,
}

impl SqliteWorkspaceQueryService {
    pub fn with_repository(repository: Arc<SqliteWorkspaceTreeRepository>) -> Arc<Self> {
        Arc::new(Self {
            repository,
            references: Default::default(),
            #[cfg(feature = "test-support")]
            reference_search_observer: Default::default(),
        })
    }

    #[cfg(feature = "test-support")]
    pub fn set_reference_search_observer(&self, observer: Option<ReferenceSearchObserver>) {
        *self.reference_search_observer.lock() = observer;
    }

    pub async fn execution_records(
        &self,
        workspace_identity: Option<&WorkspaceIdentity>,
        status: Option<ExecutionStatusFilter>,
    ) -> Result<Vec<crate::domain::local_event::WorkflowExecutionMetadataRecord>, WorkflowError>
    {
        let tree_roots = self.repository.tree_roots().await.map_err(|error| {
            WorkflowError::from(
                crate::adaptor::gateway::workflow::fact_log::FactReadError::Query(error),
            )
        })?;
        let mut records = Vec::new();
        for (tree_id, root) in tree_roots {
            if root.launched_as != ExecutionTreeLaunch::Workflow
                || workspace_identity.is_some_and(|wanted| wanted.as_str() != root.worktree_path)
            {
                continue;
            }
            let Some(execution) = self
                .repository
                .folded_tree(&tree_id)
                .await
                .map_err(query_error)?
            else {
                continue;
            };
            let (folded, record) = &*execution;
            debug_assert_eq!(folded.root.launched_as, ExecutionTreeLaunch::Workflow);
            let keep = match status {
                Some(ExecutionStatusFilter::Active) => !record.status.is_finished(),
                None => true,
            };
            if keep {
                records.push(record.clone());
            }
        }
        // 旧一覧と同じ並び: active が先、次に更新時刻の新しい順、最後に id。
        records.sort_by(|left, right| {
            left.status
                .is_finished()
                .cmp(&right.status.is_finished())
                .then_with(|| {
                    f64::from_bits(right.updated_at_bits)
                        .total_cmp(&f64::from_bits(left.updated_at_bits))
                })
                .then_with(|| left.execution_id.cmp(&right.execution_id))
        });
        Ok(records)
    }
}

#[async_trait::async_trait]
impl WorkspaceQueryService for SqliteWorkspaceQueryService {
    async fn resolve_session_nodes(
        &self,
        workspace_identity: &WorkspaceIdentity,
        node_ids: &[String],
    ) -> crate::usecase::workspace_tree::query_service::SessionNodeResolutionDto {
        use crate::adaptor::gateway::local_event_store::node_events;
        let mut result =
            crate::usecase::workspace_tree::query_service::SessionNodeResolutionDto::default();
        let workspace = workspace_identity.as_str().to_string();
        let ids = node_ids.to_vec();
        let index = Arc::clone(&self.references);
        #[cfg(feature = "test-support")]
        let observer = self.reference_search_observer.lock().clone();
        let rows = self
            .repository
            .fact_backend()
            .run_indexed(move |connection| {
                let unavailable = |error: rusqlite::Error| {
                    crate::adaptor::gateway::local_event_store::reader::storage_unavailable(&error)
                };
                let transaction = connection.unchecked_transaction().map_err(unavailable)?;
                let mut index = index.lock();
                if index.workspace.as_deref() != Some(&workspace) {
                    let standalone = std::mem::take(&mut index.standalone);
                    *index = ReferenceIndex {
                        workspace: Some(workspace.clone()),
                        standalone,
                        ..Default::default()
                    };
                }
                if ids
                    .iter()
                    .any(|id| crate::domain::workspace_tree::entities::is_standalone_reference(id))
                {
                    index.standalone.update(&transaction).map_err(unavailable)?;
                }
                let mut selected = std::collections::HashSet::new();
                let mut owners = std::collections::HashMap::new();
                let mut needs_workspace = false;
                for id in &ids {
                    let direct = node_events::latest_row_for_node_with_event_types(
                        &transaction,
                        id,
                        &["started"],
                    )
                    .map_err(unavailable)?;
                    let owner = direct
                        .map(|row| row.tree_id)
                        .or_else(|| index.standalone.owner(id).map(str::to_string))
                        .or(crate::domain::workspace_tree::entities::public_node_tree(
                            id,
                        ));
                    let owner = match owner {
                        Some(owner) => Some(owner),
                        None => node_events::first_row_of_tree(&transaction, id)
                            .map_err(unavailable)?
                            .map(|row| row.tree_id),
                    };
                    if let Some(owner) = owner {
                        selected.insert(owner.clone());
                        owners.insert(id.clone(), owner);
                    } else if !crate::domain::workspace_tree::entities::is_standalone_reference(id)
                    {
                        needs_workspace = true;
                    }
                }
                if needs_workspace {
                    #[cfg(feature = "test-support")]
                    if let Some(observer) = &observer {
                        observer();
                    }
                    selected.extend(
                        node_events::workspace_reference_trees(&transaction, &workspace)
                            .map_err(unavailable)?,
                    );
                }
                selected.extend(index.trees.keys().cloned());
                let heads =
                    node_events::reference_heads(&transaction, &selected).map_err(unavailable)?;
                index
                    .update(&transaction, &heads, &workspace)
                    .map_err(unavailable)?;
                let rows = ids
                    .into_iter()
                    .map(|id| {
                        let resolved = index.resolve(
                            &transaction,
                            &workspace,
                            &id,
                            owners.get(&id).map(String::as_str),
                        );
                        (id, resolved)
                    })
                    .collect::<Vec<_>>();
                transaction.commit().map_err(unavailable)?;
                Ok(rows)
            })
            .await;
        match rows {
            Ok(rows) => {
                for (id, record) in rows {
                    match record {
                        Ok(Some(crate::domain::workflow::NodeFactRecord {
                            fact: crate::domain::workflow::NodeFact::SessionAttached(attached),
                            ..
                        })) => {
                            result.sessions.insert(id, attached.session_id);
                        }
                        Ok(_) => {}
                        Err(error) => result.failures.push(crate::usecase::workspace_tree::query_service::SessionNodeResolutionFailureDto {
                            reference_id: id,
                            failure: crate::domain::failure::WorkFailure::from_error(&error),
                        }),
                    }
                }
            }
            Err(error) => {
                let failure = crate::domain::failure::WorkFailure::from_error(&query_error(error));
                result.failures.extend(node_ids.iter().map(|id| {
                    crate::usecase::workspace_tree::query_service::SessionNodeResolutionFailureDto {
                        reference_id: id.clone(),
                        failure: failure.clone(),
                    }
                }));
            }
        }
        result
    }

    async fn node_detail(
        &self,
        workspace_identity: &WorkspaceIdentity,
        node_id: &str,
    ) -> Result<Option<WorkspaceNodeDetailDto>, WorkflowError> {
        let workspace = workspace_identity.as_str();
        let trees = self
            .repository
            .folded_workspace_trees(workspace)
            .await
            .map_err(query_error)?;
        for execution in trees {
            let (folded, record) = &*execution;
            let nodes = self
                .repository
                .tree_nodes(workspace, folded, record)
                .map_err(query_error)?;
            let node = WorkspacePublicRoot::resolve_node(&nodes, node_id);
            let Some(node) = node else { continue };
            let attempts = node
                .past_attempt_ids
                .iter()
                .filter_map(|id| nodes.iter().find(|attempt| &attempt.id == id))
                .map(
                    |attempt| crate::usecase::workflow::WorkspaceNodeAttemptDto {
                        id: attempt.id.clone(),
                        title: attempt.title.clone(),
                        updated_at: attempt.updated_at(),
                        error_reason: attempt.failure_reason(),
                    },
                )
                .collect();
            let workflow = nodes.iter().find(|node| {
                node.kind == WorkspaceNodeKind::Workflow && node.id == folded.aggregate.id
            });
            let pending_session = (node.kind == WorkspaceNodeKind::WorkflowSession
                && node.session_id.is_none())
            .then(|| node.node_execution_id.clone())
            .flatten();
            let mut detail = node_detail_with_workflow(node, workflow);
            if let Some(id) = pending_session {
                if let Some(record) = crate::adaptor::gateway::workflow::fact_log::read_latest_record_for_node_with_event_types(
                    &self.repository.fact_backend(), &id, &["runtime_failure_observed"],
                ).await.map_err(WorkflowError::from)? {
                    if let crate::domain::workflow::NodeFact::RuntimeFailureObserved(failure) = record.fact {
                        detail.error_reason = Some(failure.reason);
                    }
                }
            }
            detail.past_attempts = attempts;
            return Ok(Some(detail));
        }
        Ok(None)
    }

    async fn node_artifact(
        &self,
        node_execution_id: &str,
    ) -> Result<crate::usecase::workflow::WorkflowGetOutputResult, WorkflowError> {
        use crate::domain::workflow::NodeFact;
        use crate::usecase::workflow::WorkflowGetOutputResult;
        let record = crate::adaptor::gateway::workflow::fact_log::read_latest_record_for_node_with_event_types(
            &self.repository.fact_backend(), node_execution_id, &["artifact_produced"],
        ).await.map_err(WorkflowError::from)?;
        Ok(match record {
            Some(record) => match record.fact {
                NodeFact::ArtifactProduced(artifact) => WorkflowGetOutputResult::Submitted {
                    contract: artifact.contract,
                    structured_output: artifact.value,
                    submitted_at: Some(record.timestamp_ms as f64 / 1000.0),
                    request_id: artifact.request_id,
                    timestamp: record.timestamp_ms as f64 / 1000.0,
                },
                _ => {
                    return Err(WorkflowError::CorruptStoredState(
                        "Expected ArtifactProduced fact".into(),
                    ))
                }
            },
            None => WorkflowGetOutputResult::NotSubmitted,
        })
    }

    async fn execution_summaries(
        &self,
        workspace_identity: Option<&WorkspaceIdentity>,
        status: Option<ExecutionStatusFilter>,
    ) -> Result<Vec<WorkflowExecutionSummary>, WorkflowError> {
        self.execution_records(workspace_identity, status)
            .await?
            .into_iter()
            .map(execution_summary)
            .collect()
    }
}

fn node_detail_with_workflow(
    node: WorkspaceTreeNode,
    workflow: Option<&WorkspaceTreeNode>,
) -> WorkspaceNodeDetailDto {
    node_detail(node, workflow.is_some_and(|workflow| workflow.can_abort))
}

pub fn node_detail(node: WorkspaceTreeNode, can_abort: bool) -> WorkspaceNodeDetailDto {
    let updated_at = node.updated_at();
    let error_reason = node.failure_reason();
    let (submit_received, stop_received, waiting_for) = signal_display(node.completion_signals);
    let content = match node.kind {
        WorkspaceNodeKind::WorkflowCommand => {
            WorkspaceNodeContentDto::Command(WorkspaceCommandNodeContentDto {
                display_command: node.display_command,
                shell: node.shell,
                cwd: node.cwd,
                result: node.command_result.map(|result| WorkspaceCommandResultDto {
                    exit_code: result.exit_code,
                    duration: result.duration,
                    stdout: result.stdout,
                    stderr: result.stderr,
                }),
            })
        }
        _ => WorkspaceNodeContentDto::Session(WorkspaceSessionNodeContentDto {
            session_id: node.session_id,
        }),
    };
    WorkspaceNodeDetailDto {
        classification: node.status_classification.as_public_str().into(),
        node_execution_id: node.node_execution_id,
        node_name: node.node_name,
        model: node.model,
        past_attempts: Vec::new(),
        execution_id: node.execution_id,
        can_abort,
        process_presence: node.process_presence.as_str(),
        worktree: node
            .worktree
            .map(|worktree| crate::usecase::workflow::NodeWorktreeDto {
                branch: worktree.branch,
                path: worktree.path,
            }),
        id: node.id,
        title: node.title,
        status: node.status.as_public_str().to_string(),
        submit_received,
        stop_received,
        waiting_for,
        has_artifact: node.has_artifact,
        error_reason,
        capabilities: WorkspaceNodeCapabilitiesDto {
            can_rename: node.can_rename,
            can_approve: node.can_approve,
            can_retry: node.can_retry,
            can_resume_session: node.can_resume_session,
        },
        updated_at,
        content,
    }
}

fn signal_display(
    signals: crate::domain::workflow::NodeCompletionSignalState,
) -> (bool, bool, Option<&'static str>) {
    let submit_received = matches!(
        signals,
        crate::domain::workflow::NodeCompletionSignalState::SubmitReceived
            | crate::domain::workflow::NodeCompletionSignalState::Ready
    );
    let stop_received = matches!(
        signals,
        crate::domain::workflow::NodeCompletionSignalState::StopReceived
            | crate::domain::workflow::NodeCompletionSignalState::Ready
    );
    let waiting_for = match signals {
        crate::domain::workflow::NodeCompletionSignalState::SubmitReceived => Some("stop"),
        crate::domain::workflow::NodeCompletionSignalState::StopReceived => Some("submit"),
        crate::domain::workflow::NodeCompletionSignalState::Pending
        | crate::domain::workflow::NodeCompletionSignalState::Ready => None,
    };
    (submit_received, stop_received, waiting_for)
}

pub fn execution_summary(
    record: crate::domain::local_event::WorkflowExecutionMetadataRecord,
) -> Result<WorkflowExecutionSummary, WorkflowError> {
    let started_at = f64::from_bits(record.started_at_bits);
    let updated_at = f64::from_bits(record.updated_at_bits);
    let completed_at = record.completed_at_bits.map(f64::from_bits);
    if !started_at.is_finite()
        || !updated_at.is_finite()
        || completed_at.is_some_and(|value| !value.is_finite())
    {
        return Err(record_projection_error(
            "Workspace execution summary contains an invalid timestamp",
        ));
    }
    Ok(WorkflowExecutionSummary {
        execution_id: record.execution_id,
        workflow_name: record.workflow_name,
        status: record.status,
        worktree_path: record.worktree_path,
        current_node: record.current_node,
        created_from: record.created_from,
        started_at,
        updated_at,
        completed_at,
        error_reason: record.error_reason,
        total_token_usage: record.total_token_usage,
    })
}

pub(super) fn query_error(
    error: crate::domain::local_event::LocalEventQueryError,
) -> WorkflowError {
    use crate::domain::local_event::LocalEventQueryError;

    match error {
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

fn record_projection_error(context: &str) -> WorkflowError {
    let correlation_id = uuid::Uuid::new_v4();
    log::error!("Workspace query record invariant failure [{correlation_id}]: {context}");
    WorkflowError::CorruptStoredState(format!("{context} (correlation_id={correlation_id})"))
}

#[cfg(test)]
#[path = "query_service_test.rs"]
mod query_service_tests;
