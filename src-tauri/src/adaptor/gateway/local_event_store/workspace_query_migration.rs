//! One-time v1/v2 -> v3 indexed Workspace query-record rebuild.
//!
//! Only schema evolution calls this module. Runtime queries never replay
//! events or use this access path as a fallback.

use std::collections::{BTreeMap, HashMap};

use rusqlite::{params, Connection, OptionalExtension};

use super::envelope::{DecodedStoredEvent, EventCodecRegistry};
use super::indexed_projection_codec::{
    indexed_execution_node_row, indexed_execution_row, indexed_session_public_columns,
};
use super::projection_record_codec::{
    decode_session_projection_record_v1, encode_session_projection_record_v1,
};
use crate::adaptor::gateway::workflow::execution_store::workflow_worktree_storage_key;
use crate::domain::local_event::{
    LocalDomainEvent, Revision, SessionProjectionRecord, WorkflowExecutionMetadataRecord,
    WorkflowExecutionProjectionRecord, WorkflowWorktreeOwnerRecord,
};
use crate::domain::workspace_tree::{
    recovery_reason, workflow_fact, WorkspaceSessionPublicationPolicy, WorkspaceStructureFact,
    WorkspaceTree, WorkspaceTreeProjector,
};

struct CanonicalProjection {
    key: String,
    revision: Revision,
    commit_id: String,
    record: SessionProjectionRecord,
}

#[derive(Default)]
struct WorkspaceRebuild {
    facts: Vec<WorkspaceStructureFact>,
    executions: Vec<(WorkflowExecutionMetadataRecord, Revision)>,
}

fn invalid(context: &str, error: impl std::fmt::Debug) -> rusqlite::Error {
    let correlation_id = uuid::Uuid::new_v4();
    log::error!("Workspace query-record migration {context} failed [{correlation_id}]: {error:?}");
    rusqlite::Error::InvalidQuery
}

pub(crate) fn rebuild_workspace_query_records(
    connection: &Connection,
) -> Result<(), rusqlite::Error> {
    let recovery_by_owner = recovery_reasons(connection)?;
    let projections = canonical_projections(connection)?;
    normalize_canonical_workspace_projections(connection, &projections)?;
    let mut execution_paths = execution_paths(&projections);
    let mut changes = BTreeMap::<String, WorkspaceRebuild>::new();

    replay_workflow_facts(connection, &mut execution_paths, &mut changes)?;

    for projection in &projections {
        match &projection.record {
            SessionProjectionRecord::AgentSession(session) => {
                let source_summary = WorkspaceSessionPublicationPolicy::summary(session);
                let session_reason = recovery_by_owner.get(&source_summary.id).cloned();
                let execution_recovery =
                    session.meta.workflow_node_context.as_ref().map(|context| {
                        (
                            context.execution_id.clone(),
                            recovery_by_owner.get(&context.execution_id).cloned(),
                        )
                    });
                let entry = changes
                    .entry(source_summary.worktree_path.clone())
                    .or_default();
                if let Some((_list, summary)) =
                    WorkspaceSessionPublicationPolicy::public_summary(session)
                {
                    entry
                        .facts
                        .push(WorkspaceSessionPublicationPolicy::structure_fact(
                            &summary,
                            session_reason,
                        ));
                    if let Some((owner, reason)) = execution_recovery {
                        entry
                            .facts
                            .push(WorkspaceStructureFact::RecoveryFenceProjected { owner, reason });
                    }
                } else {
                    entry.facts.push(WorkspaceStructureFact::SessionRemoved {
                        session_id: source_summary.id,
                    });
                }
            }
            SessionProjectionRecord::WorkflowExecution(
                WorkflowExecutionProjectionRecord::Present(execution),
            ) => {
                let workspace =
                    crate::domain::repository::normalize_repo_path(&execution.worktree_path);
                let entry = changes.entry(workspace.clone()).or_default();
                entry
                    .facts
                    .push(WorkspaceStructureFact::WorkflowSummaryProjected {
                        execution_id: execution.execution_id.clone(),
                        workflow_name: execution.workflow_name.clone(),
                        status: execution.status,
                        updated_at: f64::from_bits(execution.updated_at_bits),
                    });
                let mut execution = execution.clone();
                execution.worktree_path = workspace;
                entry.executions.push((execution, projection.revision));
            }
            SessionProjectionRecord::WorkflowExecution(
                WorkflowExecutionProjectionRecord::Deleted { execution_id },
            ) => {
                if let Some(path) = execution_paths.get(execution_id) {
                    changes
                        .entry(crate::domain::repository::normalize_repo_path(path))
                        .or_default()
                        .facts
                        .push(WorkspaceStructureFact::WorkflowRemoved {
                            execution_id: execution_id.clone(),
                        });
                }
            }
            SessionProjectionRecord::ProviderAgentSession(_)
            | SessionProjectionRecord::ProviderSessionOwnership(_)
            | SessionProjectionRecord::ProviderHookHealth(_)
            | SessionProjectionRecord::WorkflowWorktreeOwner(_) => {}
        }
    }

    let commit_id = latest_sealed_commit(connection)?;
    if !changes.is_empty() && commit_id.is_none() {
        return Err(invalid(
            "commit binding",
            "canonical projections exist without a sealed logical commit",
        ));
    }

    connection.execute("DELETE FROM workflow_execution_nodes", [])?;
    connection.execute("DELETE FROM workflow_executions", [])?;
    connection.execute(
        "UPDATE session_projection
         SET workspace_identity = NULL, public_list_kind = NULL,
             public_sort_key_bits = NULL, public_summary = NULL",
        [],
    )?;
    for projection in &projections {
        let public = indexed_session_public_columns(&projection.record)
            .map_err(|error| invalid("Session public columns", error))?;
        connection.execute(
            "UPDATE session_projection
             SET workspace_identity = ?1, public_list_kind = ?2,
                 public_sort_key_bits = ?3, public_summary = ?4
             WHERE session_id = ?5",
            params![
                public.workspace_identity,
                public.list_kind,
                public.sort_key_bits,
                public.summary,
                projection.key,
            ],
        )?;
    }

    for (workspace, change) in changes {
        let mut tree = WorkspaceTree::empty(&workspace);
        WorkspaceTreeProjector::project(&mut tree, change.facts)
            .map_err(|error| invalid("domain projection", error))?;
        let commit_id = commit_id
            .as_deref()
            .ok_or_else(|| invalid("commit binding", "missing sealed commit"))?;
        let revisions = change
            .executions
            .iter()
            .map(|(execution, revision)| (execution.execution_id.clone(), *revision))
            .collect::<HashMap<_, _>>();

        for (execution, revision) in change.executions {
            insert_execution(connection, commit_id, &execution, revision)?;
        }
        for node in tree
            .nodes()
            .iter()
            .filter(|node| node.execution_id.is_some())
        {
            let execution_id = node
                .execution_id
                .as_deref()
                .ok_or_else(|| invalid("node owner", &node.id))?;
            let revision = revisions
                .get(execution_id)
                .copied()
                .ok_or_else(|| invalid("node source revision", execution_id))?;
            insert_node(connection, commit_id, execution_id, node, revision)?;
        }
        verify_workspace_records(connection, &workspace, &tree)?;
    }
    Ok(())
}

fn recovery_reasons(connection: &Connection) -> Result<HashMap<String, String>, rusqlite::Error> {
    let mut reasons = HashMap::new();
    let mut statement = connection.prepare(
        "SELECT pending.owner, pending.obligation_id, obligation.record
         FROM pending_obligations AS pending
         JOIN obligations AS obligation ON obligation.obligation_id = pending.obligation_id
         ORDER BY pending.owner, pending.ordered_key",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    for row in rows {
        let (owner, obligation_id, raw) = row?;
        let record = super::state_record_codec::StoredObligationV1::decode(&raw)
            .map_err(|error| invalid("obligation decode", error))?
            .into_value();
        if let Some(reason) = recovery_reason(&obligation_id, &record) {
            reasons.entry(owner).or_insert(reason);
        }
    }
    Ok(reasons)
}

fn canonical_projections(
    connection: &Connection,
) -> Result<Vec<CanonicalProjection>, rusqlite::Error> {
    let mut projections = Vec::new();
    let mut statement = connection.prepare(
        "SELECT session_id, projection, revision, commit_id
         FROM session_projection ORDER BY session_id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;
    for row in rows {
        let (key, raw, revision, commit_id) = row?;
        projections.push(CanonicalProjection {
            record: decode_session_projection_record_v1(&raw, &key)
                .map_err(|error| invalid("canonical projection decode", error))?,
            key,
            revision: Revision::new(revision)
                .map_err(|_| invalid("canonical projection revision", revision))?,
            commit_id,
        });
    }
    Ok(projections)
}

fn normalize_canonical_workspace_projections(
    connection: &Connection,
    projections: &[CanonicalProjection],
) -> Result<(), rusqlite::Error> {
    for projection in projections {
        if matches!(
            projection.record,
            SessionProjectionRecord::WorkflowExecution(WorkflowExecutionProjectionRecord::Present(
                _
            ))
        ) {
            let encoded = encode_session_projection_record_v1(&projection.record)
                .map_err(|error| invalid("canonical execution encode", error))?;
            connection.execute(
                "UPDATE session_projection SET projection = ?1 WHERE session_id = ?2",
                params![encoded, projection.key],
            )?;
        }
    }

    let mut owners = BTreeMap::<String, Vec<&CanonicalProjection>>::new();
    for projection in projections {
        let SessionProjectionRecord::WorkflowWorktreeOwner(owner) = &projection.record else {
            continue;
        };
        let workspace = crate::domain::workspace_tree::WorkspaceIdentity::new(&owner.worktree_path);
        owners
            .entry(workspace.as_str().to_string())
            .or_default()
            .push(projection);
    }

    for (worktree_path, group) in owners {
        let winner = group
            .iter()
            .copied()
            .max_by(|left, right| {
                let SessionProjectionRecord::WorkflowWorktreeOwner(left_owner) = &left.record
                else {
                    unreachable!("owner group contains only owner records");
                };
                let SessionProjectionRecord::WorkflowWorktreeOwner(right_owner) = &right.record
                else {
                    unreachable!("owner group contains only owner records");
                };
                left_owner
                    .active
                    .cmp(&right_owner.active)
                    .then_with(|| left.revision.value().cmp(&right.revision.value()))
                    .then_with(|| left.key.cmp(&right.key))
            })
            .expect("owner group is non-empty");
        let SessionProjectionRecord::WorkflowWorktreeOwner(winner_owner) = &winner.record else {
            unreachable!("owner winner is an owner record");
        };
        let normalized =
            SessionProjectionRecord::WorkflowWorktreeOwner(WorkflowWorktreeOwnerRecord {
                worktree_path: worktree_path.clone(),
                execution_id: winner_owner.execution_id.clone(),
                active: winner_owner.active,
            });
        let encoded = encode_session_projection_record_v1(&normalized)
            .map_err(|error| invalid("canonical owner encode", error))?;
        for projection in group {
            connection.execute(
                "DELETE FROM session_projection WHERE session_id = ?1",
                [&projection.key],
            )?;
        }
        connection.execute(
            "INSERT INTO session_projection
                (session_id, projection, revision, commit_id, workspace_identity,
                 public_list_kind, public_sort_key_bits, public_summary)
             VALUES (?1, ?2, ?3, ?4, NULL, NULL, NULL, NULL)",
            params![
                workflow_worktree_storage_key(&worktree_path),
                encoded,
                winner.revision.value(),
                winner.commit_id,
            ],
        )?;
    }
    Ok(())
}

fn execution_paths(projections: &[CanonicalProjection]) -> HashMap<String, String> {
    projections
        .iter()
        .filter_map(|projection| match &projection.record {
            SessionProjectionRecord::WorkflowExecution(
                WorkflowExecutionProjectionRecord::Present(execution),
            ) => Some((
                execution.execution_id.clone(),
                crate::domain::repository::normalize_repo_path(&execution.worktree_path),
            )),
            SessionProjectionRecord::WorkflowWorktreeOwner(owner) => Some((
                owner.execution_id.clone(),
                crate::domain::repository::normalize_repo_path(&owner.worktree_path),
            )),
            _ => None,
        })
        .collect()
}

fn replay_workflow_facts(
    connection: &Connection,
    execution_paths: &mut HashMap<String, String>,
    changes: &mut BTreeMap<String, WorkspaceRebuild>,
) -> Result<(), rusqlite::Error> {
    let registry = EventCodecRegistry::new();
    let mut statement = connection.prepare(
        "SELECT event_type, payload_version, payload FROM events ORDER BY global_sequence",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, Vec<u8>>(2)?,
        ))
    })?;
    for row in rows {
        let (event_type, payload_version, payload) = row?;
        let DecodedStoredEvent::Known(event) = registry
            .decode(&event_type, payload_version, &payload)
            .map_err(|error| invalid("Workflow event decode", error))?
        else {
            continue;
        };
        let LocalDomainEvent::Workflow(event) = *event else {
            continue;
        };
        if let crate::domain::workflow::WorkflowDomainEvent::WorkflowExecutionStarted {
            execution_id,
            worktree_path,
            ..
        } = &event
        {
            execution_paths.insert(
                execution_id.clone(),
                crate::domain::repository::normalize_repo_path(worktree_path),
            );
        }
        let path = execution_paths
            .get(event.execution_id())
            .cloned()
            .ok_or_else(|| invalid("Workflow owner resolution", event.execution_id()))?;
        if let Some(fact) = workflow_fact(&event) {
            changes.entry(path).or_default().facts.push(fact);
        }
    }
    Ok(())
}

fn latest_sealed_commit(connection: &Connection) -> Result<Option<String>, rusqlite::Error> {
    connection
        .query_row(
            "SELECT commit_id FROM logical_commits
             WHERE state = 'sealed'
             ORDER BY COALESCE(last_global_sequence, 0) DESC, committed_at_ms DESC
             LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()
}

fn insert_execution(
    connection: &Connection,
    commit_id: &str,
    execution: &WorkflowExecutionMetadataRecord,
    revision: Revision,
) -> Result<(), rusqlite::Error> {
    let row = indexed_execution_row(execution).map_err(|error| invalid("execution row", error))?;
    super::commit::upsert_indexed_execution_row(connection, commit_id, revision.value(), row)?;
    Ok(())
}

fn insert_node(
    connection: &Connection,
    commit_id: &str,
    execution_id: &str,
    node: &crate::domain::workspace_tree::WorkspaceTreeNode,
    revision: Revision,
) -> Result<(), rusqlite::Error> {
    let row = indexed_execution_node_row(execution_id, node)
        .map_err(|error| invalid("node row", error))?;
    super::commit::insert_indexed_execution_node_row(connection, commit_id, revision.value(), row)?;
    Ok(())
}

fn verify_workspace_records(
    connection: &Connection,
    workspace: &str,
    expected: &WorkspaceTree,
) -> Result<(), rusqlite::Error> {
    let mut statement = connection.prepare(
        "SELECT node.tree_record
         FROM workflow_execution_nodes AS node
         JOIN workflow_executions AS execution ON execution.execution_id = node.execution_id
         WHERE execution.workspace_identity = ?1",
    )?;
    let mut restored_nodes = statement
        .query_map(params![workspace], |row| row.get::<_, String>(0))?
        .map(|row| {
            row.and_then(|raw| {
                super::indexed_projection_codec::decode_workflow_execution_node_tree_v1(&raw)
                    .map_err(|error| invalid("verification node decode", error))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    restored_nodes.sort_by(|left, right| left.id.cmp(&right.id));
    let mut expected_nodes = expected
        .nodes()
        .iter()
        .filter(|node| node.execution_id.is_some())
        .cloned()
        .collect::<Vec<_>>();
    for node in &mut expected_nodes {
        node.display_command = None;
        node.command_result = None;
    }
    expected_nodes.sort_by(|left, right| left.id.cmp(&right.id));
    if restored_nodes != expected_nodes {
        return Err(invalid(
            "order-independent verification",
            format!("rebuilt records differ for {workspace}"),
        ));
    }
    Ok(())
}
