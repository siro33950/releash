fn command_result_from_value(
    value: &serde_json::Value,
) -> Option<crate::domain::workspace_tree::WorkspaceCommandResult> {
    Some(crate::domain::workspace_tree::WorkspaceCommandResult {
        exit_code: value.get("exit_code")?.as_i64()?,
        duration: value.get("duration")?.as_u64()?,
        stdout: value.get("stdout")?.as_str()?.to_string(),
        stderr: value.get("stderr")?.as_str()?.to_string(),
    })
}

pub struct RuntimeSnapshotNodeProjection<'a> {
    pub process_presences:
        &'a std::collections::HashMap<String, crate::domain::workflow::NodeProcessPresence>,
    pub execution_id: &'a str,
    pub workflow_name: &'a str,
    pub workspace_identity: &'a str,
    pub workflow_definition: Option<&'a crate::domain::workflow::WorkflowDefinition>,
    pub node_executions:
        &'a [crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecution],
    pub retry_predecessors: &'a std::collections::HashMap<String, String>,
    pub delegate_waiting_node_ids: &'a std::collections::HashSet<String>,
    pub execution_active: bool,
    pub started_at: f64,
    pub updated_at: f64,
    pub execution: &'a crate::domain::local_event::WorkflowExecutionMetadataRecord,

    pub session_activities:
        &'a std::collections::HashMap<String, crate::domain::workflow::AgentSessionActivity>,
    pub session_display_names: &'a std::collections::HashMap<
        String,
        crate::domain::workflow::services::fact_replay::SessionDisplayNameInputs,
    >,
}

pub fn runtime_snapshot_nodes(
    input: RuntimeSnapshotNodeProjection<'_>,
) -> Result<Vec<crate::domain::workspace_tree::WorkspaceTreeNode>, String> {
    use crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecutionStatus as S;
    use crate::domain::workflow::NodeExecutionFailureKind;
    use crate::domain::workspace_tree::{
        WorkspaceStructureFact as F, WorkspaceTree, WorkspaceTreeProjector,
    };
    let RuntimeSnapshotNodeProjection {
        process_presences,
        execution_id,
        workflow_name,
        workspace_identity,
        workflow_definition,
        node_executions,
        retry_predecessors,
        delegate_waiting_node_ids,
        execution_active,
        started_at,
        updated_at,
        execution,
        session_activities,
        session_display_names,
    } = input;

    let mut facts = vec![F::WorkflowStarted {
        execution_id: execution_id.to_string(),
        workflow_name: workflow_name.to_string(),
        worktree_path: workspace_identity.to_string(),
        dynamic_fanout_names: workflow_definition
            .map(|definition| definition.dynamic_fanout_names())
            .unwrap_or_default(),
        timestamp: started_at,
    }];
    for node in node_executions
        .iter()
        .filter(|node| node.execution_id == execution_id)
    {
        facts.push(F::NodeStarted {
            execution_id: execution_id.to_string(),
            node_execution_id: node.id.clone(),
            node_name: node.node_name.clone(),
            kind: node.kind,
            attempt: node.attempt,
            parent: node.parent.clone(),
            timestamp: node.started_at,
        });
        if let Some(predecessor_node_execution_id) = retry_predecessors.get(&node.id) {
            facts.push(F::NodeRetryLinked {
                execution_id: execution_id.to_string(),
                node_execution_id: node.id.clone(),
                predecessor_node_execution_id: predecessor_node_execution_id.clone(),
            });
        }
        if let Some(session_id) = node.session_id.as_deref() {
            facts.push(F::NodeAgentBound {
                execution_id: execution_id.to_string(),
                node_execution_id: node.id.clone(),
                session_id: session_id.to_string(),
                timestamp: node.started_at,
            });
        }
        if node.kind == crate::domain::workflow::NodeKindName::Session {
            facts.push(F::NodeActivityProjected {
                execution_id: execution_id.to_string(),
                node_execution_id: node.id.clone(),
                activity: session_activities
                    .get(&node.id)
                    .copied()
                    .unwrap_or_default(),
            });
            let display_name = session_display_names
                .get(&node.id)
                .cloned()
                .unwrap_or_default();
            facts.push(F::NodeSessionDisplayNameProjected {
                execution_id: execution_id.to_string(),
                node_execution_id: node.id.clone(),
                manual_name: display_name.manual_name,
                provider_session_title: display_name.provider_session_title,
            });
        }
        if let Some(display_command) = &node.display_command {
            facts.push(F::NodeCommandPrepared {
                execution_id: execution_id.to_string(),
                node_execution_id: node.id.clone(),
                display_command: display_command.clone(),
                timestamp: node.started_at,
            });
        }
        if let Some(value) = &node.artifact {
            facts.push(F::NodeArtifactProduced {
                execution_id: execution_id.to_string(),
                node_execution_id: node.id.clone(),
                command_result_candidate: command_result_from_value(value),
                timestamp: node.completed_at.unwrap_or(updated_at),
            });
        }
        match node.status {
            S::Running => {}
            S::WaitingApproval => facts.push(F::NodeApprovalRequested {
                execution_id: execution_id.to_string(),
                node_execution_id: node.id.clone(),
                timestamp: updated_at,
            }),
            S::Succeeded => facts.push(F::NodeCompleted {
                execution_id: execution_id.to_string(),
                node_execution_id: node.id.clone(),
                timestamp: node.completed_at.unwrap_or(updated_at),
            }),
            S::Aborted => facts.push(F::NodeFailed {
                execution_id: execution_id.to_string(),
                node_execution_id: node.id.clone(),
                reason: "Workflow node aborted".to_string(),
                failure_kind: NodeExecutionFailureKind::UserAbort,
                timestamp: node.completed_at.unwrap_or(updated_at),
            }),
        }
    }
    facts.push(F::WorkflowSummaryProjected {
        execution_id: execution.execution_id.clone(),
        workflow_name: execution.workflow_name.clone(),
        status: execution.status,
        updated_at: f64::from_bits(execution.updated_at_bits),
    });
    let mut tree = WorkspaceTree::empty(workspace_identity);
    WorkspaceTreeProjector::project(&mut tree, facts).map_err(|error| error.to_string())?;
    for runtime in node_executions
        .iter()
        .filter(|runtime| runtime.execution_id == execution_id)
    {
        let Some(node) = tree.execution_node_mut(execution_id, &runtime.id) else {
            continue;
        };
        node.completion_signals = runtime.completion_signals;
        node.delegate_waits_for_child = delegate_waiting_node_ids.contains(&runtime.id);
        node.has_artifact = runtime.artifact.is_some();
        node.process_presence = process_presences
            .get(&runtime.id)
            .copied()
            .unwrap_or_default();
        node.can_retry = execution_active
            && runtime.can_retry(node.process_presence)
            && node_executions.iter().all(|candidate| {
                !same_retry_target(runtime, candidate) || candidate.attempt <= runtime.attempt
            });
        node.worktree = runtime.worktree.clone();
        node.can_resume_session = runtime.can_resume_session(node.process_presence);
    }
    tree.recompute_status_classifications();
    Ok(tree
        .nodes()
        .iter()
        .filter(|node| node.execution_id.as_deref() == Some(execution_id))
        .cloned()
        .collect())
}

fn same_retry_target(
    left: &crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecution,
    right: &crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecution,
) -> bool {
    left.node_name == right.node_name && left.parent == right.parent
}

#[cfg(test)]
#[path = "projection_test.rs"]
mod projection_tests;
