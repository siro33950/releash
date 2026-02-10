mod auth;
mod branch;
mod comment;
mod error;
mod file;
mod git;
mod pty;
mod worktree;

pub use auth::*;
pub use branch::*;
pub use comment::*;
pub use error::*;
pub use file::*;
pub use git::*;
pub use pty::*;
pub use worktree::*;

use serde::{Deserialize, Serialize};

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
    #[serde(rename = "pty_output_request")]
    PtyOutputRequest(PtyOutputRequest),

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
    #[serde(rename = "git_stage_hunk")]
    GitStageHunk(GitStageHunk),
    #[serde(rename = "git_commit_request")]
    GitCommitRequest(GitCommitRequest),
    #[serde(rename = "git_commit_result")]
    GitCommitResult(GitCommitResult),
    #[serde(rename = "git_push_request")]
    GitPushRequest(GitPushRequest),
    #[serde(rename = "git_push_result")]
    GitPushResult(GitPushResult),
    #[serde(rename = "branch_info_request")]
    BranchInfoRequest(BranchInfoRequest),
    #[serde(rename = "branch_info_response")]
    BranchInfoResponse(BranchInfoResponse),

    // コメント
    #[serde(rename = "add_comment")]
    AddComment(AddComment),
    #[serde(rename = "comments_sync")]
    CommentsSync(CommentSync),

    // PTYスポーン
    #[serde(rename = "pty_spawn_request")]
    PtySpawnRequest(PtySpawnRequest),
    #[serde(rename = "pty_spawn_response")]
    PtySpawnResponse(PtySpawnResponse),

    // Worktree
    #[serde(rename = "worktree_list_request")]
    WorktreeListRequest(WorktreeListRequest),
    #[serde(rename = "worktree_list_response")]
    WorktreeListResponse(WorktreeListResponse),
    #[serde(rename = "worktree_select_request")]
    WorktreeSelectRequest(WorktreeSelectRequest),
    #[serde(rename = "worktree_select_response")]
    WorktreeSelectResponse(WorktreeSelectResponse),

    // ブランチリスト同期
    #[serde(rename = "branch_list_sync")]
    BranchListSync(BranchListSync),

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
            staged: None,
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
    fn file_content_request_default_diff_base() {
        let json = r#"{"type":"file_content_request","payload":{"path":"test.rs"}}"#;
        let msg = deserialize_message(json).unwrap();
        match msg {
            WsMessage::FileContentRequest(req) => {
                assert_eq!(req.path, "test.rs");
                assert_eq!(req.diff_base, "HEAD");
            }
            _ => panic!("unexpected variant"),
        }
    }

    #[test]
    fn file_content_request_staged_diff_base() {
        let json =
            r#"{"type":"file_content_request","payload":{"path":"a.rs","diff_base":"staged"}}"#;
        let msg = deserialize_message(json).unwrap();
        match msg {
            WsMessage::FileContentRequest(req) => {
                assert_eq!(req.diff_base, "staged");
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
    fn roundtrip_git_stage_hunk() {
        let msg = WsMessage::GitStageHunk(GitStageHunk {
            patch: "--- a/f\n+++ b/f\n".to_string(),
        });
        let json = serialize_message(&msg).unwrap();
        let deserialized = deserialize_message(&json).unwrap();
        match deserialized {
            WsMessage::GitStageHunk(h) => {
                assert_eq!(h.patch, "--- a/f\n+++ b/f\n");
            }
            _ => panic!("unexpected variant"),
        }
    }

    #[test]
    fn file_content_response_omits_none_staged() {
        let msg = WsMessage::FileContentResponse(FileContentResponse {
            path: "f".to_string(),
            original: "".to_string(),
            modified: "".to_string(),
            staged: None,
        });
        let json = serialize_message(&msg).unwrap();
        assert!(!json.contains("\"staged\""));
    }

    #[test]
    fn file_content_response_includes_staged() {
        let msg = WsMessage::FileContentResponse(FileContentResponse {
            path: "f".to_string(),
            original: "a".to_string(),
            modified: "b".to_string(),
            staged: Some("s".to_string()),
        });
        let json = serialize_message(&msg).unwrap();
        assert!(json.contains("\"staged\":\"s\""));
        let deserialized = deserialize_message(&json).unwrap();
        match deserialized {
            WsMessage::FileContentResponse(r) => {
                assert_eq!(r.staged.unwrap(), "s");
            }
            _ => panic!("unexpected variant"),
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
            WsMessage::PtyOutputRequest(PtyOutputRequest { pty_id: 1 }),
            WsMessage::GitStatusSync(GitStatusSync { files: vec![] }),
            WsMessage::FileContentRequest(FileContentRequest {
                path: "f".to_string(),
                diff_base: "HEAD".to_string(),
            }),
            WsMessage::FileContentResponse(FileContentResponse {
                path: "f".to_string(),
                original: "".to_string(),
                modified: "".to_string(),
                staged: None,
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
            WsMessage::GitStageHunk(GitStageHunk {
                patch: "p".to_string(),
            }),
            WsMessage::GitCommitRequest(GitCommitRequest {
                message: "msg".to_string(),
            }),
            WsMessage::GitCommitResult(GitCommitResult {
                success: true,
                hash: Some("abc123".to_string()),
                error: None,
            }),
            WsMessage::GitPushRequest(GitPushRequest {}),
            WsMessage::GitPushResult(GitPushResult {
                success: true,
                output: Some("ok".to_string()),
                error: None,
            }),
            WsMessage::BranchInfoRequest(BranchInfoRequest {}),
            WsMessage::BranchInfoResponse(BranchInfoResponse {
                branch: "main".to_string(),
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
            WsMessage::PtySpawnRequest(PtySpawnRequest { cols: 80, rows: 24 }),
            WsMessage::PtySpawnResponse(PtySpawnResponse {
                success: true,
                pty_id: Some(1),
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
                    has_pr: false,
                    pr_number: None,
                    pr_url: None,
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
                    is_default: false,
                    worktree_path: Some("/repo-worktrees/feature-test".to_string()),
                    dirty_count: 2,
                    is_merged: false,
                    has_pr: true,
                    pr_number: Some(42),
                    pr_url: Some("https://github.com/owner/repo/pull/42".to_string()),
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
