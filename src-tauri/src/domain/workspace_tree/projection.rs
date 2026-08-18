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

pub fn workflow_fact(
    event: &crate::domain::workflow::WorkflowDomainEvent,
) -> Option<crate::domain::workspace_tree::WorkspaceStructureFact> {
    use crate::domain::workflow::WorkflowDomainEvent as E;
    use crate::domain::workspace_tree::WorkspaceStructureFact as F;

    Some(match event {
        E::WorkflowExecutionStarted {
            execution_id,
            workflow_name,
            worktree_path,
            definition,
            timestamp,
            ..
        } => F::WorkflowStarted {
            execution_id: execution_id.clone(),
            workflow_name: workflow_name.clone(),
            worktree_path: worktree_path.clone(),
            definition: definition.clone(),
            timestamp: *timestamp,
        },
        E::NodeExecutionStarted {
            execution_id,
            node_execution_id,
            node_name,
            kind,
            attempt,
            parent,
            timestamp,
        } => F::NodeStarted {
            execution_id: execution_id.clone(),
            node_execution_id: node_execution_id.clone(),
            node_name: node_name.clone(),
            kind: *kind,
            attempt: *attempt,
            parent: parent.clone(),
            timestamp: *timestamp,
        },
        E::NodeExecutionAgentBound {
            execution_id,
            node_execution_id,
            session_id,
            timestamp,
        } => F::NodeAgentBound {
            execution_id: execution_id.clone(),
            node_execution_id: node_execution_id.clone(),
            session_id: session_id.clone(),
            timestamp: *timestamp,
        },
        E::NodeExecutionSubmitReceived {
            execution_id,
            node_execution_id,
            timestamp,
        } => F::NodeSubmitReceived {
            execution_id: execution_id.clone(),
            node_execution_id: node_execution_id.clone(),
            timestamp: *timestamp,
        },
        E::NodeExecutionStopReceived {
            execution_id,
            node_execution_id,
            timestamp,
        } => F::NodeStopReceived {
            execution_id: execution_id.clone(),
            node_execution_id: node_execution_id.clone(),
            timestamp: *timestamp,
        },
        E::NodeExecutionPaused {
            execution_id,
            node_execution_id,
            timestamp,
        } => F::NodePaused {
            execution_id: execution_id.clone(),
            node_execution_id: node_execution_id.clone(),
            timestamp: *timestamp,
        },
        E::NodeExecutionResumed {
            execution_id,
            node_execution_id,
            timestamp,
        } => F::NodeResumed {
            execution_id: execution_id.clone(),
            node_execution_id: node_execution_id.clone(),
            timestamp: *timestamp,
        },
        E::NodeExecutionCommandPrepared {
            execution_id,
            node_execution_id,
            display_command,
            timestamp,
        } => F::NodeCommandPrepared {
            execution_id: execution_id.clone(),
            node_execution_id: node_execution_id.clone(),
            display_command: display_command.clone(),
            timestamp: *timestamp,
        },
        E::WorkflowArtifactProduced {
            execution_id,
            node_execution_id,
            value,
            timestamp,
            ..
        } => {
            let result = serde_json::from_str::<serde_json::Value>(value.as_str())
                .ok()
                .and_then(|value| command_result_from_value(&value));
            F::NodeArtifactProduced {
                execution_id: execution_id.clone(),
                node_execution_id: node_execution_id.clone(),
                result,
                timestamp: *timestamp,
            }
        }
        E::NodeExecutionCompleted {
            execution_id,
            node_execution_id,
            timestamp,
            ..
        } => F::NodeCompleted {
            execution_id: execution_id.clone(),
            node_execution_id: node_execution_id.clone(),
            timestamp: *timestamp,
        },
        E::NodeExecutionFailed {
            execution_id,
            node_execution_id,
            reason,
            failure_kind,
            timestamp,
            ..
        } => F::NodeFailed {
            execution_id: execution_id.clone(),
            node_execution_id: node_execution_id.clone(),
            reason: reason.clone(),
            failure_kind: *failure_kind,
            timestamp: *timestamp,
        },
        E::WorkflowApprovalRequested {
            execution_id,
            node_execution_id,
            timestamp,
            ..
        } => F::NodeApprovalRequested {
            execution_id: execution_id.clone(),
            node_execution_id: node_execution_id.clone(),
            timestamp: *timestamp,
        },
        E::WorkflowApprovalResolved {
            execution_id,
            node_execution_id,
            timestamp,
            ..
        } => F::NodeApprovalResolved {
            execution_id: execution_id.clone(),
            node_execution_id: node_execution_id.clone(),
            timestamp: *timestamp,
        },
        _ => return None,
    })
}

pub struct RuntimeSnapshotNodeProjection<'a> {
    pub execution_id: &'a str,
    pub workflow_name: &'a str,
    pub workspace_identity: &'a str,
    pub workflow_definition: &'a crate::domain::workflow::WorkflowDefinition,
    pub node_executions:
        &'a [crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecution],
    pub started_at: f64,
    pub updated_at: f64,
    pub execution: &'a crate::domain::local_event::WorkflowExecutionMetadataRecord,
    pub recovery_owner_reason: Option<String>,
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
        execution_id,
        workflow_name,
        workspace_identity,
        workflow_definition,
        node_executions,
        started_at,
        updated_at,
        execution,
        recovery_owner_reason,
    } = input;

    let mut facts = vec![F::WorkflowStarted {
        execution_id: execution_id.to_string(),
        workflow_name: workflow_name.to_string(),
        worktree_path: workspace_identity.to_string(),
        definition: workflow_definition.clone(),
        timestamp: started_at,
    }];
    facts.push(F::RecoveryFenceProjected {
        owner: execution_id.to_string(),
        reason: recovery_owner_reason,
    });
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
        if let Some(session_id) = &node.session_id {
            facts.push(F::NodeAgentBound {
                execution_id: execution_id.to_string(),
                node_execution_id: node.id.clone(),
                session_id: session_id.clone(),
                timestamp: node.started_at,
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
                result: command_result_from_value(value),
                timestamp: node.completed_at.unwrap_or(updated_at),
            });
        }
        match node.status {
            S::Running | S::Paused => {}
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
            S::Failed | S::Aborted => facts.push(F::NodeFailed {
                execution_id: execution_id.to_string(),
                node_execution_id: node.id.clone(),
                reason: node
                    .failure
                    .as_ref()
                    .map(|failure| failure.reason.clone())
                    .unwrap_or_else(|| {
                        if node.status == S::Aborted {
                            "Workflow node aborted".to_string()
                        } else {
                            "Workflow node failed".to_string()
                        }
                    }),
                failure_kind: node.failure.as_ref().map(|failure| failure.kind).unwrap_or(
                    if node.status == S::Aborted {
                        NodeExecutionFailureKind::UserAbort
                    } else {
                        NodeExecutionFailureKind::InfrastructureCrash
                    },
                ),
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
    let mut projected = tree
        .nodes()
        .iter()
        .filter(|node| node.execution_id.as_deref() == Some(execution_id))
        .cloned()
        .collect::<Vec<_>>();
    for node in &mut projected {
        let Some(node_execution_id) = node.node_execution_id.as_deref() else {
            continue;
        };
        let Some(runtime) = node_executions
            .iter()
            .find(|runtime| runtime.id == node_execution_id)
        else {
            continue;
        };
        node.completion_signals = runtime.completion_signals;
        node.has_artifact = runtime.artifact.is_some();
        node.can_retry = runtime.can_retry()
            && node_executions.iter().all(|candidate| {
                !same_retry_target(runtime, candidate) || candidate.attempt <= runtime.attempt
            });
        if runtime.status == S::Paused {
            node.status = crate::domain::workspace_tree::WorkspaceNodeStatus::Paused;
        }
    }
    Ok(projected)
}

fn same_retry_target(
    left: &crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecution,
    right: &crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecution,
) -> bool {
    left.node_name == right.node_name && left.parent == right.parent
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::local_event::WorkflowExecutionMetadataRecord;
    use crate::domain::workflow::entities::workflow_execution::{
        RuntimeNodeExecution, RuntimeNodeExecutionFailure, RuntimeNodeExecutionStatus,
    };
    use crate::domain::workflow::{
        ExecutionOrigin, ExecutionStatus, NodeCompletionSignalState, NodeDefinition,
        NodeExecutionFailureKind, NodeKindName, TokenUsage, WorkflowDefinition,
    };
    use crate::domain::workspace_tree::WorkspaceNodeStatus;

    const EXECUTION_ID: &str = "00000000-0000-4000-8000-000000000901";
    const OTHER_EXECUTION_ID: &str = "00000000-0000-4000-8000-000000000902";

    fn node(
        id: &str,
        execution_id: &str,
        status: RuntimeNodeExecutionStatus,
    ) -> RuntimeNodeExecution {
        RuntimeNodeExecution {
            id: id.to_string(),
            execution_id: execution_id.to_string(),
            node_name: "test".to_string(),
            kind: NodeKindName::Command,
            attempt: 1,
            status,
            session_id: None,
            display_command: Some("cargo test".to_string()),
            artifact: None,
            token_usage: None,
            failure: None,
            parent: None,
            completion_signals: NodeCompletionSignalState::Pending,
            started_at: 2.0,
            completed_at: None,
        }
    }

    fn execution() -> WorkflowExecutionMetadataRecord {
        WorkflowExecutionMetadataRecord {
            execution_id: EXECUTION_ID.to_string(),
            workflow_name: "workflow".to_string(),
            status: ExecutionStatus::Running,
            worktree_path: "/repo".to_string(),
            current_node: Some("test".to_string()),
            created_from: ExecutionOrigin::DesktopUi,
            started_at_bits: 1.0f64.to_bits(),
            updated_at_bits: 10.0f64.to_bits(),
            completed_at_bits: None,
            error_reason: None,
            interruption_reason: None,
            resume_from_node: None,
            total_token_usage: TokenUsage::default(),
        }
    }

    #[test]
    fn same_retry_target_requires_matching_name_and_parent_scope() {
        use crate::domain::workflow::ExecutionParentRef;

        let base = node("left", EXECUTION_ID, RuntimeNodeExecutionStatus::Failed);
        let mut same_lane = node("right", EXECUTION_ID, RuntimeNodeExecutionStatus::Running);
        assert!(same_retry_target(&base, &same_lane));

        // 別名は別ターゲット。
        let mut other_name = same_lane.clone();
        other_name.node_name = "other".to_string();
        assert!(!same_retry_target(&base, &other_name));

        // 同名でも親スコープ（lane）が違えば別ターゲット。
        same_lane.parent = Some(ExecutionParentRef::sequence_child("part-lane-1"));
        let mut other_lane = same_lane.clone();
        other_lane.parent = Some(ExecutionParentRef::sequence_child("part-lane-2"));
        assert!(!same_retry_target(&same_lane, &other_lane));

        let mut peer = same_lane.clone();
        peer.parent = Some(ExecutionParentRef::sequence_child("part-lane-1"));
        assert!(same_retry_target(&same_lane, &peer));
    }

    #[test]
    fn runtime_snapshot_nodes_uses_bounded_defaults_and_filters_other_executions() {
        let mut failed = node("failed", EXECUTION_ID, RuntimeNodeExecutionStatus::Failed);
        failed.artifact = Some(serde_json::json!({"unexpected": true}));
        let completed = node(
            "completed",
            EXECUTION_ID,
            RuntimeNodeExecutionStatus::Succeeded,
        );
        let unrelated = node(
            "unrelated",
            OTHER_EXECUTION_ID,
            RuntimeNodeExecutionStatus::Running,
        );
        let definition = WorkflowDefinition {
            name: "workflow".to_string(),
            nodes: vec![NodeDefinition {
                name: "test".to_string(),
                ..NodeDefinition::default()
            }],
            ..WorkflowDefinition::default()
        };

        let execution = execution();
        let node_executions = [failed, completed, unrelated];
        let nodes = runtime_snapshot_nodes(RuntimeSnapshotNodeProjection {
            execution_id: EXECUTION_ID,
            workflow_name: "workflow",
            workspace_identity: "/repo",
            workflow_definition: &definition,
            node_executions: &node_executions,
            started_at: 1.0,
            updated_at: 10.0,
            execution: &execution,
            recovery_owner_reason: None,
        })
        .unwrap();

        assert!(nodes
            .iter()
            .all(|node| node.execution_id.as_deref() == Some(EXECUTION_ID)));
        assert!(nodes
            .iter()
            .all(|node| node.node_execution_id.as_deref() != Some("unrelated")));
        let failed = nodes
            .iter()
            .find(|node| node.node_execution_id.as_deref() == Some("failed"))
            .unwrap();
        assert_eq!(failed.error_reason.as_deref(), Some("Workflow node failed"));
        assert_eq!(failed.command_result, None);
        assert_eq!(failed.updated_at_bits, 10.0f64.to_bits());
        let completed = nodes
            .iter()
            .find(|node| node.node_execution_id.as_deref() == Some("completed"))
            .unwrap();
        assert_eq!(completed.updated_at_bits, 10.0f64.to_bits());
    }

    #[test]
    fn runtime_snapshot_projects_completion_wait_and_retry_from_the_current_attempt() {
        let mut waiting = node(
            "waiting-submit",
            EXECUTION_ID,
            RuntimeNodeExecutionStatus::Running,
        );
        waiting.kind = NodeKindName::Session;
        waiting.display_command = None;
        waiting.completion_signals = NodeCompletionSignalState::SubmitReceived;
        waiting.artifact = Some(serde_json::json!({"result": "ready"}));
        let definition = WorkflowDefinition {
            name: "workflow".to_string(),
            nodes: vec![NodeDefinition {
                name: "test".to_string(),
                ..NodeDefinition::default()
            }],
            ..WorkflowDefinition::default()
        };
        let execution = execution();

        let nodes = runtime_snapshot_nodes(RuntimeSnapshotNodeProjection {
            execution_id: EXECUTION_ID,
            workflow_name: "workflow",
            workspace_identity: "/repo",
            workflow_definition: &definition,
            node_executions: &[waiting],
            started_at: 1.0,
            updated_at: 10.0,
            execution: &execution,
            recovery_owner_reason: None,
        })
        .unwrap();

        let waiting = nodes
            .iter()
            .find(|node| node.node_execution_id.as_deref() == Some("waiting-submit"))
            .unwrap();
        assert_eq!(
            waiting.completion_signals,
            NodeCompletionSignalState::SubmitReceived
        );
        assert!(waiting.has_artifact);
        assert!(waiting.can_retry);
    }

    #[test]
    fn started_nodes_keep_every_execution_status() {
        let cases = [
            (
                "running",
                RuntimeNodeExecutionStatus::Running,
                WorkspaceNodeStatus::Running,
            ),
            (
                "paused",
                RuntimeNodeExecutionStatus::Paused,
                WorkspaceNodeStatus::Paused,
            ),
            (
                "waiting",
                RuntimeNodeExecutionStatus::WaitingApproval,
                WorkspaceNodeStatus::Waiting,
            ),
            (
                "completed",
                RuntimeNodeExecutionStatus::Succeeded,
                WorkspaceNodeStatus::Completed,
            ),
            (
                "failed",
                RuntimeNodeExecutionStatus::Failed,
                WorkspaceNodeStatus::Failed,
            ),
            (
                "aborted",
                RuntimeNodeExecutionStatus::Aborted,
                WorkspaceNodeStatus::Aborted,
            ),
        ];
        let runtime_nodes = cases
            .iter()
            .map(|(id, status, _)| node(id, EXECUTION_ID, *status))
            .collect::<Vec<_>>();
        let execution = execution();
        let nodes = runtime_snapshot_nodes(RuntimeSnapshotNodeProjection {
            execution_id: EXECUTION_ID,
            workflow_name: "workflow",
            workspace_identity: "/repo",
            workflow_definition: &WorkflowDefinition::default(),
            node_executions: &runtime_nodes,
            started_at: 1.0,
            updated_at: 10.0,
            execution: &execution,
            recovery_owner_reason: None,
        })
        .unwrap();

        for (id, _, expected) in cases {
            assert_eq!(
                nodes
                    .iter()
                    .find(|node| node.node_execution_id.as_deref() == Some(id))
                    .unwrap()
                    .status,
                expected
            );
        }
    }

    #[test]
    fn failure_metadata_never_enters_workspace_summary_or_detail() {
        let mut failed = node(
            "internal-node-id",
            EXECUTION_ID,
            RuntimeNodeExecutionStatus::Failed,
        );
        failed.failure = Some(RuntimeNodeExecutionFailure {
            reason: "raw internal failure".to_string(),
            kind: NodeExecutionFailureKind::InfrastructureCrash,
        });
        let execution = execution();
        let nodes = runtime_snapshot_nodes(RuntimeSnapshotNodeProjection {
            execution_id: EXECUTION_ID,
            workflow_name: "workflow",
            workspace_identity: "/repo",
            workflow_definition: &WorkflowDefinition::default(),
            node_executions: &[failed],
            started_at: 1.0,
            updated_at: 10.0,
            execution: &execution,
            recovery_owner_reason: None,
        })
        .unwrap();
        let failed = nodes
            .iter()
            .find(|node| node.node_execution_id.as_deref() == Some("internal-node-id"))
            .unwrap();
        assert_eq!(failed.error_reason.as_deref(), Some("Workflow node failed"));
        assert!(!failed
            .error_reason
            .as_deref()
            .unwrap()
            .contains("raw internal failure"));
    }
}
