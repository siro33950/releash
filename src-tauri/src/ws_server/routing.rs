use std::sync::Arc;

use tokio::sync::Mutex;

use crate::protocol::*;

use super::handlers::*;
use super::WsServerState;

pub(super) async fn route_message(
    msg: &WsMessage,
    state: &WsServerState,
    selected_worktree: &Arc<Mutex<Option<String>>>,
) -> Option<WsMessage> {
    match msg {
        WsMessage::PtyInput(input) => handle_pty_input(input, state),
        WsMessage::PtyResize(_) => None,
        WsMessage::GitStatusRequest(_) => handle_git_status_request(selected_worktree).await,
        WsMessage::FileContentRequest(req) => handle_file_content_req(req, selected_worktree).await,
        WsMessage::GitStage(req) => handle_git_stage_request(req, state, selected_worktree).await,
        WsMessage::GitUnstage(req) => {
            handle_git_unstage_request(req, state, selected_worktree).await
        }
        WsMessage::GitStageHunk(req) => {
            handle_git_stage_hunk_request(req, state, selected_worktree).await
        }
        WsMessage::PtySpawnRequest(req) => {
            handle_pty_spawn_request(req, state, selected_worktree).await
        }
        WsMessage::PtyOutputRequest(req) => handle_pty_output_request(req, state),
        WsMessage::PtyKillRequest(req) => handle_pty_kill_request(req, state).await,
        WsMessage::GitCommitRequest(req) => {
            handle_git_commit_request(req, state, selected_worktree).await
        }
        WsMessage::GitPushRequest(_) => handle_git_push_request(selected_worktree).await,
        WsMessage::BranchInfoRequest(_) => handle_branch_info_request(selected_worktree).await,
        WsMessage::WorktreeListRequest(_) => handle_worktree_list_request(state).await,
        WsMessage::WorktreeSelectRequest(req) => {
            handle_worktree_select_request(req, state, selected_worktree).await
        }
        WsMessage::AddComment(comment) => handle_add_comment(comment, state),
        WsMessage::DeleteComment(req) => handle_delete_comment(req, state),
        WsMessage::UpdateComment(req) => handle_update_comment(req, state),
        _ => Some(WsMessage::Error(ErrorMsg {
            code: "INVALID_MESSAGE".to_string(),
            message: "Unexpected message from client".to_string(),
        })),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tokio::sync::Mutex;

    use crate::config::AppConfig;
    use crate::protocol::*;
    use crate::ws_bridge::WsBroadcaster;
    use crate::ws_server::WsServerState;

    use super::route_message;

    fn test_state() -> WsServerState {
        let config = crate::config::ReleashConfig::default();
        let app_config = Arc::new(AppConfig::new(
            config,
            std::path::PathBuf::from("/tmp/test-releash.toml"),
        ));
        WsServerState::new(
            None,
            Arc::new(WsBroadcaster::default()),
            None,
            vec![],
            app_config,
            None,
            false,
            Arc::new(crate::git_host::PrCache::new()),
        )
    }

    fn test_selected_worktree() -> Arc<Mutex<Option<String>>> {
        Arc::new(Mutex::new(None))
    }

    #[tokio::test]
    async fn test_route_unknown_message_returns_error() {
        let state = test_state();
        let wt = test_selected_worktree();
        let msg = WsMessage::AuthChallenge(AuthChallenge {
            challenge: "x".to_string(),
        });
        let result = route_message(&msg, &state, &wt).await;
        match result {
            Some(WsMessage::Error(e)) => assert_eq!(e.code, "INVALID_MESSAGE"),
            _ => panic!("expected error"),
        }
    }

    #[tokio::test]
    async fn test_route_add_comment_returns_none() {
        let state = test_state();
        let wt = test_selected_worktree();
        let msg = WsMessage::AddComment(AddComment {
            file_path: "src/main.rs".to_string(),
            line_number: 10,
            end_line: None,
            content: "fix this".to_string(),
        });
        let result = route_message(&msg, &state, &wt).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_route_delete_comment_returns_none() {
        let state = test_state();
        let wt = test_selected_worktree();
        let msg = WsMessage::DeleteComment(DeleteComment {
            id: "comment-1".to_string(),
        });
        let result = route_message(&msg, &state, &wt).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_route_update_comment_returns_none() {
        let state = test_state();
        let wt = test_selected_worktree();
        let msg = WsMessage::UpdateComment(UpdateComment {
            id: "comment-1".to_string(),
            content: "updated content".to_string(),
        });
        let result = route_message(&msg, &state, &wt).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_route_pty_input_without_manager_returns_none() {
        let state = test_state();
        let wt = test_selected_worktree();
        let msg = WsMessage::PtyInput(PtyInput {
            pty_id: 1,
            data: "ls".to_string(),
        });
        let result = route_message(&msg, &state, &wt).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_route_git_status_request_without_worktree() {
        let state = test_state();
        let wt = test_selected_worktree();
        let msg = WsMessage::GitStatusRequest(GitStatusRequest {});
        let result = route_message(&msg, &state, &wt).await;
        match result {
            Some(WsMessage::Error(e)) => assert_eq!(e.code, "NO_WORKTREE_SELECTED"),
            _ => panic!("expected no worktree selected error"),
        }
    }

    #[tokio::test]
    async fn test_route_pty_spawn_request_without_worktree() {
        let state = test_state();
        let wt = test_selected_worktree();
        let msg = WsMessage::PtySpawnRequest(PtySpawnRequest {
            cols: 80,
            rows: 24,
            label: None,
        });
        let result = route_message(&msg, &state, &wt).await;
        match result {
            Some(WsMessage::Error(e)) => assert_eq!(e.code, "NO_WORKTREE_SELECTED"),
            _ => panic!("expected no worktree selected error"),
        }
    }

    #[tokio::test]
    async fn test_route_pty_spawn_request_without_pty_manager() {
        let state = test_state(); // pty_manager = None
        let wt = Arc::new(Mutex::new(Some("/tmp/test".to_string())));
        let msg = WsMessage::PtySpawnRequest(PtySpawnRequest {
            cols: 80,
            rows: 24,
            label: None,
        });
        let result = route_message(&msg, &state, &wt).await;
        match result {
            Some(WsMessage::PtySpawnResponse(r)) => {
                assert!(!r.success);
                assert!(r.error.is_some());
            }
            _ => panic!("expected PtySpawnResponse with error"),
        }
    }

    #[tokio::test]
    async fn test_route_file_content_request_without_worktree() {
        let state = test_state();
        let wt = test_selected_worktree();
        let msg = WsMessage::FileContentRequest(FileContentRequest {
            path: "test.rs".to_string(),
            diff_base: "HEAD".to_string(),
        });
        let result = route_message(&msg, &state, &wt).await;
        match result {
            Some(WsMessage::Error(e)) => assert_eq!(e.code, "NO_WORKTREE_SELECTED"),
            _ => panic!("expected no worktree selected error"),
        }
    }

    #[tokio::test]
    async fn test_route_git_stage_without_worktree() {
        let state = test_state();
        let wt = test_selected_worktree();
        let msg = WsMessage::GitStage(GitStage {
            paths: vec!["file.txt".to_string()],
        });
        let result = route_message(&msg, &state, &wt).await;
        match result {
            Some(WsMessage::Error(e)) => assert_eq!(e.code, "NO_WORKTREE_SELECTED"),
            _ => panic!("expected no worktree selected error"),
        }
    }

    #[tokio::test]
    async fn test_route_git_unstage_without_worktree() {
        let state = test_state();
        let wt = test_selected_worktree();
        let msg = WsMessage::GitUnstage(GitUnstage {
            paths: vec!["file.txt".to_string()],
        });
        let result = route_message(&msg, &state, &wt).await;
        match result {
            Some(WsMessage::Error(e)) => assert_eq!(e.code, "NO_WORKTREE_SELECTED"),
            _ => panic!("expected no worktree selected error"),
        }
    }

    #[tokio::test]
    async fn test_route_git_stage_hunk_without_worktree() {
        let state = test_state();
        let wt = test_selected_worktree();
        let msg = WsMessage::GitStageHunk(GitStageHunk {
            patch: "--- a/file.txt\n+++ b/file.txt\n".to_string(),
        });
        let result = route_message(&msg, &state, &wt).await;
        match result {
            Some(WsMessage::Error(e)) => assert_eq!(e.code, "NO_WORKTREE_SELECTED"),
            _ => panic!("expected no worktree selected error"),
        }
    }

    #[tokio::test]
    async fn test_route_git_commit_without_worktree() {
        let state = test_state();
        let wt = test_selected_worktree();
        let msg = WsMessage::GitCommitRequest(GitCommitRequest {
            message: "test".to_string(),
        });
        let result = route_message(&msg, &state, &wt).await;
        match result {
            Some(WsMessage::Error(e)) => assert_eq!(e.code, "NO_WORKTREE_SELECTED"),
            _ => panic!("expected no worktree selected error"),
        }
    }

    #[tokio::test]
    async fn test_route_git_push_without_worktree() {
        let state = test_state();
        let wt = test_selected_worktree();
        let msg = WsMessage::GitPushRequest(GitPushRequest {});
        let result = route_message(&msg, &state, &wt).await;
        match result {
            Some(WsMessage::Error(e)) => assert_eq!(e.code, "NO_WORKTREE_SELECTED"),
            _ => panic!("expected no worktree selected error"),
        }
    }

    #[tokio::test]
    async fn test_route_branch_info_without_worktree() {
        let state = test_state();
        let wt = test_selected_worktree();
        let msg = WsMessage::BranchInfoRequest(BranchInfoRequest {});
        let result = route_message(&msg, &state, &wt).await;
        match result {
            Some(WsMessage::Error(e)) => assert_eq!(e.code, "NO_WORKTREE_SELECTED"),
            _ => panic!("expected no worktree selected error"),
        }
    }

    #[tokio::test]
    async fn test_route_worktree_list_without_repo() {
        let state = test_state(); // repo_path = None
        let wt = test_selected_worktree();
        let msg = WsMessage::WorktreeListRequest(WorktreeListRequest {});
        let result = route_message(&msg, &state, &wt).await;
        match result {
            Some(WsMessage::Error(e)) => assert_eq!(e.code, "NO_REPO"),
            _ => panic!("expected no repo error"),
        }
    }

    #[tokio::test]
    async fn test_route_worktree_select_without_repo() {
        let state = test_state(); // repo_path = None
        let wt = test_selected_worktree();
        let msg = WsMessage::WorktreeSelectRequest(WorktreeSelectRequest {
            path: "/tmp/some-worktree".to_string(),
        });
        let result = route_message(&msg, &state, &wt).await;
        match result {
            Some(WsMessage::Error(e)) => assert_eq!(e.code, "NO_REPO"),
            _ => panic!("expected no repo error"),
        }
    }

    #[tokio::test]
    async fn test_route_pty_kill_without_pty_manager() {
        let state = test_state(); // pty_manager = None
        let wt = test_selected_worktree();
        let msg = WsMessage::PtyKillRequest(PtyKillRequest { pty_id: 1 });
        let result = route_message(&msg, &state, &wt).await;
        match result {
            Some(WsMessage::PtyKillResponse(r)) => {
                assert!(!r.success);
                assert_eq!(r.pty_id, 1);
                assert!(r.error.is_some());
            }
            _ => panic!("expected PtyKillResponse with error"),
        }
    }

    #[tokio::test]
    async fn test_route_pty_output_without_pty_manager() {
        let state = test_state(); // pty_manager = None
        let wt = test_selected_worktree();
        let msg = WsMessage::PtyOutputRequest(PtyOutputRequest { pty_id: 1 });
        let result = route_message(&msg, &state, &wt).await;
        match result {
            Some(WsMessage::Error(e)) => assert_eq!(e.code, "NO_PTY"),
            _ => panic!("expected NO_PTY error"),
        }
    }
}
