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
}

impl SqliteWorkspaceQueryService {
    pub fn with_repository(repository: Arc<SqliteWorkspaceTreeRepository>) -> Arc<Self> {
        Arc::new(Self { repository })
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
        use crate::adaptor::gateway::workflow::fact_log;
        let mut result =
            crate::usecase::workspace_tree::query_service::SessionNodeResolutionDto::default();
        let workspace = workspace_identity.as_str().to_string();
        let ids = node_ids
            .iter()
            .cloned()
            .collect::<std::collections::HashSet<_>>();
        let rows = self.repository.fact_backend().run_indexed(move |connection| {
            let unavailable = |error: rusqlite::Error| crate::adaptor::gateway::local_event_store::reader::storage_unavailable(&error);
            let transaction = connection.unchecked_transaction().map_err(unavailable)?;
            let mut rows = Vec::new();
            for root in node_events::list_tree_roots(&transaction, "started").map_err(unavailable)? {
                match crate::adaptor::gateway::workflow::stored_definition::read_tree_header(&root.detail) {
                    Ok(Some(header)) if header.worktree_path == workspace => {}
                    Ok(_) => continue,
                    Err(error) => { rows.push((None, Err(WorkflowError::CorruptStoredState(error)))); continue; }
                }
                let tree = root.tree_id;
                let starts = node_events::tree_rows_with_event_types(&transaction, &tree, &["started"]).map_err(unavailable)?;
                let resolved = (|| {
                    let records = starts.iter().filter_map(|row| fact_log::record_from_row(row).transpose()).collect::<Result<Vec<_>, _>>().map_err(WorkflowError::CorruptStoredState)?;
                    let identities = crate::domain::workspace_tree::entities::node_identities(&records).map_err(|error| WorkflowError::CorruptStoredState(error.to_string()))?;
                    let public = records.iter().find(|record| matches!(&record.fact, crate::domain::workflow::NodeFact::Started(started) if started.parent.is_none()));
                    Ok::<_, WorkflowError>(records.iter().filter(|record| record.meta.kind == crate::domain::workflow::NodeKindName::Session).flat_map(|record| {
                        let meta = &record.meta;
                        let node_id = &identities[&meta.node_execution_id];
                        let public_id = public.filter(|first| first.meta.node_execution_id == meta.node_execution_id).map(|_| tree.as_str());
                        ids.iter().filter(move |id| *id == node_id || id.as_str() == meta.node_execution_id || Some(id.as_str()) == public_id)
                            .map(move |id| (id.clone(), meta.node_execution_id.clone()))
                    }).collect::<Vec<_>>())
                })();
                match resolved {
                    Ok(nodes) => for (id, node) in nodes {
                        let attachment = node_events::latest_row_for_node_with_event_types(&transaction, &node, &["session_attached"]).map_err(unavailable)?;
                        rows.push((Some(id), attachment.map(|row| fact_log::record_from_row(&row).map_err(WorkflowError::CorruptStoredState)).transpose().map(|record| record.flatten())));
                    },
                    Err(error) => rows.push((None, Err(error))),
                }
            }
            transaction.commit().map_err(unavailable)?;
            Ok(rows)
        }).await;
        match rows {
            Ok(rows) => {
                for (id, record) in rows {
                    match record {
                        Ok(Some(crate::domain::workflow::NodeFactRecord {
                            fact: crate::domain::workflow::NodeFact::SessionAttached(attached),
                            ..
                        })) => {
                            result
                                .sessions
                                .insert(id.expect("resolved reference"), attached.session_id);
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
        use crate::domain::workflow::{NodeExecutionStatus, NodeFact, NodeKind, NodeKindName};
        use crate::domain::workspace_tree::{WorkspaceNodeClassification, WorkspaceNodeStatus};
        let backend = self.repository.fact_backend();
        let (tree, target) = (tree_id.to_string(), node_execution_id.to_string());
        let workspace = workspace_identity.as_str().to_string();
        let read = backend
            .run_indexed(move |connection| {
                use crate::adaptor::gateway::local_event_store::{node_events, reader};
                let unavailable = |error: rusqlite::Error| reader::storage_unavailable(&error);
                let Some(root_row) =
                    node_events::first_root_row_of_tree(connection, &tree, "started")
                        .map_err(unavailable)?
                else {
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
                let (detail_projection, parameters) = session_artifact_projection(
                    crate::domain::workflow::services::node_read::artifact_fields(root),
                );
                let records = node_events::read_subtree(
                    connection,
                    &tree,
                    &target,
                    &detail_projection,
                    &parameters,
                    |row| fact_log::record_from_row(&row),
                )
                .map_err(unavailable)?
                .into_iter()
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| reader::corrupt(&error))?
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
                let starts =
                    node_events::tree_rows_with_event_types(connection, &tree, &["started"])
                        .map_err(unavailable)?;
                let terminal = node_events::tree_rows_with_event_types(
                    connection,
                    &tree,
                    &["execution_completed", "abort_requested"],
                )
                .map_err(unavailable)?
                .into_iter()
                .next();
                Ok(Some((root_record, records, starts, terminal)))
            })
            .await
            .map_err(query_error)?;
        let Some((root_record, records, starts, terminal)) = read else {
            return Ok(None);
        };
        let NodeFact::Started(started) = &root_record.fact else {
            return Ok(None);
        };
        let Some(root) = started.root.as_deref() else {
            return Ok(None);
        };
        let starts = starts
            .iter()
            .filter_map(|row| fact_log::record_from_row(row).transpose())
            .collect::<Result<Vec<_>, _>>()
            .map_err(WorkflowError::CorruptStoredState)?;
        let identities = crate::domain::workspace_tree::entities::node_identities(&starts)
            .map_err(|error| WorkflowError::CorruptStoredState(error.to_string()))?;
        let terminal = terminal
            .as_ref()
            .map(fact_log::record_from_row)
            .transpose()
            .map_err(WorkflowError::CorruptStoredState)?
            .flatten();
        let Some(first) = records.iter().find(|record| {
            record.meta.node_execution_id == node_execution_id
                && matches!(record.fact, NodeFact::Started(_))
        }) else {
            return Ok(None);
        };
        if first.meta.kind != NodeKindName::Session {
            return Ok(None);
        }
        let rules = crate::domain::workflow::services::node_read::subtree_read_rules(
            root,
            &records,
            terminal.as_ref(),
        )
        .map_err(WorkflowError::CorruptStoredState)?;
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
            classification.push(WorkspaceNodeClassification {
                id: identities[id].clone(),
                parent_id: start
                    .meta
                    .parent_id
                    .as_ref()
                    .and_then(|parent| identities.get(parent))
                    .cloned(),
                execution_id: Some(tree_id.into()),
                node_execution_id: Some(id.clone()),
                kind: match start.meta.kind {
                    NodeKindName::Session => WorkspaceNodeKind::WorkflowSession,
                    NodeKindName::Command => WorkspaceNodeKind::WorkflowCommand,
                    NodeKindName::Fanout => WorkspaceNodeKind::Fanout,
                    NodeKindName::Sequence => WorkspaceNodeKind::Sequence,
                },
                status: match rules.status {
                    NodeExecutionStatus::Running => WorkspaceNodeStatus::Running,
                    NodeExecutionStatus::WaitingApproval => WorkspaceNodeStatus::Waiting,
                    NodeExecutionStatus::Succeeded => WorkspaceNodeStatus::Completed,
                    NodeExecutionStatus::Aborted => WorkspaceNodeStatus::Aborted,
                },
                process_presence: presence,
                delegate_waits_for_child: rules.delegate_waits_for_child,
                background_failure: false,
                activity: (start.meta.kind == NodeKindName::Session).then_some(session.activity),
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
        let session_id = records.iter().find_map(|record| match &record.fact {
            NodeFact::SessionAttached(attached)
                if record.meta.node_execution_id == node_execution_id =>
            {
                Some(attached.session_id.clone())
            }
            _ => None,
        });
        let session = crate::domain::workflow::services::fact_replay::derive_session_facts(
            &records,
            node_execution_id,
            session_id.as_deref().unwrap_or_default(),
        );
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
        let signals = state.completion_signals;
        let updated_at = records
            .iter()
            .filter(|record| record.meta.node_execution_id == node_execution_id)
            .filter(|record| {
                matches!(
                    record.fact,
                    NodeFact::Started(_)
                        | NodeFact::SessionAttached(_)
                        | NodeFact::ArtifactProduced(_)
                        | NodeFact::ApprovalGranted(_)
                        | NodeFact::StandaloneSessionNodeCompleted
                        | NodeFact::ExecutionCompleted
                        | NodeFact::AbortRequested(_)
                )
            })
            .map(|record| record.timestamp_ms)
            .max()
            .unwrap_or(first.timestamp_ms) as f64
            / 1000.0;
        let error_reason = records.iter().rev().find_map(|record| match &record.fact {
            NodeFact::RuntimeFailureObserved(failure)
                if record.meta.node_execution_id == node_execution_id =>
            {
                Some(failure.reason.clone())
            }
            _ => None,
        });
        let rules = &rules[node_execution_id];
        let detail = WorkspaceNodeDetailDto {
            classification: WorkspaceNodeClassification::aggregate(&classification, &state.id)
                .as_public_str()
                .into(),
            node_execution_id: Some(node_execution_id.into()),
            node_name: Some(first.meta.node_name.clone()),
            model,
            past_attempts: Vec::new(),
            execution_id: Some(tree_id.into()),
            can_abort: terminal.is_none(),
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
            submit_received: matches!(
                signals,
                crate::domain::workflow::NodeCompletionSignalState::SubmitReceived
                    | crate::domain::workflow::NodeCompletionSignalState::Ready
            ),
            stop_received: matches!(
                signals,
                crate::domain::workflow::NodeCompletionSignalState::StopReceived
                    | crate::domain::workflow::NodeCompletionSignalState::Ready
            ),
            waiting_for: match signals {
                crate::domain::workflow::NodeCompletionSignalState::SubmitReceived => Some("stop"),
                crate::domain::workflow::NodeCompletionSignalState::StopReceived => Some("submit"),
                _ => None,
            },
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
                can_approve: state.status == WorkspaceNodeStatus::Waiting,
                can_retry: terminal.is_none()
                    && rules
                        .status
                        .can_retry(NodeKindName::Session, state.process_presence),
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
    let submit_received = matches!(
        node.completion_signals,
        crate::domain::workflow::NodeCompletionSignalState::SubmitReceived
            | crate::domain::workflow::NodeCompletionSignalState::Ready
    );
    let stop_received = matches!(
        node.completion_signals,
        crate::domain::workflow::NodeCompletionSignalState::StopReceived
            | crate::domain::workflow::NodeCompletionSignalState::Ready
    );
    let waiting_for = match node.completion_signals {
        crate::domain::workflow::NodeCompletionSignalState::SubmitReceived => Some("stop"),
        crate::domain::workflow::NodeCompletionSignalState::StopReceived => Some("submit"),
        crate::domain::workflow::NodeCompletionSignalState::Pending
        | crate::domain::workflow::NodeCompletionSignalState::Ready => None,
    };
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
