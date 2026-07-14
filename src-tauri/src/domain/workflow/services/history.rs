//! Workflow history entry construction rules.

use std::collections::HashMap;

use crate::domain::workflow::value_objects::{
    FanoutChildSnapshot, NodeHistoryEntry, RuntimeArtifact, TokenUsage, NODE_STATUS_ABORTED,
    NODE_STATUS_COMPLETED,
};
use crate::domain::workflow::FanoutRuntimeState;

#[derive(Debug, Clone, PartialEq)]
pub struct CompletedNodeHistoryInput {
    pub node_name: String,
    pub completed_at: f64,
    pub result: Option<String>,
    pub session_id: Option<String>,
    pub token_usage: Option<TokenUsage>,
    pub artifact: Option<serde_json::Value>,
    pub attempt: u32,
}

pub fn completed_node_history_entry(input: CompletedNodeHistoryInput) -> NodeHistoryEntry {
    NodeHistoryEntry {
        node_name: input.node_name,
        completed_at: input.completed_at,
        result: input.result,
        session_id: input.session_id,
        token_usage: input.token_usage,
        artifact: input.artifact,
        attempt: input.attempt,
        fanout_children: None,
        state: crate::domain::workflow::value_objects::default_node_history_status(),
    }
}

pub fn artifact_from_completed_history_entry(
    entry: &NodeHistoryEntry,
    contract: Option<String>,
) -> Option<RuntimeArtifact> {
    entry.artifact.as_ref()?;
    Some(RuntimeArtifact {
        node_name: entry.node_name.clone(),
        attempt: entry.attempt,
        session_id: entry.session_id.clone(),
        result: entry.result.clone(),
        artifact: entry.artifact.clone(),
        contract,
        token_usage: entry.token_usage.clone(),
        completed_at: entry.completed_at,
    })
}

pub fn session_start_failed_result(reason: impl AsRef<str>) -> String {
    format!("session_start_failed: {}", reason.as_ref())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeStartFailureKind {
    StepSession,
    ParallelChildren,
}

pub fn runtime_start_failure_reason(
    kind: RuntimeStartFailureKind,
    error: impl AsRef<str>,
) -> String {
    match kind {
        RuntimeStartFailureKind::StepSession => {
            format!("Failed to start node session: {}", error.as_ref())
        }
        RuntimeStartFailureKind::ParallelChildren => {
            format!("Failed to start parallel children: {}", error.as_ref())
        }
    }
}

pub fn aborted_node_history_entry(
    node_name: String,
    attempt: u32,
    session_id: Option<String>,
    token_usage: TokenUsage,
    timestamp: f64,
) -> NodeHistoryEntry {
    NodeHistoryEntry {
        node_name,
        completed_at: timestamp,
        result: None,
        session_id,
        token_usage: Some(token_usage),
        artifact: None,
        attempt,
        fanout_children: None,
        state: NODE_STATUS_ABORTED.to_string(),
    }
}

pub fn aborted_parallel_history_entry(
    parallel_run: &FanoutRuntimeState,
    artifacts: &HashMap<String, RuntimeArtifact>,
    parent_attempt: u32,
    timestamp: f64,
) -> NodeHistoryEntry {
    let fanout_children = parallel_run
        .children
        .iter()
        .map(|child| {
            let output = artifacts.get(&child.node_name);
            FanoutChildSnapshot {
                node_name: child.node_name.clone(),
                session_id: output
                    .and_then(|value| value.session_id.clone())
                    .or_else(|| non_empty_session_id(&child.session_id)),
                result: output
                    .and_then(|value| value.result.clone())
                    .or_else(|| child.result.clone()),
                attempt: child.attempt,
                completed_at: output.map(|value| value.completed_at).unwrap_or(timestamp),
                artifact: output.and_then(|value| value.artifact.clone()),
                contract: output.and_then(|value| value.contract.clone()),
                state: if child.state.is_completed() {
                    NODE_STATUS_COMPLETED.to_string()
                } else {
                    NODE_STATUS_ABORTED.to_string()
                },
                failure_kind: child.failure_kind,
                failure_disposition: child.failure_disposition,
            }
        })
        .collect();
    NodeHistoryEntry {
        node_name: parallel_run.parent_node_name.clone(),
        completed_at: timestamp,
        result: None,
        session_id: None,
        token_usage: None,
        artifact: None,
        attempt: parent_attempt,
        fanout_children: Some(fanout_children),
        state: NODE_STATUS_ABORTED.to_string(),
    }
}

fn non_empty_session_id(session_id: &str) -> Option<String> {
    if session_id.is_empty() {
        None
    } else {
        Some(session_id.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::{FanoutChildRuntime, FanoutChildRuntimeState};

    #[test]
    fn completed_node_history_entry_and_output_share_completion_fields() {
        let entry = completed_node_history_entry(CompletedNodeHistoryInput {
            node_name: "review".to_string(),
            completed_at: 10.0,
            result: Some("LGTM".to_string()),
            session_id: Some("session-review".to_string()),
            token_usage: Some(TokenUsage {
                input_tokens: 2,
                output_tokens: 3,
            }),
            artifact: Some(serde_json::json!({"ok": true})),
            attempt: 4,
        });

        assert_eq!(entry.state, "completed");
        let output =
            artifact_from_completed_history_entry(&entry, Some("review-contract".to_string()))
                .unwrap();
        assert_eq!(output.node_name, "review");
        assert_eq!(output.session_id.as_deref(), Some("session-review"));
        assert_eq!(output.result.as_deref(), Some("LGTM"));
        assert_eq!(output.contract.as_deref(), Some("review-contract"));
        assert_eq!(output.token_usage.unwrap().input_tokens, 2);
    }

    #[test]
    fn completed_node_without_artifact_has_no_artifact() {
        let entry = completed_node_history_entry(CompletedNodeHistoryInput {
            node_name: "review".to_string(),
            completed_at: 10.0,
            result: None,
            session_id: None,
            token_usage: None,
            artifact: None,
            attempt: 1,
        });

        assert!(artifact_from_completed_history_entry(&entry, None).is_none());
    }

    #[test]
    fn session_start_failed_result_preserves_persisted_prefix() {
        assert_eq!(
            session_start_failed_result("backend unavailable"),
            "session_start_failed: backend unavailable"
        );
    }

    #[test]
    fn runtime_start_failure_reason_preserves_failed_state_messages() {
        assert_eq!(
            runtime_start_failure_reason(RuntimeStartFailureKind::StepSession, "backend down"),
            "Failed to start node session: backend down"
        );
        assert_eq!(
            runtime_start_failure_reason(RuntimeStartFailureKind::ParallelChildren, "backend down"),
            "Failed to start parallel children: backend down"
        );
    }

    #[test]
    fn aborted_node_history_entry_keeps_session_and_token_usage() {
        let entry = aborted_node_history_entry(
            "review".to_string(),
            3,
            Some("session-review".to_string()),
            TokenUsage {
                input_tokens: 5,
                output_tokens: 8,
            },
            12.0,
        );

        assert_eq!(entry.node_name, "review");
        assert_eq!(entry.attempt, 3);
        assert_eq!(entry.session_id.as_deref(), Some("session-review"));
        assert_eq!(entry.token_usage.unwrap().input_tokens, 5);
        assert_eq!(entry.state, "aborted");
    }

    #[test]
    fn aborted_parallel_history_entry_snapshots_children_with_output_fallback() {
        let mut artifacts = HashMap::new();
        artifacts.insert(
            "child-a".to_string(),
            RuntimeArtifact {
                node_name: "child-a".to_string(),
                attempt: 1,
                session_id: Some("output-session-a".to_string()),
                result: Some("from output".to_string()),
                artifact: Some(serde_json::json!({ "ok": true })),
                contract: Some("review".to_string()),
                token_usage: None,
                completed_at: 10.0,
            },
        );

        let entry = aborted_parallel_history_entry(
            &FanoutRuntimeState {
                parent_node_name: "parallel-review".to_string(),
                children: vec![
                    FanoutChildRuntime {
                        node_name: "child-a".to_string(),
                        session_id: "child-session-a".to_string(),
                        state: FanoutChildRuntimeState::Completed,
                        result: Some("from child".to_string()),
                        artifact: None,
                        contract: None,
                        failure_kind: None,
                        failure_disposition: None,
                        token_usage: TokenUsage::default(),
                        attempt: 1,
                    },
                    FanoutChildRuntime {
                        node_name: "child-b".to_string(),
                        session_id: "child-session-b".to_string(),
                        state: FanoutChildRuntimeState::Running,
                        result: Some("partial".to_string()),
                        artifact: None,
                        contract: None,
                        failure_kind: None,
                        failure_disposition: None,
                        token_usage: TokenUsage::default(),
                        attempt: 2,
                    },
                ],
            },
            &artifacts,
            4,
            20.0,
        );

        assert_eq!(entry.node_name, "parallel-review");
        assert_eq!(entry.attempt, 4);
        let children = entry.fanout_children.unwrap();
        assert_eq!(children[0].session_id.as_deref(), Some("output-session-a"));
        assert_eq!(children[0].result.as_deref(), Some("from output"));
        assert_eq!(children[0].completed_at, 10.0);
        assert_eq!(children[0].state, "completed");
        assert_eq!(children[1].session_id.as_deref(), Some("child-session-b"));
        assert_eq!(children[1].result.as_deref(), Some("partial"));
        assert_eq!(children[1].completed_at, 20.0);
        assert_eq!(children[1].state, "aborted");
    }
}
