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

pub struct SqliteWorkspaceQueryService {
    repository: Arc<SqliteWorkspaceTreeRepository>,
    references: Arc<parking_lot::Mutex<ReferenceIndex>>,
    clocks: Arc<parking_lot::Mutex<std::collections::HashMap<String, SessionClock>>>,
}

#[derive(Default)]
struct ReferenceIndex {
    workspace: Option<String>,
    trees: std::collections::HashMap<String, ReferenceTree>,
    aliases: std::collections::HashMap<String, (String, String)>,
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
                                if root.worktree_path.trim().is_empty() {
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
                            if !header.worktree_path.trim().is_empty() {
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

#[derive(Default)]
struct SessionClock {
    root: Option<(String, i64, [u8; 32])>,
    workspace: String,
    state: crate::domain::workflow::services::node_read::NodeReadState,
    timestamp_ms: i64,
    without_definition: bool,
}

struct SessionFactSource<'a> {
    connection: &'a rusqlite::Connection,
    tree: &'a str,
    root: &'a crate::domain::workflow::TreeRootFact,
    selection: &'a (String, Vec<String>),
    terminal: Option<&'a crate::domain::workflow::NodeFactRecord>,
    retries: &'a std::collections::HashMap<String, String>,
}

impl SessionFactSource<'_> {
    fn read_scope(
        &self,
        roots: &[String],
    ) -> Result<
        (
            Vec<crate::domain::workflow::NodeFactRecord>,
            crate::domain::workflow::services::node_read::NodeReadProjectionResult,
        ),
        crate::domain::local_event::LocalEventQueryError,
    > {
        use crate::adaptor::gateway::{
            local_event_store::{node_events, reader},
            workflow::fact_log,
        };
        let records = node_events::read_subtrees(
            self.connection,
            self.tree,
            roots,
            &self.selection.0,
            &self.selection.1,
            |row| fact_log::record_from_row(&row),
        )
        .map_err(|error| reader::storage_unavailable(&error))?
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| reader::corrupt(&error))?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        let projection = crate::domain::workflow::services::node_read::subtree_read_projection(
            self.root,
            &records,
            self.terminal,
            self.retries,
            None,
        )
        .map_err(|error| reader::corrupt(&error))?;
        Ok((records, projection))
    }

    fn artifact_value(&self, seq: i64) -> Result<serde_json::Value, String> {
        let text = crate::adaptor::gateway::local_event_store::node_events::artifact_value(
            self.connection,
            self.tree,
            seq,
        )
        .map_err(|error| error.to_string())?;
        serde_json::from_str(&text).map_err(|error| error.to_string())
    }

    fn records_after(
        &self,
        seq: i64,
    ) -> Result<
        Vec<crate::domain::workflow::NodeFactRecord>,
        crate::domain::local_event::LocalEventQueryError,
    > {
        use crate::adaptor::gateway::{
            local_event_store::{node_events, reader},
            workflow::fact_log,
        };
        let through = self.terminal.map(|record| record.seq).unwrap_or(i64::MAX);
        node_events::projected_tree_after(
            self.connection,
            self.tree,
            seq,
            &self.selection.0,
            &self.selection.1,
            |row| fact_log::record_from_row(&row),
        )
        .map_err(|error| reader::storage_unavailable(&error))?
        .into_iter()
        .filter_map(Result::transpose)
        .take_while(|record| {
            record.as_ref().is_err() || record.as_ref().is_ok_and(|record| record.seq <= through)
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| reader::corrupt(&error))
    }

    fn execution_updated_at(
        &self,
        held: &mut SessionClock,
        root_row: &crate::adaptor::gateway::local_event_store::node_events::NodeEventRow,
    ) -> Result<i64, crate::domain::local_event::LocalEventQueryError> {
        use crate::adaptor::gateway::local_event_store::{node_events, reader};
        let head = node_events::reference_heads(
            self.connection,
            &std::collections::HashSet::from([self.tree.to_string()]),
        )
        .map_err(|error| reader::storage_unavailable(&error))?
        .first()
        .map(|(_, seq)| *seq)
        .unwrap_or_default();
        use sha2::Digest;
        let root = (
            root_row.node_execution_id.clone(),
            root_row.timestamp_ms,
            sha2::Sha256::digest(root_row.detail.as_bytes()).into(),
        );
        if held.root.as_ref() != Some(&root) || head < held.state.seq {
            *held = SessionClock {
                root: Some(root),
                workspace: self.root.worktree_path.clone(),
                ..Default::default()
            };
        }
        if head == held.state.seq {
            return Ok(held.timestamp_ms);
        }
        let records = self.records_after(held.state.seq)?;
        let root = if held.without_definition {
            self.root.without_definition()
        } else {
            self.root.clone()
        };
        let result = crate::domain::workflow::services::node_read::update_read_state(
            &mut held.state,
            &root,
            &records,
            self.retries,
            &mut |seq| self.artifact_value(seq),
        );
        let result = match result {
            Err(_)
                if self.terminal.is_some()
                    && !held.without_definition
                    && self.root.definition.is_some() =>
            {
                held.state = Default::default();
                held.without_definition = true;
                let records = self.records_after(0)?;
                crate::domain::workflow::services::node_read::update_read_state(
                    &mut held.state,
                    &self.root.without_definition(),
                    &records,
                    self.retries,
                    &mut |seq| self.artifact_value(seq),
                )
            }
            result => result,
        }
        .map_err(|error| reader::corrupt(&error))?;
        held.state.seq = head;
        held.timestamp_ms = result.timestamp_ms;
        Ok(held.timestamp_ms)
    }
}

impl SqliteWorkspaceQueryService {
    pub fn with_repository(repository: Arc<SqliteWorkspaceTreeRepository>) -> Arc<Self> {
        Arc::new(Self {
            repository,
            references: Default::default(),
            clocks: Default::default(),
        })
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
        let rows = self
            .repository
            .fact_backend()
            .run_indexed(move |connection| {
                let unavailable = |error: rusqlite::Error| {
                    crate::adaptor::gateway::local_event_store::reader::storage_unavailable(&error)
                };
                let transaction = connection.unchecked_transaction().map_err(unavailable)?;
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
                    let owner = direct.map(|row| row.tree_id).or(
                        crate::domain::workspace_tree::entities::public_node_tree(id),
                    );
                    let owner = match owner {
                        Some(owner) => Some(owner),
                        None => node_events::first_row_of_tree(&transaction, id)
                            .map_err(unavailable)?
                            .map(|row| row.tree_id),
                    };
                    if let Some(owner) = owner {
                        selected.insert(owner.clone());
                        owners.insert(id.clone(), owner);
                    } else {
                        needs_workspace = true;
                    }
                }
                if needs_workspace {
                    let standalone: Vec<_> = ids
                        .iter()
                        .filter(|id| crate::domain::workspace_tree::entities::is_standalone_reference(id))
                        .collect();
                    if !standalone.is_empty() {
                        for (tree, name) in node_events::corrupt_standalone_roots(&transaction)
                            .map_err(unavailable)?
                        {
                            for id in standalone.iter().copied() {
                                if crate::domain::workspace_tree::entities::standalone_reference_matches(&tree, &name, id) {
                                    selected.insert(tree.clone());
                                    owners.insert(id.clone(), tree.clone());
                                }
                            }
                        }
                    }
                    selected.extend(
                        node_events::workspace_reference_trees(&transaction, &workspace)
                            .map_err(unavailable)?,
                    );
                }
                let mut index = index.lock();
                if index.workspace.as_deref() != Some(&workspace) {
                    *index = ReferenceIndex {
                        workspace: Some(workspace.clone()),
                        ..Default::default()
                    };
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
                        Err(error) => result
                            .failures
                            .push(crate::domain::failure::WorkFailure::from_error(&error)),
                    }
                }
            }
            Err(error) => result
                .failures
                .push(crate::domain::failure::WorkFailure::from_error(
                    &query_error(error),
                )),
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

    async fn session_node_detail(
        &self,
        workspace_identity: &WorkspaceIdentity,
        tree_id: &str,
        node_execution_id: &str,
    ) -> Result<
        Option<crate::usecase::workspace_tree::query_service::SessionNodeDetailDto>,
        WorkflowError,
    > {
        use crate::adaptor::gateway::workflow::fact_log;
        use crate::domain::workflow::{NodeFact, NodeKind, NodeKindName};
        use crate::domain::workspace_tree::WorkspaceNodeClassification;
        let backend = self.repository.fact_backend();
        let (tree, target) = (tree_id.to_string(), node_execution_id.to_string());
        let workspace = workspace_identity.as_str().to_string();
        let clocks = Arc::clone(&self.clocks);
        let read = backend
            .run_indexed(move |connection| {
                use crate::adaptor::gateway::local_event_store::{node_events, reader};
                let unavailable = |error: rusqlite::Error| reader::storage_unavailable(&error);
                let transaction = connection.unchecked_transaction().map_err(unavailable)?;
                let connection = &*transaction;
                let Some(root_row) =
                    node_events::first_root_row_of_tree(connection, &tree, "started")
                        .map_err(unavailable)?
                else {
                    clocks.lock().remove(&tree);
                    return Ok(None);
                };
                let Some(root_record) = fact_log::record_from_row(&root_row)
                    .map_err(|error| reader::corrupt(&error))?
                else {
                    return Ok(None);
                };
                let NodeFact::Started(started) = &root_record.fact else {
                    return Ok(None);
                };
                let Some(root) = started
                    .root
                    .as_deref()
                    .filter(|root| root.worktree_path == workspace)
                else {
                    return Ok(None);
                };
                let selection = session_artifact_projection(
                    crate::domain::workflow::services::node_read::artifact_fields(root),
                );
                let lineage = node_events::tree_rows_with_event_types(
                    connection,
                    &tree,
                    &["started", "retry_requested"],
                )
                .map_err(unavailable)?
                .iter()
                .filter_map(|row| fact_log::record_from_row(row).transpose())
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| reader::corrupt(&error))?;
                let retries =
                    crate::domain::workflow::services::history::retry_predecessors(&lineage);
                let roots = crate::domain::workflow::services::history::read_lineage_roots(
                    &target, &retries,
                );
                let terminal = node_events::tree_rows_with_event_types(
                    connection,
                    &tree,
                    &["execution_completed", "abort_requested"],
                )
                .map_err(unavailable)?
                .into_iter()
                .next()
                .map(|row| fact_log::record_from_row(&row))
                .transpose()
                .map_err(|error| reader::corrupt(&error))?
                .flatten();
                let source = SessionFactSource {
                    connection,
                    tree: &tree,
                    root,
                    selection: &selection,
                    terminal: terminal.as_ref(),
                    retries: &retries,
                };
                let (records, projection) = source.read_scope(&roots)?;
                let mut clocks = clocks.lock();
                let tracked = clocks.keys().cloned().collect();
                let live: std::collections::HashSet<_> =
                    node_events::reference_heads(connection, &tracked)
                        .map_err(unavailable)?
                        .into_iter()
                        .map(|(id, _)| id)
                        .collect();
                clocks.retain(|id, held| live.contains(id) && held.workspace == workspace);
                let updated_at = source
                    .execution_updated_at(clocks.entry(tree.clone()).or_default(), &root_row)?;
                let rules = projection.0;
                let starts = lineage;
                Ok(Some((
                    root_record,
                    records,
                    starts,
                    terminal,
                    updated_at,
                    rules,
                )))
            })
            .await
            .map_err(query_error)?;
        let Some((root_record, records, starts, terminal, execution_updated_at, rules)) = read
        else {
            return Ok(None);
        };
        let NodeFact::Started(started) = &root_record.fact else {
            return Ok(None);
        };
        let Some(root) = started.root.as_deref() else {
            return Ok(None);
        };
        let identities = crate::domain::workspace_tree::entities::node_identities(&starts)
            .map_err(|error| WorkflowError::CorruptStoredState(error.to_string()))?;
        let Some(first) = records.iter().find(|record| {
            record.meta.node_execution_id == node_execution_id
                && matches!(record.fact, NodeFact::Started(_))
        }) else {
            return Ok(None);
        };
        if first.meta.kind != NodeKindName::Session {
            return Ok(None);
        }
        let execution_status =
            crate::domain::workflow::ExecutionStatus::from_terminal_fact(terminal.as_ref());
        let mut target_session = None;
        let mut classification = Vec::new();
        for (id, rules) in &rules {
            let Some(start) = records
                .iter()
                .find(|record| &record.meta.node_execution_id == id)
            else {
                continue;
            };
            let session_id = records.iter().find_map(|record| match &record.fact {
                NodeFact::SessionAttached(attached) if record.meta.node_execution_id == *id => {
                    Some(attached.session_id.as_str())
                }
                _ => None,
            });
            let session = crate::domain::workflow::services::fact_replay::derive_session_facts(
                &records,
                id,
                session_id.unwrap_or_default(),
            );
            let presence = self
                .repository
                .processes
                .as_ref()
                .map(|processes| {
                    processes.presence(workspace_identity.as_str(), id, start.meta.kind, session_id)
                })
                .transpose()?
                .unwrap_or_default();
            let activity = session.activity;
            if id == node_execution_id {
                target_session = Some((session_id.map(str::to_owned), session));
            }
            classification.push(WorkspaceNodeClassification {
                id: identities[id].clone(),
                parent_id: rules
                    .parent_id
                    .as_ref()
                    .and_then(|parent| identities.get(parent))
                    .cloned(),
                execution_id: Some(tree_id.into()),
                node_execution_id: Some(id.clone()),
                kind: start.meta.kind.into(),
                status: rules.status.into(),
                process_presence: presence,
                delegate_waits_for_child: rules.delegate_waits_for_child,
                background_failure: false,
                activity: (start.meta.kind == NodeKindName::Session).then_some(activity),
                has_session: session_id.is_some(),
                completion_signals: rules.completion_signals,
                hidden: rules.hidden,
            });
        }
        let Some(state) = classification
            .iter()
            .find(|node| node.node_execution_id.as_deref() == Some(node_execution_id))
        else {
            return Ok(None);
        };
        let (session_id, session) =
            target_session.expect("target classification has session facts");
        let title = crate::domain::workspace_tree::value_objects::session_node_title(
            session.manual_name,
            session.provider_session_title,
            first.meta.node_name.clone(),
            node_execution_id == tree_id,
        );
        let definition = root
            .definition
            .as_ref()
            .and_then(|definition| definition.node_by_name(&first.meta.node_name));
        let model = definition.and_then(|definition| match &definition.kind {
            NodeKind::Session(spec) => spec.model.clone(),
            _ => None,
        });
        let (submit_received, stop_received, waiting_for) =
            signal_display(state.completion_signals);
        let rules = &rules[node_execution_id];
        let updated_at = crate::domain::workspace_tree::value_objects::node_updated_at(
            first.timestamp_ms as f64 / 1000.0,
            rules.completed_at,
            rules.status,
            rules.has_artifact,
            execution_updated_at as f64 / 1000.0,
        );
        let error_reason = crate::domain::workspace_tree::value_objects::node_failure_reason(
            crate::domain::workspace_tree::value_objects::aborted_node_reason(rules.status),
            None,
        );
        let detail = WorkspaceNodeDetailDto {
            classification: WorkspaceNodeClassification::aggregate(&classification, &state.id)
                .as_public_str()
                .into(),
            node_execution_id: Some(node_execution_id.into()),
            node_name: Some(first.meta.node_name.clone()),
            model,
            past_attempts: Vec::new(),
            execution_id: Some(tree_id.into()),
            can_abort: execution_status.can_abort(),
            process_presence: state.process_presence.as_str(),
            worktree: records.iter().find_map(|record| match &record.fact {
                NodeFact::Started(started)
                    if record.meta.node_execution_id == node_execution_id =>
                {
                    started.worktree.as_ref().map(|worktree| {
                        crate::usecase::workflow::NodeWorktreeDto {
                            branch: worktree.branch.clone(),
                            path: worktree.path.clone(),
                        }
                    })
                }
                _ => None,
            }),
            id: identities[node_execution_id].clone(),
            title,
            status: state.status.as_public_str().into(),
            submit_received,
            stop_received,
            waiting_for,
            has_artifact: rules.has_artifact,
            error_reason,
            capabilities: WorkspaceNodeCapabilitiesDto {
                can_rename: crate::domain::workspace_tree::value_objects::session_can_rename(
                    WorkspaceNodeKind::WorkflowSession,
                    session_id.is_some(),
                    crate::domain::workspace_tree::entities::public_root_node(&starts)
                        == Some(node_execution_id),
                    node_execution_id == tree_id,
                ),
                can_approve: state.status.can_approve(),
                can_retry: rules.status.can_retry_in_execution(
                    execution_status.is_active(),
                    NodeKindName::Session,
                    state.process_presence,
                    || true,
                ),
                can_resume_session: state
                    .process_presence
                    .can_resume_session(NodeKindName::Session),
            },
            updated_at,
            content: WorkspaceNodeContentDto::Session(WorkspaceSessionNodeContentDto {
                session_id,
            }),
        };
        Ok(Some(
            crate::usecase::workspace_tree::query_service::SessionNodeDetailDto {
                detail,
                classification,
            },
        ))
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

fn session_artifact_projection(fields: Vec<String>) -> (String, Vec<String>) {
    let mut parameters = Vec::new();
    let mut inputs = Vec::new();
    for field in fields {
        let path = format!(
            "$.value.{}",
            field
                .split('.')
                .map(|segment| serde_json::to_string(segment).expect("string"))
                .collect::<Vec<_>>()
                .join(".")
        );
        let index = parameters.len() + 3;
        parameters.extend([field, path]);
        inputs.push(format!("?{index}, json(CASE json_type(detail, ?{}) WHEN 'true' THEN 'true' WHEN 'false' THEN 'false' WHEN 'text' THEN json_quote(json_extract(detail, ?{})) WHEN 'array' THEN CAST(json_array_length(detail, ?{}) AS TEXT) ELSE 'null' END)", index + 1, index + 1, index + 1));
    }
    // Project predicate inputs before decoding; artifact bodies stay in the event store.
    (format!("CASE WHEN event_type = 'artifact_produced' AND json_valid(detail) AND json_type(detail, '$.value') IS NOT NULL THEN json_object('value', json_object({}), 'contract', json_extract(detail, '$.contract'), 'requestId', json_extract(detail, '$.requestId')) ELSE detail END", inputs.join(", ")), parameters)
}

fn record_projection_error(context: &str) -> WorkflowError {
    let correlation_id = uuid::Uuid::new_v4();
    log::error!("Workspace query record invariant failure [{correlation_id}]: {context}");
    WorkflowError::CorruptStoredState(format!("{context} (correlation_id={correlation_id})"))
}

#[cfg(test)]
#[path = "query_service_test.rs"]
mod query_service_tests;
