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
    pub execution_id: &'a str,
    pub workflow_name: &'a str,
    pub workspace_identity: &'a str,
    pub workflow_definition: &'a crate::domain::workflow::WorkflowDefinition,
    pub node_executions:
        &'a [crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecution],
    pub retry_predecessors: &'a std::collections::HashMap<String, String>,
    pub accepts_explicit_retry: bool,
    pub started_at: f64,
    pub updated_at: f64,
    pub execution: &'a crate::domain::local_event::WorkflowExecutionMetadataRecord,
    pub recovery_owner_reason: Option<String>,
    pub node_recovery_reasons: &'a [(String, String)],
    pub session_activities:
        &'a std::collections::HashMap<String, crate::domain::workflow::AgentSessionActivity>,
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
        retry_predecessors,
        accepts_explicit_retry,
        started_at,
        updated_at,
        execution,
        recovery_owner_reason,
        node_recovery_reasons,
        session_activities,
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
    facts.extend(
        node_recovery_reasons
            .iter()
            .map(|(owner, reason)| F::RecoveryFenceProjected {
                owner: owner.clone(),
                reason: Some(reason.clone()),
            }),
    );
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
        node.has_artifact = runtime.artifact.is_some();
        node.can_retry = node.recovery_owner_reason.is_none()
            && accepts_explicit_retry
            && runtime.can_retry()
            && node_executions.iter().all(|candidate| {
                !same_retry_target(runtime, candidate) || candidate.attempt <= runtime.attempt
            });
        node.resume_eligible = runtime.can_resume();
        if runtime.status == S::Paused {
            node.status = crate::domain::workspace_tree::WorkspaceNodeStatus::Paused;
        }
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
mod tests {
    use super::*;
    use crate::domain::local_event::WorkflowExecutionMetadataRecord;
    use crate::domain::workflow::entities::workflow_execution::{
        RuntimeNodeExecution, RuntimeNodeExecutionFailure, RuntimeNodeExecutionFailureOrigin,
        RuntimeNodeExecutionStatus,
    };
    use crate::domain::workflow::{
        ExecutionOrigin, ExecutionStatus, NodeCompletionSignalState, NodeDefinition,
        NodeExecutionFailureKind, NodeKindName, TokenUsage, WorkflowDefinition,
    };
    use crate::domain::workspace_tree::{
        WorkspaceNodeStatus, WorkspaceNodeStatusClassification, WorkspaceTree, WorkspaceTreeNode,
    };

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
            result_summary: None,
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

    fn capability_state(
        node: &WorkspaceTreeNode,
    ) -> (bool, bool, bool, bool, bool, bool, Option<&str>) {
        (
            node.can_approve,
            node.can_retry,
            node.can_stop,
            node.can_resume,
            node.can_abort,
            node.can_archive,
            node.resume_unavailable_reason.as_deref(),
        )
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
            retry_predecessors: &std::collections::HashMap::new(),
            accepts_explicit_retry: true,
            started_at: 1.0,
            updated_at: 10.0,
            execution: &execution,
            recovery_owner_reason: None,
            node_recovery_reasons: &[],
            session_activities: &std::collections::HashMap::new(),
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
            node_executions: &[waiting.clone()],
            retry_predecessors: &std::collections::HashMap::new(),
            accepts_explicit_retry: true,
            started_at: 1.0,
            updated_at: 10.0,
            execution: &execution,
            recovery_owner_reason: None,
            node_recovery_reasons: &[],
            session_activities: &std::collections::HashMap::new(),
        })
        .unwrap();

        let waiting_node = nodes
            .iter()
            .find(|node| node.node_execution_id.as_deref() == Some("waiting-submit"))
            .unwrap();
        assert_eq!(
            waiting_node.completion_signals,
            NodeCompletionSignalState::SubmitReceived
        );
        assert!(waiting_node.has_artifact);
        assert!(waiting_node.can_retry);

        let nodes = runtime_snapshot_nodes(RuntimeSnapshotNodeProjection {
            execution_id: EXECUTION_ID,
            workflow_name: "session",
            workspace_identity: "/repo",
            workflow_definition: &definition,
            node_executions: &[waiting.clone()],
            retry_predecessors: &std::collections::HashMap::new(),
            accepts_explicit_retry: false,
            started_at: 1.0,
            updated_at: 10.0,
            execution: &execution,
            recovery_owner_reason: None,
            node_recovery_reasons: &[],
            session_activities: &std::collections::HashMap::new(),
        })
        .unwrap();
        assert!(!nodes[0].can_retry);
    }

    #[test]
    fn explicit_retry_links_group_only_the_retry_chain_in_execution_order() {
        let mut first = node("first", EXECUTION_ID, RuntimeNodeExecutionStatus::Failed);
        first.attempt = 1;
        first.completed_at = Some(3.0);
        let mut second = node("second", EXECUTION_ID, RuntimeNodeExecutionStatus::Failed);
        second.attempt = 2;
        second.started_at = 4.0;
        second.completed_at = Some(5.0);
        let mut latest = node("latest", EXECUTION_ID, RuntimeNodeExecutionStatus::Running);
        latest.attempt = 3;
        latest.started_at = 6.0;
        let mut loop_visit = node(
            "same-name-loop-visit",
            EXECUTION_ID,
            RuntimeNodeExecutionStatus::Succeeded,
        );
        loop_visit.attempt = 4;
        loop_visit.started_at = 7.0;
        loop_visit.completed_at = Some(8.0);
        let retry_predecessors = std::collections::HashMap::from([
            ("second".to_string(), "first".to_string()),
            ("latest".to_string(), "second".to_string()),
        ]);
        let execution = execution();

        let nodes = runtime_snapshot_nodes(RuntimeSnapshotNodeProjection {
            execution_id: EXECUTION_ID,
            workflow_name: "workflow",
            workspace_identity: "/repo",
            workflow_definition: &WorkflowDefinition::default(),
            node_executions: &[first, second, latest, loop_visit],
            retry_predecessors: &retry_predecessors,
            accepts_explicit_retry: true,
            started_at: 1.0,
            updated_at: 10.0,
            execution: &execution,
            recovery_owner_reason: None,
            node_recovery_reasons: &[],
            session_activities: &std::collections::HashMap::new(),
        })
        .unwrap();
        let by_execution_id = nodes
            .iter()
            .filter_map(|node| node.node_execution_id.as_deref().map(|id| (id, node)))
            .collect::<std::collections::HashMap<_, _>>();
        let first = by_execution_id["first"];
        let second = by_execution_id["second"];
        let latest = by_execution_id["latest"];
        let loop_visit = by_execution_id["same-name-loop-visit"];

        assert!(first.is_retry_history);
        assert!(second.is_retry_history);
        assert!(!latest.is_retry_history);
        assert!(!loop_visit.is_retry_history);
        assert_eq!(
            latest.past_attempt_ids,
            vec![first.id.clone(), second.id.clone()]
        );
        assert!(first.past_attempt_ids.is_empty());
        assert!(second.past_attempt_ids.is_empty());
        assert!(loop_visit.past_attempt_ids.is_empty());
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
            retry_predecessors: &std::collections::HashMap::new(),
            accepts_explicit_retry: true,
            started_at: 1.0,
            updated_at: 10.0,
            execution: &execution,
            recovery_owner_reason: None,
            node_recovery_reasons: &[],
            session_activities: &std::collections::HashMap::new(),
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
            origin: RuntimeNodeExecutionFailureOrigin::Runtime,
        });
        let execution = execution();
        let nodes = runtime_snapshot_nodes(RuntimeSnapshotNodeProjection {
            execution_id: EXECUTION_ID,
            workflow_name: "workflow",
            workspace_identity: "/repo",
            workflow_definition: &WorkflowDefinition::default(),
            node_executions: &[failed],
            retry_predecessors: &std::collections::HashMap::new(),
            accepts_explicit_retry: true,
            started_at: 1.0,
            updated_at: 10.0,
            execution: &execution,
            recovery_owner_reason: None,
            node_recovery_reasons: &[],
            session_activities: &std::collections::HashMap::new(),
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

    #[test]
    fn isolated_worktree_loss_fences_retry_and_projects_the_recovery_reason() {
        let failed = node(
            "lost-node",
            EXECUTION_ID,
            RuntimeNodeExecutionStatus::Failed,
        );
        let execution = execution();
        let reason = "isolated worktree is missing: /repo/.releash-isolated/lost-node-a1";

        let nodes = runtime_snapshot_nodes(RuntimeSnapshotNodeProjection {
            execution_id: EXECUTION_ID,
            workflow_name: "workflow",
            workspace_identity: "/repo",
            workflow_definition: &WorkflowDefinition::default(),
            node_executions: &[failed],
            retry_predecessors: &std::collections::HashMap::new(),
            accepts_explicit_retry: true,
            started_at: 1.0,
            updated_at: 10.0,
            execution: &execution,
            recovery_owner_reason: None,
            node_recovery_reasons: &[("lost-node".to_string(), reason.to_string())],
            session_activities: &std::collections::HashMap::new(),
        })
        .unwrap();

        let failed = nodes
            .iter()
            .find(|node| node.node_execution_id.as_deref() == Some("lost-node"))
            .unwrap();
        assert_eq!(failed.recovery_owner_reason.as_deref(), Some(reason));
        assert!(!failed.can_retry);
        let root = nodes
            .iter()
            .find(|node| node.node_execution_id.is_none())
            .unwrap();
        assert_eq!(root.resume_unavailable_reason.as_deref(), Some(reason));
    }

    #[test]
    fn test_runtime_snapshot分類_session_workingはstop_receivedでもactiveにしてcapabilityを維持する(
    ) {
        // Given
        let mut sequence = node(
            "sequence",
            EXECUTION_ID,
            RuntimeNodeExecutionStatus::Succeeded,
        );
        sequence.node_name = "sequence".to_string();
        sequence.kind = NodeKindName::Sequence;
        sequence.display_command = None;
        sequence.completed_at = Some(4.0);
        let mut stopped_child = node(
            "stopped-child",
            EXECUTION_ID,
            RuntimeNodeExecutionStatus::Running,
        );
        stopped_child.node_name = "stopped-child".to_string();
        stopped_child.kind = NodeKindName::Session;
        stopped_child.display_command = None;
        stopped_child.parent = Some(crate::domain::workflow::ExecutionParentRef::sequence_child(
            "sequence",
        ));
        stopped_child.completion_signals = NodeCompletionSignalState::StopReceived;
        let execution = execution();
        let runtime_nodes = [sequence, stopped_child];
        let session_activities = std::collections::HashMap::from([(
            "stopped-child".to_string(),
            crate::domain::workflow::AgentSessionActivity::Working,
        )]);

        // When
        let nodes = runtime_snapshot_nodes(RuntimeSnapshotNodeProjection {
            execution_id: EXECUTION_ID,
            workflow_name: "workflow",
            workspace_identity: "/repo",
            workflow_definition: &WorkflowDefinition::default(),
            node_executions: &runtime_nodes,
            retry_predecessors: &std::collections::HashMap::new(),
            accepts_explicit_retry: true,
            started_at: 1.0,
            updated_at: 10.0,
            execution: &execution,
            recovery_owner_reason: None,
            node_recovery_reasons: &[],
            session_activities: &session_activities,
        })
        .unwrap();
        let by_execution_id = nodes
            .iter()
            .filter_map(|node| node.node_execution_id.as_deref().map(|id| (id, node)))
            .collect::<std::collections::HashMap<_, _>>();
        let workflow = nodes
            .iter()
            .find(|node| node.node_execution_id.is_none())
            .unwrap();

        // Then
        assert_eq!(
            by_execution_id["stopped-child"].status_classification,
            WorkspaceNodeStatusClassification::Active
        );
        assert_eq!(
            by_execution_id["sequence"].status_classification,
            WorkspaceNodeStatusClassification::Active
        );
        assert_eq!(
            capability_state(by_execution_id["stopped-child"]),
            (false, true, false, false, false, false, None)
        );
        assert_eq!(
            capability_state(workflow),
            (false, false, true, false, true, false, None)
        );
    }

    #[test]
    fn test_runtime_snapshot分類_session承認待ちでもworkingならactiveになる() {
        let mut waiting = node(
            "waiting-session",
            EXECUTION_ID,
            RuntimeNodeExecutionStatus::WaitingApproval,
        );
        waiting.kind = NodeKindName::Session;
        waiting.display_command = None;
        let execution = execution();
        let session_activities = std::collections::HashMap::from([(
            "waiting-session".to_string(),
            crate::domain::workflow::AgentSessionActivity::Working,
        )]);

        let nodes = runtime_snapshot_nodes(RuntimeSnapshotNodeProjection {
            execution_id: EXECUTION_ID,
            workflow_name: "workflow",
            workspace_identity: "/repo",
            workflow_definition: &WorkflowDefinition::default(),
            node_executions: &[waiting],
            retry_predecessors: &std::collections::HashMap::new(),
            accepts_explicit_retry: true,
            started_at: 1.0,
            updated_at: 10.0,
            execution: &execution,
            recovery_owner_reason: None,
            node_recovery_reasons: &[],
            session_activities: &session_activities,
        })
        .unwrap();
        let waiting = nodes
            .iter()
            .find(|node| node.node_execution_id.as_deref() == Some("waiting-session"))
            .unwrap();

        assert_eq!(
            waiting.status_classification,
            WorkspaceNodeStatusClassification::Active
        );
        assert!(waiting.can_approve);
        assert_eq!(
            waiting.activity,
            Some(crate::domain::workflow::AgentSessionActivity::Working)
        );
    }

    #[test]
    fn test_runtime_snapshot操作可否_session活動状態が変わっても不変になる() {
        // Given: Stop 受領済みの実行中 Session と承認待ち Session
        let cases = [
            (
                "stopped-session",
                RuntimeNodeExecutionStatus::Running,
                NodeCompletionSignalState::StopReceived,
            ),
            (
                "waiting-session",
                RuntimeNodeExecutionStatus::WaitingApproval,
                NodeCompletionSignalState::Pending,
            ),
        ];

        for (node_execution_id, status, completion_signals) in cases {
            let mut session = node(node_execution_id, EXECUTION_ID, status);
            session.kind = NodeKindName::Session;
            session.display_command = None;
            session.completion_signals = completion_signals;
            let runtime_nodes = [session];
            let definition = WorkflowDefinition::default();
            let execution = execution();

            // When: Node 状態を固定して3つの活動状態を投影する
            let snapshots = [
                crate::domain::workflow::AgentSessionActivity::Working,
                crate::domain::workflow::AgentSessionActivity::AwaitingAnswer,
                crate::domain::workflow::AgentSessionActivity::AwaitingInstruction,
            ]
            .map(|activity| {
                let session_activities =
                    std::collections::HashMap::from([(node_execution_id.to_string(), activity)]);
                runtime_snapshot_nodes(RuntimeSnapshotNodeProjection {
                    execution_id: EXECUTION_ID,
                    workflow_name: "workflow",
                    workspace_identity: "/repo",
                    workflow_definition: &definition,
                    node_executions: &runtime_nodes,
                    retry_predecessors: &std::collections::HashMap::new(),
                    accepts_explicit_retry: true,
                    started_at: 1.0,
                    updated_at: 10.0,
                    execution: &execution,
                    recovery_owner_reason: None,
                    node_recovery_reasons: &[],
                    session_activities: &session_activities,
                })
                .unwrap()
            });

            let baseline_leaf = snapshots[0]
                .iter()
                .find(|node| node.node_execution_id.as_deref() == Some(node_execution_id))
                .unwrap();
            let baseline_root = snapshots[0]
                .iter()
                .find(|node| node.node_execution_id.is_none())
                .unwrap();

            // Then: leaf と workflow root の操作可否・resume 不能理由は一致する
            for snapshot in &snapshots[1..] {
                let leaf = snapshot
                    .iter()
                    .find(|node| node.node_execution_id.as_deref() == Some(node_execution_id))
                    .unwrap();
                let root = snapshot
                    .iter()
                    .find(|node| node.node_execution_id.is_none())
                    .unwrap();
                assert_eq!(capability_state(leaf), capability_state(baseline_leaf));
                assert_eq!(capability_state(root), capability_state(baseline_root));
            }
        }
    }

    #[test]
    fn test_runtime_snapshot復旧_異常終了したsessionにresumeを提示する() {
        let mut failed = node(
            "failed-session",
            EXECUTION_ID,
            RuntimeNodeExecutionStatus::Failed,
        );
        failed.kind = NodeKindName::Session;
        failed.display_command = None;
        failed.failure = Some(RuntimeNodeExecutionFailure {
            reason: "provider process exited abnormally".to_string(),
            kind: NodeExecutionFailureKind::InfrastructureCrash,
            origin: RuntimeNodeExecutionFailureOrigin::ProviderProcessExit,
        });
        let execution = execution();

        let nodes = runtime_snapshot_nodes(RuntimeSnapshotNodeProjection {
            execution_id: EXECUTION_ID,
            workflow_name: "workflow",
            workspace_identity: "/repo",
            workflow_definition: &WorkflowDefinition::default(),
            node_executions: &[failed],
            retry_predecessors: &std::collections::HashMap::new(),
            accepts_explicit_retry: true,
            started_at: 1.0,
            updated_at: 10.0,
            execution: &execution,
            recovery_owner_reason: None,
            node_recovery_reasons: &[],
            session_activities: &std::collections::HashMap::new(),
        })
        .unwrap();
        let tree = WorkspaceTree::restore("/repo", nodes).unwrap();
        let failed = tree
            .nodes()
            .iter()
            .find(|node| node.node_execution_id.as_deref() == Some("failed-session"))
            .unwrap();
        let workflow = tree
            .nodes()
            .iter()
            .find(|node| node.node_execution_id.is_none())
            .unwrap();

        assert_eq!(
            failed.status_classification,
            WorkspaceNodeStatusClassification::Failure
        );
        assert!(failed.can_retry);
        assert!(workflow.can_resume);
        assert_eq!(workflow.resume_unavailable_reason, None);
    }

    #[test]
    fn test_runtime_snapshot復旧_runtime失敗sessionにresumeを提示しない() {
        let mut failed = node(
            "runtime-failed-session",
            EXECUTION_ID,
            RuntimeNodeExecutionStatus::Failed,
        );
        failed.kind = NodeKindName::Session;
        failed.display_command = None;
        failed.failure = Some(RuntimeNodeExecutionFailure {
            reason: "activation failed".to_string(),
            kind: NodeExecutionFailureKind::InfrastructureCrash,
            origin: RuntimeNodeExecutionFailureOrigin::Runtime,
        });
        let execution = execution();

        let nodes = runtime_snapshot_nodes(RuntimeSnapshotNodeProjection {
            execution_id: EXECUTION_ID,
            workflow_name: "workflow",
            workspace_identity: "/repo",
            workflow_definition: &WorkflowDefinition::default(),
            node_executions: &[failed],
            retry_predecessors: &std::collections::HashMap::new(),
            accepts_explicit_retry: true,
            started_at: 1.0,
            updated_at: 10.0,
            execution: &execution,
            recovery_owner_reason: None,
            node_recovery_reasons: &[],
            session_activities: &std::collections::HashMap::new(),
        })
        .unwrap();
        let tree = WorkspaceTree::restore("/repo", nodes).unwrap();
        let failed = tree
            .nodes()
            .iter()
            .find(|node| node.node_execution_id.as_deref() == Some("runtime-failed-session"))
            .unwrap();
        let workflow = tree
            .nodes()
            .iter()
            .find(|node| node.node_execution_id.is_none())
            .unwrap();

        assert!(failed.can_retry);
        assert!(!failed.can_resume);
        assert!(!workflow.can_resume);
    }

    #[test]
    fn test_runtime_snapshot分類_pausedのstop_receivedをidleにしてcapabilityを維持する() {
        // Given
        let mut paused = node("paused", EXECUTION_ID, RuntimeNodeExecutionStatus::Paused);
        paused.node_name = "paused".to_string();
        paused.kind = NodeKindName::Session;
        paused.display_command = None;
        paused.completion_signals = NodeCompletionSignalState::StopReceived;
        let execution = execution();

        // When
        let nodes = runtime_snapshot_nodes(RuntimeSnapshotNodeProjection {
            execution_id: EXECUTION_ID,
            workflow_name: "workflow",
            workspace_identity: "/repo",
            workflow_definition: &WorkflowDefinition::default(),
            node_executions: &[paused],
            retry_predecessors: &std::collections::HashMap::new(),
            accepts_explicit_retry: true,
            started_at: 1.0,
            updated_at: 10.0,
            execution: &execution,
            recovery_owner_reason: None,
            node_recovery_reasons: &[],
            session_activities: &std::collections::HashMap::new(),
        })
        .unwrap();
        let paused = nodes
            .iter()
            .find(|node| node.node_execution_id.as_deref() == Some("paused"))
            .unwrap();
        let workflow = nodes
            .iter()
            .find(|node| node.node_execution_id.is_none())
            .unwrap();

        // Then
        assert_eq!(
            paused.status_classification,
            WorkspaceNodeStatusClassification::Idle
        );
        assert_eq!(
            capability_state(paused),
            (false, true, false, false, false, false, None)
        );
        assert_eq!(
            capability_state(workflow),
            (false, false, true, false, true, false, None)
        );
    }

    #[test]
    fn test_runtime_snapshot分類_recovery_fenceをfailureにしてcapabilityを維持する() {
        // Given
        let mut fanout = node(
            "fanout",
            EXECUTION_ID,
            RuntimeNodeExecutionStatus::Succeeded,
        );
        fanout.node_name = "fanout".to_string();
        fanout.kind = NodeKindName::Fanout;
        fanout.display_command = None;
        fanout.completed_at = Some(4.0);
        let mut recovery = node("recovery", EXECUTION_ID, RuntimeNodeExecutionStatus::Paused);
        recovery.node_name = "recovery".to_string();
        recovery.parent = Some(crate::domain::workflow::ExecutionParentRef::fanout_child(
            "fanout", None, 0,
        ));
        let execution = execution();
        let runtime_nodes = [fanout, recovery];
        let recovery_reason = "recovery fence";

        // When
        let nodes = runtime_snapshot_nodes(RuntimeSnapshotNodeProjection {
            execution_id: EXECUTION_ID,
            workflow_name: "workflow",
            workspace_identity: "/repo",
            workflow_definition: &WorkflowDefinition::default(),
            node_executions: &runtime_nodes,
            retry_predecessors: &std::collections::HashMap::new(),
            accepts_explicit_retry: true,
            started_at: 1.0,
            updated_at: 10.0,
            execution: &execution,
            recovery_owner_reason: None,
            node_recovery_reasons: &[("recovery".to_string(), recovery_reason.to_string())],
            session_activities: &std::collections::HashMap::new(),
        })
        .unwrap();
        let by_execution_id = nodes
            .iter()
            .filter_map(|node| node.node_execution_id.as_deref().map(|id| (id, node)))
            .collect::<std::collections::HashMap<_, _>>();
        let workflow = nodes
            .iter()
            .find(|node| node.node_execution_id.is_none())
            .unwrap();

        // Then
        assert_eq!(
            by_execution_id["recovery"].status_classification,
            WorkspaceNodeStatusClassification::Failure
        );
        assert_eq!(
            by_execution_id["fanout"].status_classification,
            WorkspaceNodeStatusClassification::Failure
        );
        assert_eq!(
            capability_state(by_execution_id["recovery"]),
            (false, false, false, false, false, false, None)
        );
        assert_eq!(
            capability_state(workflow),
            (
                false,
                false,
                true,
                false,
                true,
                false,
                Some(recovery_reason)
            )
        );
    }
}
