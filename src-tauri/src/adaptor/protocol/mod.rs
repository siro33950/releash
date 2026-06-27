//! 外部入口（Tauri コマンド引数・WebSocket メッセージ）のメッセージ型。
//!
//! ドメイン型でも DTO でもない（[`CONTROLLER.md`] 参照）。フロントから受け取る転送表現を
//! 受理し、controller が対応するドメイン値オブジェクトへ変換する。

pub(crate) mod agent;
pub(crate) mod auth;
pub(crate) mod branch;
pub(crate) mod code;
pub(crate) mod error;
pub(crate) mod mention;
pub(crate) mod pty;
pub(crate) mod workflow;
pub(crate) mod worktree;

pub(crate) use agent::*;
pub(crate) use auth::*;
pub(crate) use branch::*;
pub(crate) use error::*;
pub(crate) use worktree::*;

use crate::adaptor::protocol::pty::{PtyEvictedMsg, PtyExitMsg, PtyOutputMsg};
use crate::adaptor::protocol::workflow::WorkflowStateSync;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
#[allow(clippy::large_enum_variant)]
pub(crate) enum WsMessage {
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
    #[serde(rename = "pty_evicted")]
    PtyEvicted(PtyEvictedMsg),

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
    #[serde(rename = "agent_stream_delta")]
    AgentStreamDelta(AgentStreamDeltaMsg),
    #[serde(rename = "resync_stream")]
    ResyncStream(ResyncStreamReq),

    // 制御
    #[serde(rename = "error")]
    Error(ErrorMsg),
}

pub(crate) fn serialize_message(msg: &WsMessage) -> Result<String, String> {
    serde_json::to_string(msg).map_err(|e| format!("シリアライズ失敗: {e}"))
}

pub(crate) fn deserialize_message(json: &str) -> Result<WsMessage, String> {
    serde_json::from_str(json).map_err(|e| format!("デシリアライズ失敗: {e}"))
}

#[cfg(test)]
mod adaptor_protocol_tests {
    use super::*;
    use crate::adaptor::protocol::pty::{
        PtyEvictReasonMsg, PtyEvictedMsg, PtyExitMsg, PtyOutputMsg,
    };

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

    fn ws_message_wire_tag(msg: &WsMessage) -> &'static str {
        match msg {
            WsMessage::AuthChallenge(_) => "auth_challenge",
            WsMessage::AuthResponse(_) => "auth_response",
            WsMessage::AuthResult(_) => "auth_result",
            WsMessage::PtyOutput(_) => "pty_output",
            WsMessage::PtyExit(_) => "pty_exit",
            WsMessage::PtyEvicted(_) => "pty_evicted",
            WsMessage::WorktreePrStatusSync(_) => "worktree_pr_status_sync",
            WsMessage::BranchListSync(_) => "branch_list_sync",
            WsMessage::AgentStateSync(_) => "agent_state_sync",
            WsMessage::WorkflowStateSync(_) => "workflow_state_sync",
            WsMessage::AgentStreamSync(_) => "agent_stream_sync",
            WsMessage::AgentStreamDelta(_) => "agent_stream_delta",
            WsMessage::ResyncStream(_) => "resync_stream",
            WsMessage::Error(_) => "error",
        }
    }

    #[test]
    fn all_variants_match_golden_wire_json() {
        let cases = [
            (
                "auth_challenge",
                r#"{"type":"auth_challenge","payload":{"challenge":"x"}}"#,
            ),
            (
                "auth_response",
                r#"{"type":"auth_response","payload":{"hmac":"y"}}"#,
            ),
            (
                "auth_result",
                r#"{"type":"auth_result","payload":{"success":false,"message":"invalid token"}}"#,
            ),
            (
                "pty_output",
                r#"{"type":"pty_output","payload":{"pty_id":1,"data":"d","sequence":1}}"#,
            ),
            (
                "pty_exit",
                r#"{"type":"pty_exit","payload":{"pty_id":1,"exit_code":0}}"#,
            ),
            (
                "pty_evicted",
                r#"{"type":"pty_evicted","payload":{"pty_id":1,"session_key":"key","reason":"idle"}}"#,
            ),
            (
                "worktree_pr_status_sync",
                r#"{"type":"worktree_pr_status_sync","payload":{"entries":[{"path":"/repo","pr_number":12,"pr_url":"https://example.test/pr/12"}]}}"#,
            ),
            (
                "branch_list_sync",
                r#"{"type":"branch_list_sync","payload":{"branches":[{"name":"feature/test","is_main_worktree":false,"worktree_path":"/repo-worktrees/feature-test","dirty_count":2,"is_merged":false,"ahead":3,"behind":1,"has_upstream":true,"base_ahead":0}]}}"#,
            ),
            (
                "agent_state_sync",
                r#"{"type":"agent_state_sync","payload":{"worktree_path":"/repo","state":"running","exit_code":0,"timestamp":1234567890.0,"session_id":"sess-1","pty_id":"pty-1"}}"#,
            ),
            (
                "workflow_state_sync",
                r#"{"type":"workflow_state_sync","payload":{"worktreePath":"/repo","workflowState":{"executionId":"exec-1","workflowName":"test","state":{"type":"running"},"currentStepIndex":0,"currentStepName":"step1","currentSessionId":"step-session-1","totalSteps":1,"stepHistory":[],"stepExecutionCounts":{},"workflowDefinition":{"name":"test","description":"test","builtin":false,"nodes":[]},"totalTokenUsage":{"inputTokens":0,"outputTokens":0},"stepStates":{},"stepOutputs":{},"startedAt":1000.0,"updatedAt":1000.0}}}"#,
            ),
            (
                "agent_stream_sync",
                r#"{"type":"agent_stream_sync","payload":{"session_id":"sess-1","message_id":"a-1","seq":7,"parts":[{"type":"text","content":"hello","parentToolUseId":"tool-parent"}]}}"#,
            ),
            (
                "agent_stream_delta",
                r#"{"type":"agent_stream_delta","payload":{"session_id":"sess-1","message_id":"a-1","seq":8,"parts":[{"type":"tool_result","content":"preview","isError":false,"toolUseId":"tool-1","parentToolUseId":"parent-1","contentRef":{"id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","byteSize":4096},"summary":{"lineCount":10,"byteSize":4096,"isError":false,"truncated":true}}]}}"#,
            ),
            (
                "resync_stream",
                r#"{"type":"resync_stream","payload":{"session_id":"sess-1","message_id":"a-1","since_seq":7}}"#,
            ),
            (
                "error",
                r#"{"type":"error","payload":{"code":"E","message":"M"}}"#,
            ),
        ];
        assert_eq!(cases.len(), 14);

        for (expected_tag, golden) in cases {
            let msg = deserialize_message(golden)
                .unwrap_or_else(|err| panic!("deserialize golden {expected_tag}: {err}"));
            assert_eq!(ws_message_wire_tag(&msg), expected_tag);

            let serialized = serialize_message(&msg).unwrap();
            let actual: serde_json::Value = serde_json::from_str(&serialized).unwrap();
            let expected: serde_json::Value = serde_json::from_str(golden).unwrap();
            assert_eq!(actual, expected, "golden mismatch for {expected_tag}");
        }
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
                sequence: 1,
            }),
            WsMessage::PtyExit(PtyExitMsg {
                pty_id: 1,
                exit_code: Some(0),
            }),
            WsMessage::PtyEvicted(PtyEvictedMsg {
                pty_id: 1,
                session_key: "key".to_string(),
                reason: PtyEvictReasonMsg::Idle,
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
                seq: 7,
                parts: vec![],
            }),
            WsMessage::AgentStreamDelta(AgentStreamDeltaMsg {
                session_id: "sess-1".to_string(),
                message_id: "a-1".to_string(),
                seq: 8,
                parts: vec![],
            }),
            WsMessage::ResyncStream(ResyncStreamReq {
                session_id: "sess-1".to_string(),
                message_id: "a-1".to_string(),
                since_seq: 7,
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
