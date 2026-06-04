mod agent;
mod auth;
mod branch;
mod error;
mod pty;
mod review;
mod workflow;
mod worktree;

pub use agent::*;
pub use auth::*;
pub use branch::*;
pub use error::*;
pub use pty::*;
pub use review::*;
pub use workflow::*;
pub use worktree::*;

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

    // ターミナル
    #[serde(rename = "pty_output")]
    PtyOutput(PtyOutputMsg),
    #[serde(rename = "pty_exit")]
    PtyExit(PtyExitMsg),
    #[serde(rename = "pty_input")]
    PtyInput(PtyInput),
    #[serde(rename = "pty_resize")]
    PtyResize(PtyResize),
    #[serde(rename = "pty_ready")]
    PtyReady(PtyReady),
    #[serde(rename = "pty_output_request")]
    PtyOutputRequest(PtyOutputRequest),

    // ブランチ情報
    #[serde(rename = "branch_info_request")]
    BranchInfoRequest(BranchInfoRequest),
    #[serde(rename = "branch_info_response")]
    BranchInfoResponse(BranchInfoResponse),

    // PTYスポーン
    #[serde(rename = "pty_spawn_request")]
    PtySpawnRequest(PtySpawnRequest),
    #[serde(rename = "pty_spawn_response")]
    PtySpawnResponse(PtySpawnResponse),

    // PTY Kill
    #[serde(rename = "pty_kill_request")]
    PtyKillRequest(PtyKillRequest),
    #[serde(rename = "pty_kill_response")]
    PtyKillResponse(PtyKillResponse),

    // Worktree
    #[serde(rename = "worktree_list_request")]
    WorktreeListRequest(WorktreeListRequest),
    #[serde(rename = "worktree_list_response")]
    WorktreeListResponse(WorktreeListResponse),
    #[serde(rename = "worktree_select_request")]
    WorktreeSelectRequest(WorktreeSelectRequest),
    #[serde(rename = "worktree_select_response")]
    WorktreeSelectResponse(WorktreeSelectResponse),
    #[serde(rename = "worktree_pr_status_sync")]
    WorktreePrStatusSync(WorktreePrStatusSync),

    // ブランチリスト同期
    #[serde(rename = "branch_list_sync")]
    BranchListSync(BranchListSync),

    // エージェント状態
    #[serde(rename = "agent_state_sync")]
    AgentStateSync(AgentStateSync),

    // ワークフロー状態
    #[serde(rename = "workflow_state_sync")]
    WorkflowStateSync(Box<WorkflowStateSync>),

    // バックエンド一覧
    #[serde(rename = "backend_list_request")]
    BackendListRequest(BackendListRequest),
    #[serde(rename = "backend_list_response")]
    BackendListResponse(BackendListResponse),

    // エージェントセッション
    #[serde(rename = "agent_session_start_request")]
    AgentSessionStartRequest(AgentSessionStartRequest),
    #[serde(rename = "agent_session_start_response")]
    AgentSessionStartResponse(AgentSessionStartResponse),
    #[serde(rename = "agent_message_request")]
    AgentMessageRequest(AgentMessageRequest),
    #[serde(rename = "agent_message_response")]
    AgentMessageResponse(AgentMessageResponse),
    #[serde(rename = "agent_interrupt_request")]
    AgentInterruptRequest(AgentInterruptRequest),
    #[serde(rename = "agent_interrupt_response")]
    AgentInterruptResponse(AgentInterruptResponse),
    #[serde(rename = "agent_model_set_request")]
    AgentModelSetRequest(AgentModelSetRequest),
    #[serde(rename = "agent_model_set_response")]
    AgentModelSetResponse(AgentModelSetResponse),
    #[serde(rename = "agent_permission_mode_set_request")]
    AgentPermissionModeSetRequest(AgentPermissionModeSetRequest),
    #[serde(rename = "agent_permission_mode_set_response")]
    AgentPermissionModeSetResponse(AgentPermissionModeSetResponse),
    #[serde(rename = "agent_stream_sync")]
    AgentStreamSync(AgentStreamSync),

    // Review comments
    #[serde(rename = "review_list_request")]
    ReviewListRequest(ReviewListRequest),
    #[serde(rename = "review_list_response")]
    ReviewListResponse(ReviewListResponse),
    #[serde(rename = "review_get_request")]
    ReviewGetRequest(ReviewGetRequest),
    #[serde(rename = "review_thread_response")]
    ReviewThreadResponse(ReviewThreadResponse),
    #[serde(rename = "review_create_request")]
    ReviewCreateRequest(ReviewCreateRequest),
    #[serde(rename = "review_append_comment_request")]
    ReviewAppendCommentRequest(ReviewAppendCommentRequest),
    #[serde(rename = "review_resolve_request")]
    ReviewResolveRequest(ReviewResolveRequest),
    #[serde(rename = "review_history_request")]
    ReviewHistoryRequest(ReviewHistoryRequest),
    #[serde(rename = "review_history_response")]
    ReviewHistoryResponse(ReviewHistoryResponse),

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
    fn roundtrip_pty_output() {
        let msg = WsMessage::PtyOutput(PtyOutputMsg {
            pty_id: 42,
            data: "hello\x1b[31mworld".to_string(),
        });
        let json = serialize_message(&msg).unwrap();
        let deserialized = deserialize_message(&json).unwrap();
        match deserialized {
            WsMessage::PtyOutput(p) => {
                assert_eq!(p.pty_id, 42);
                assert!(p.data.contains("hello"));
            }
            _ => panic!("unexpected variant"),
        }
    }

    #[test]
    fn roundtrip_pty_exit_with_null_exit_code() {
        let msg = WsMessage::PtyExit(PtyExitMsg {
            pty_id: 1,
            exit_code: None,
        });
        let json = serialize_message(&msg).unwrap();
        let deserialized = deserialize_message(&json).unwrap();
        match deserialized {
            WsMessage::PtyExit(p) => {
                assert_eq!(p.pty_id, 1);
                assert!(p.exit_code.is_none());
            }
            _ => panic!("unexpected variant"),
        }
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
            WsMessage::PtyInput(PtyInput {
                pty_id: 1,
                data: "i".to_string(),
            }),
            WsMessage::PtyResize(PtyResize {
                pty_id: 1,
                rows: 24,
                cols: 80,
            }),
            WsMessage::PtyReady(PtyReady {
                pty_id: 1,
                cols: 80,
                rows: 24,
                label: None,
                worktree_path: None,
            }),
            WsMessage::PtyOutputRequest(PtyOutputRequest { pty_id: 1 }),
            WsMessage::BranchInfoRequest(BranchInfoRequest {}),
            WsMessage::BranchInfoResponse(BranchInfoResponse {
                branch: "main".to_string(),
            }),
            WsMessage::PtySpawnRequest(PtySpawnRequest {
                cols: 80,
                rows: 24,
                label: None,
            }),
            WsMessage::PtySpawnResponse(PtySpawnResponse {
                success: true,
                pty_id: Some(1),
                error: None,
            }),
            WsMessage::PtyKillRequest(PtyKillRequest { pty_id: 1 }),
            WsMessage::PtyKillResponse(PtyKillResponse {
                success: true,
                pty_id: 1,
                error: None,
            }),
            WsMessage::WorktreeListRequest(WorktreeListRequest {}),
            WsMessage::WorktreeListResponse(WorktreeListResponse {
                worktrees: vec![WorktreeEntryMsg {
                    name: "main".to_string(),
                    path: "/repo".to_string(),
                    branch: "main".to_string(),
                    is_main: true,
                    is_locked: false,
                    dirty_count: 0,
                    base_branch: None,
                    repo_path: Some("/repo".to_string()),
                }],
            }),
            WsMessage::WorktreeSelectRequest(WorktreeSelectRequest {
                path: "/repo".to_string(),
            }),
            WsMessage::WorktreeSelectResponse(WorktreeSelectResponse {
                success: true,
                path: "/repo".to_string(),
                error: None,
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
            WsMessage::AgentStateSync(AgentStateSync {
                worktree_path: "/repo".to_string(),
                state: AgentState::Waiting,
                exit_code: None,
                timestamp: 1234567890.0,
                session_id: None,
                pty_id: Some("42".to_string()),
            }),
            WsMessage::WorkflowStateSync(Box::new(WorkflowStateSync {
                worktree_path: "/repo".to_string(),
                workflow_state: crate::protocol::WorkflowStateView::from_parts(
                    crate::workflow_state_presenter::workflow_state_to_view(
                        crate::workflow::state::WorkflowState {
                            execution_id: "exec-1".to_string(),
                            workflow_name: "test".to_string(),
                            state: crate::workflow::state::WorkflowExecutionState::Running,
                            current_step_index: 0,
                            current_step_name: "step1".to_string(),
                            current_session_id: Some("step-session-1".to_string()),
                            total_steps: 1,
                            step_history: vec![],
                            step_execution_counts: std::collections::HashMap::new(),
                            step_outputs: std::collections::HashMap::new(),
                            workflow_definition: crate::workflow::schema::Workflow {
                                variables: Default::default(),
                                name: "test".to_string(),
                                description: "test".to_string(),
                                builtin: false,
                                nodes: vec![],
                            },
                            total_token_usage: crate::workflow::state::TokenUsage::default(),
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
            WsMessage::BackendListRequest(BackendListRequest {}),
            WsMessage::BackendListResponse(BackendListResponse {
                backends: vec![BackendInfoMsg {
                    id: "claude".to_string(),
                    name: "Claude".to_string(),
                    available: true,
                    available_models: vec![],
                }],
                default_id: Some("claude".to_string()),
            }),
            WsMessage::AgentSessionStartRequest(AgentSessionStartRequest {
                worktree_path: "/repo".to_string(),
                backend_id: Some("claude".to_string()),
                permission_mode: Some("edit".to_string()),
            }),
            WsMessage::AgentSessionStartResponse(AgentSessionStartResponse {
                success: true,
                session_id: Some("sess-1".to_string()),
                backend_id: Some("claude".to_string()),
                error: None,
            }),
            WsMessage::AgentMessageRequest(AgentMessageRequest {
                session_id: Some("sess-1".to_string()),
                worktree_path: "/repo".to_string(),
                content: "hello".to_string(),
                permission_mode: Some("edit".to_string()),
                backend_id: Some("claude".to_string()),
            }),
            WsMessage::AgentMessageResponse(AgentMessageResponse {
                success: true,
                session_id: Some("sess-1".to_string()),
                human_message_id: Some("h-1".to_string()),
                agent_message_id: Some("a-1".to_string()),
                backend_id: Some("claude".to_string()),
                error: None,
            }),
            WsMessage::AgentInterruptRequest(AgentInterruptRequest {
                session_id: "sess-1".to_string(),
            }),
            WsMessage::AgentInterruptResponse(AgentInterruptResponse {
                success: true,
                session_id: "sess-1".to_string(),
                error: None,
            }),
            WsMessage::AgentModelSetRequest(AgentModelSetRequest {
                session_id: "sess-1".to_string(),
                model_id: "gpt-5.4".to_string(),
            }),
            WsMessage::AgentModelSetResponse(AgentModelSetResponse {
                success: true,
                session_id: "sess-1".to_string(),
                model_id: Some("gpt-5.4".to_string()),
                error: None,
            }),
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

    #[test]
    fn roundtrip_pty_spawn_request_with_label() {
        let msg = WsMessage::PtySpawnRequest(PtySpawnRequest {
            cols: 120,
            rows: 40,
            label: Some("dev-server".to_string()),
        });
        let json = serialize_message(&msg).unwrap();
        assert!(json.contains("\"label\":\"dev-server\""));
        let back = deserialize_message(&json).unwrap();
        match back {
            WsMessage::PtySpawnRequest(r) => {
                assert_eq!(r.label.unwrap(), "dev-server");
            }
            _ => panic!("unexpected variant"),
        }
    }

    #[test]
    fn roundtrip_pty_ready_with_label_and_worktree() {
        let msg = WsMessage::PtyReady(PtyReady {
            pty_id: 5,
            cols: 80,
            rows: 24,
            label: Some("build".to_string()),
            worktree_path: Some("/repo/wt".to_string()),
        });
        let json = serialize_message(&msg).unwrap();
        assert!(json.contains("\"label\":\"build\""));
        assert!(json.contains("\"worktree_path\":\"/repo/wt\""));
        let back = deserialize_message(&json).unwrap();
        match back {
            WsMessage::PtyReady(r) => {
                assert_eq!(r.label.unwrap(), "build");
                assert_eq!(r.worktree_path.unwrap(), "/repo/wt");
            }
            _ => panic!("unexpected variant"),
        }
    }

    #[test]
    fn backward_compat_pty_spawn_request_without_label() {
        let json = r#"{"type":"pty_spawn_request","payload":{"cols":80,"rows":24}}"#;
        let msg = deserialize_message(json).unwrap();
        match msg {
            WsMessage::PtySpawnRequest(r) => {
                assert_eq!(r.cols, 80);
                assert_eq!(r.rows, 24);
                assert!(r.label.is_none());
            }
            _ => panic!("unexpected variant"),
        }
    }

    #[test]
    fn backward_compat_pty_ready_without_label() {
        let json = r#"{"type":"pty_ready","payload":{"pty_id":1,"cols":80,"rows":24}}"#;
        let msg = deserialize_message(json).unwrap();
        match msg {
            WsMessage::PtyReady(r) => {
                assert_eq!(r.pty_id, 1);
                assert!(r.label.is_none());
                assert!(r.worktree_path.is_none());
            }
            _ => panic!("unexpected variant"),
        }
    }

    #[test]
    fn roundtrip_pty_kill_request() {
        let msg = WsMessage::PtyKillRequest(PtyKillRequest { pty_id: 42 });
        let json = serialize_message(&msg).unwrap();
        let back = deserialize_message(&json).unwrap();
        match back {
            WsMessage::PtyKillRequest(r) => assert_eq!(r.pty_id, 42),
            _ => panic!("unexpected variant"),
        }
    }

    #[test]
    fn roundtrip_pty_kill_response() {
        let msg = WsMessage::PtyKillResponse(PtyKillResponse {
            success: true,
            pty_id: 42,
            error: None,
        });
        let json = serialize_message(&msg).unwrap();
        assert!(!json.contains("\"error\""));
        let back = deserialize_message(&json).unwrap();
        match back {
            WsMessage::PtyKillResponse(r) => {
                assert!(r.success);
                assert_eq!(r.pty_id, 42);
                assert!(r.error.is_none());
            }
            _ => panic!("unexpected variant"),
        }
    }

    #[test]
    fn review_history_json_uses_dto_shape_and_camel_case_fields() {
        let msg = WsMessage::ReviewHistoryResponse(ReviewHistoryResponse {
            success: true,
            worktree_name: Some("wt".to_string()),
            events: vec![ReviewHistoryEntry::ThreadCreated {
                id: "event-1".to_string(),
                thread_id: "thread-1".to_string(),
                comment_id: "comment-1".to_string(),
                actor: crate::review_comments::ReviewActorDto::human(),
                target: ReviewTarget {
                    file_path: Some("src/main.rs".to_string()),
                    line_number: Some(12),
                    end_line: None,
                },
                content: "Check this".to_string(),
                at: 1.0,
            }],
            error: None,
        });

        let json = serialize_message(&msg).unwrap();

        assert!(json.contains("\"kind\":\"thread_created\""));
        assert!(json.contains("\"worktreeName\":\"wt\""));
        assert!(json.contains("\"id\":\"event-1\""));
        assert!(json.contains("\"threadId\":\"thread-1\""));
        assert!(json.contains("\"commentId\":\"comment-1\""));
        assert!(json.contains("\"filePath\":\"src/main.rs\""));
        assert!(json.contains("\"lineNumber\":12"));
        assert!(!json.contains("\"thread_id\""));
        assert!(!json.contains("\"sessionId\""));
    }
}
