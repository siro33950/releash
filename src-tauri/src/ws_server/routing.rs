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
        WsMessage::PtySpawnRequest(req) => {
            handle_pty_spawn_request(req, state, selected_worktree).await
        }
        WsMessage::PtyOutputRequest(req) => handle_pty_output_request(req, state),
        WsMessage::PtyKillRequest(req) => handle_pty_kill_request(req, state).await,
        WsMessage::BranchInfoRequest(_) => handle_branch_info_request(selected_worktree).await,
        WsMessage::WorktreeListRequest(_) => handle_worktree_list_request(state).await,
        WsMessage::WorktreeSelectRequest(req) => {
            handle_worktree_select_request(req, state, selected_worktree).await
        }
        WsMessage::BackendListRequest(_) => handle_backend_list_request(state),
        WsMessage::AgentSessionStartRequest(req) => {
            handle_agent_session_start_request(req, state).await
        }
        WsMessage::AgentMessageRequest(req) => handle_agent_message_request(req, state).await,
        WsMessage::AgentInterruptRequest(req) => handle_agent_interrupt_request(req, state).await,
        WsMessage::AgentModelSetRequest(req) => handle_agent_model_set_request(req, state).await,
        WsMessage::AgentPermissionModeSetRequest(req) => {
            handle_agent_permission_mode_set_request(req, state).await
        }
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
            Arc::new(parking_lot::RwLock::new(vec![])),
            app_config,
            None,
            false,
            Arc::new(crate::git_host::PrCache::new()),
            Arc::new(crate::backends::AgentBackendRegistry::new()),
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

    #[tokio::test]
    async fn test_route_backend_list_empty_registry() {
        let state = test_state(); // empty registry
        let wt = test_selected_worktree();
        let msg = WsMessage::BackendListRequest(BackendListRequest {});
        let result = route_message(&msg, &state, &wt).await;
        match result {
            Some(WsMessage::BackendListResponse(r)) => {
                assert!(r.backends.is_empty());
                assert!(r.default_id.is_none());
            }
            _ => panic!("expected BackendListResponse with empty list"),
        }
    }

    fn test_state_with_registry() -> WsServerState {
        let mut config = crate::config::ReleashConfig::default();
        config.agents.claude.models = vec!["opus-4".to_string(), "haiku".to_string()];
        let app_config = Arc::new(crate::config::AppConfig::new(
            config,
            std::path::PathBuf::from("/tmp/test-releash.toml"),
        ));
        let mut registry = crate::backends::AgentBackendRegistry::new();
        registry.register(Arc::new(crate::backends::claude::ClaudeBackend::new()));
        registry.set_config(app_config.clone());
        WsServerState::new(
            None,
            Arc::new(WsBroadcaster::default()),
            None,
            Arc::new(parking_lot::RwLock::new(vec![])),
            app_config,
            None,
            false,
            Arc::new(crate::git_host::PrCache::new()),
            Arc::new(registry),
        )
    }

    #[tokio::test]
    async fn test_route_backend_list_with_registry() {
        let state = test_state_with_registry();
        let wt = test_selected_worktree();
        let msg = WsMessage::BackendListRequest(BackendListRequest {});
        let result = route_message(&msg, &state, &wt).await;
        match result {
            Some(WsMessage::BackendListResponse(r)) => {
                assert_eq!(r.backends.len(), 1);
                assert_eq!(r.backends[0].id, "claude");
                assert_eq!(r.backends[0].name, "Claude");
                assert!(r.backends[0].available);
                let values: Vec<&str> = r.backends[0]
                    .available_models
                    .iter()
                    .map(|m| m.value.as_str())
                    .collect();
                assert_eq!(values, vec!["opus-4", "haiku"]);
            }
            _ => panic!("expected BackendListResponse"),
        }
    }

    #[tokio::test]
    async fn test_route_agent_session_start_invalid_worktree() {
        let state = test_state_with_registry();
        let wt = test_selected_worktree();
        let msg = WsMessage::AgentSessionStartRequest(AgentSessionStartRequest {
            worktree_path: "/nonexistent/repo".to_string(),
            backend_id: Some("claude".to_string()),
            permission_mode: Some("edit".to_string()),
        });
        let result = route_message(&msg, &state, &wt).await;
        match result {
            Some(WsMessage::AgentSessionStartResponse(r)) => {
                assert!(!r.success);
                assert!(r.error.is_some());
                assert!(r.error.unwrap().contains("worktree"));
            }
            _ => panic!("expected AgentSessionStartResponse with error"),
        }
    }
}
