#![allow(dead_code)]

use serde::{Deserialize, Serialize};

// --- 認証 ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthChallenge {
    pub challenge: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthResponse {
    pub hmac: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

// --- ターミナル ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtyOutputMsg {
    pub pty_id: u64,
    pub data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtyExitMsg {
    pub pty_id: u64,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtyInput {
    pub pty_id: u64,
    pub data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtyResize {
    pub pty_id: u64,
    pub rows: u16,
    pub cols: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtyReady {
    pub pty_id: u64,
    pub cols: u16,
    pub rows: u16,
}

// --- ファイル・Diff ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitFileStatusMsg {
    pub path: String,
    pub index_status: String,
    pub worktree_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitStatusSync {
    pub files: Vec<GitFileStatusMsg>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileContentRequest {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileContentResponse {
    pub path: String,
    pub original: String,
    pub modified: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    pub kind: String,
}

// --- Git操作 ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitStatusRequest {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitStage {
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitUnstage {
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitStageResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub files: Vec<GitFileStatusMsg>,
}

// --- コメント ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddComment {
    pub file_path: String,
    pub line_number: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_line: Option<u32>,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommentItem {
    pub id: String,
    pub file_path: String,
    pub line_number: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_line: Option<u32>,
    pub content: String,
    pub status: String,
    pub created_at: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommentSync {
    pub comments: Vec<CommentItem>,
}

// --- 制御 ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorMsg {
    pub code: String,
    pub message: String,
}

// --- 統合メッセージ型 ---

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
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

    // ファイル・Diff
    #[serde(rename = "git_status_sync")]
    GitStatusSync(GitStatusSync),
    #[serde(rename = "file_content_request")]
    FileContentRequest(FileContentRequest),
    #[serde(rename = "file_content_response")]
    FileContentResponse(FileContentResponse),
    #[serde(rename = "file_change")]
    FileChange(FileChange),

    // Git操作
    #[serde(rename = "git_status_request")]
    GitStatusRequest(GitStatusRequest),
    #[serde(rename = "git_stage")]
    GitStage(GitStage),
    #[serde(rename = "git_unstage")]
    GitUnstage(GitUnstage),
    #[serde(rename = "git_stage_result")]
    GitStageResult(GitStageResult),

    // コメント
    #[serde(rename = "add_comment")]
    AddComment(AddComment),
    #[serde(rename = "comments_sync")]
    CommentsSync(CommentSync),

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
    fn roundtrip_git_status_sync() {
        let msg = WsMessage::GitStatusSync(GitStatusSync {
            files: vec![
                GitFileStatusMsg {
                    path: "src/main.rs".to_string(),
                    index_status: "modified".to_string(),
                    worktree_status: "none".to_string(),
                },
                GitFileStatusMsg {
                    path: "README.md".to_string(),
                    index_status: "none".to_string(),
                    worktree_status: "new".to_string(),
                },
            ],
        });
        let json = serialize_message(&msg).unwrap();
        let deserialized = deserialize_message(&json).unwrap();
        match deserialized {
            WsMessage::GitStatusSync(s) => assert_eq!(s.files.len(), 2),
            _ => panic!("unexpected variant"),
        }
    }

    #[test]
    fn roundtrip_file_content_response() {
        let msg = WsMessage::FileContentResponse(FileContentResponse {
            path: "lib.rs".to_string(),
            original: "fn old() {}".to_string(),
            modified: "fn new() {}".to_string(),
        });
        let json = serialize_message(&msg).unwrap();
        let deserialized = deserialize_message(&json).unwrap();
        match deserialized {
            WsMessage::FileContentResponse(f) => {
                assert_eq!(f.path, "lib.rs");
                assert_eq!(f.original, "fn old() {}");
                assert_eq!(f.modified, "fn new() {}");
            }
            _ => panic!("unexpected variant"),
        }
    }

    #[test]
    fn roundtrip_git_stage_result() {
        let msg = WsMessage::GitStageResult(GitStageResult {
            success: true,
            error: None,
            files: vec![GitFileStatusMsg {
                path: "a.txt".to_string(),
                index_status: "new".to_string(),
                worktree_status: "none".to_string(),
            }],
        });
        let json = serialize_message(&msg).unwrap();
        let deserialized = deserialize_message(&json).unwrap();
        match deserialized {
            WsMessage::GitStageResult(r) => {
                assert!(r.success);
                assert!(r.error.is_none());
                assert_eq!(r.files.len(), 1);
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
            }),
            WsMessage::GitStatusSync(GitStatusSync { files: vec![] }),
            WsMessage::FileContentRequest(FileContentRequest {
                path: "f".to_string(),
            }),
            WsMessage::FileContentResponse(FileContentResponse {
                path: "f".to_string(),
                original: "".to_string(),
                modified: "".to_string(),
            }),
            WsMessage::FileChange(FileChange {
                path: "f".to_string(),
                kind: "modify".to_string(),
            }),
            WsMessage::GitStatusRequest(GitStatusRequest {}),
            WsMessage::GitStage(GitStage {
                paths: vec!["a".to_string()],
            }),
            WsMessage::GitUnstage(GitUnstage {
                paths: vec!["b".to_string()],
            }),
            WsMessage::GitStageResult(GitStageResult {
                success: true,
                error: None,
                files: vec![],
            }),
            WsMessage::AddComment(AddComment {
                file_path: "src/main.rs".to_string(),
                line_number: 10,
                end_line: None,
                content: "fix this".to_string(),
            }),
            WsMessage::CommentsSync(CommentSync {
                comments: vec![CommentItem {
                    id: "c1".to_string(),
                    file_path: "src/main.rs".to_string(),
                    line_number: 10,
                    end_line: None,
                    content: "fix this".to_string(),
                    status: "unsent".to_string(),
                    created_at: 1234567890.0,
                }],
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
