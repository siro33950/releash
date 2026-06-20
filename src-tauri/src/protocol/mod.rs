mod agent;
mod auth;
mod branch;
mod error;
mod worktree;

pub use agent::*;
pub use auth::*;
pub use branch::*;
pub use error::*;
pub use worktree::*;

use crate::adaptor::protocol::pty::{PtyExitMsg, PtyOutputMsg};
use crate::adaptor::protocol::workflow::WorkflowStateSync;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
#[allow(clippy::large_enum_variant)]
pub enum WsMessage {
    // 認証
    #[serde(rename = "auth_challenge")]
    AuthChallenge(AuthChallenge),
    #[serde(rename = "auth_response")]
    AuthResponse(AuthResponse),
    #[serde(rename = "auth_result")]
    AuthResult(AuthResult),

    // ターミナル push
    #[serde(rename = "pty_output")]
    PtyOutput(PtyOutputMsg),
    #[serde(rename = "pty_exit")]
    PtyExit(PtyExitMsg),

    // Worktree / branch push
    #[serde(rename = "worktree_pr_status_sync")]
    WorktreePrStatusSync(WorktreePrStatusSync),
    #[serde(rename = "branch_list_sync")]
    BranchListSync(BranchListSync),

    // Agent / workflow push
    #[serde(rename = "agent_state_sync")]
    AgentStateSync(AgentStateSync),
    #[serde(rename = "workflow_state_sync")]
    WorkflowStateSync(Box<WorkflowStateSync>),
    #[serde(rename = "agent_stream_sync")]
    AgentStreamSync(AgentStreamSync),

    // 制御
    #[serde(rename = "error")]
    Error(ErrorMsg),
}

pub fn serialize_message(msg: &WsMessage) -> Result<String, String> {
    serde_json::to_string(msg).map_err(|e| format!("シリアライズ失敗: {e}"))
}

pub fn deserialize_message(json: &str) -> Result<WsMessage, String> {
    serde_json::from_str(json).map_err(|e| format!("デシリアライズ失敗: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptor::protocol::pty::{PtyExitMsg, PtyOutputMsg};

    #[test]
    fn serialize_auth_challenge() {
        let msg = WsMessage::AuthChallenge(AuthChallenge {
            challenge: "abc123".to_string(),
        });
        let json = serialize_message(&msg).unwrap();
        assert!(json.contains("\"type\":\"auth_challenge\""));
        assert!(json.contains("\"challenge\":\"abc123\""));
    }

    #[test]
    fn roundtrip_auth_result_with_message() {
        let msg = WsMessage::AuthResult(AuthResult {
            success: false,
            message: Some("invalid token".to_string()),
        });
        let json = serialize_message(&msg).unwrap();
        let deserialized = deserialize_message(&json).unwrap();
        match deserialized {
            WsMessage::AuthResult(r) => {
                assert!(!r.success);
                assert_eq!(r.message.unwrap(), "invalid token");
            }
            _ => panic!("unexpected variant"),
        }
    }

    #[test]
    fn auth_result_omits_none_message() {
        let msg = WsMessage::AuthResult(AuthResult {
            success: true,
            message: None,
        });
        let json = serialize_message(&msg).unwrap();
        assert!(!json.contains("\"message\""));
    }

    #[test]
    fn roundtrip_error() {
        let msg = WsMessage::Error(ErrorMsg {
            code: "UNAUTHORIZED".to_string(),
            message: "Authentication failed".to_string(),
        });
        let json = serialize_message(&msg).unwrap();
        let deserialized = deserialize_message(&json).unwrap();
        match deserialized {
            WsMessage::Error(e) => {
                assert_eq!(e.code, "UNAUTHORIZED");
                assert_eq!(e.message, "Authentication failed");
            }
            _ => panic!("unexpected variant"),
        }
    }

    #[test]
    fn deserialize_unknown_type_fails() {
        let json = r#"{"type":"unknown_type","payload":{}}"#;
        assert!(deserialize_message(json).is_err());
    }

    #[test]
    fn all_variants_roundtrip() {
        let messages = vec![
            WsMessage::AuthChallenge(AuthChallenge {
                challenge: "x".to_string(),
            }),
            WsMessage::AuthResponse(AuthResponse {
                hmac: "y".to_string(),
            }),
            WsMessage::AuthResult(AuthResult {
                success: true,
                message: None,
            }),
            WsMessage::PtyOutput(PtyOutputMsg {
                pty_id: 1,
                data: "d".to_string(),
            }),
            WsMessage::PtyExit(PtyExitMsg {
                pty_id: 1,
                exit_code: Some(0),
            }),
            WsMessage::WorktreePrStatusSync(WorktreePrStatusSync {
                entries: vec![WorktreePrEntry {
                    path: "/repo".to_string(),
                    pr_number: 12,
                    pr_url: "https://example.test/pr/12".to_string(),
                }],
            }),
            WsMessage::BranchListSync(BranchListSync {
                branches: vec![BranchCardMsg {
                    name: "feature/test".to_string(),
                    is_main_worktree: false,
                    worktree_path: Some("/repo-worktrees/feature-test".to_string()),
                    dirty_count: 2,
                    is_merged: false,
                    ahead: 3,
                    behind: 1,
                    has_upstream: true,
                    base_ahead: 0,
                }],
            }),
            WsMessage::AgentStateSync(AgentStateSync {
                worktree_path: "/repo".to_string(),
                state: AgentState::Running,
                exit_code: None,
                timestamp: 1234567890.0,
                session_id: Some("sess-1".to_string()),
                pty_id: None,
            }),
            WsMessage::WorkflowStateSync(Box::new(WorkflowStateSync {
                worktree_path: "/repo".to_string(),
                workflow_state: crate::adaptor::protocol::workflow::WorkflowStateView::from_parts(
                    crate::adaptor::presenter::workflow::workflow_state_to_view(
                        crate::domain::workflow::WorkflowStateSnapshot {
                            execution_id: "exec-1".to_string(),
                            workflow_name: "test".to_string(),
                            state: crate::domain::workflow::WorkflowExecutionState::Running,
                            current_step_index: 0,
                            current_step_name: "step1".to_string(),
                            current_session_id: Some("step-session-1".to_string()),
                            total_steps: 1,
                            step_history: vec![],
                            step_execution_counts: std::collections::HashMap::new(),
                            step_outputs: std::collections::HashMap::new(),
                            workflow_definition: crate::domain::workflow::WorkflowDefinition {
                                variables: Default::default(),
                                name: "test".to_string(),
                                description: "test".to_string(),
                                builtin: false,
                                nodes: vec![],
                            },
                            total_token_usage: crate::domain::workflow::TokenUsage::default(),
                            workflow_variables: std::collections::HashMap::new(),
                            step_states: std::collections::HashMap::new(),
                            active_parallel_steps: vec![],
                            approval_operations: None,
                            started_at: 1000.0,
                            updated_at: 1000.0,
                        },
                    ),
                    std::collections::HashMap::new(),
                ),
            })),
            WsMessage::AgentStreamSync(AgentStreamSync {
                session_id: "sess-1".to_string(),
                message_id: "a-1".to_string(),
                parts: vec![],
            }),
            WsMessage::Error(ErrorMsg {
                code: "E".to_string(),
                message: "M".to_string(),
            }),
        ];

        for msg in &messages {
            let json = serialize_message(msg).unwrap();
            let back = deserialize_message(&json).unwrap();
            let json2 = serialize_message(&back).unwrap();
            assert_eq!(json, json2, "roundtrip failed for: {json}");
        }
    }
}
