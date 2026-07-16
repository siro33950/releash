use std::sync::Arc;

use crate::domain::workflow::WorkflowError;

use super::ports::WorkspaceNodeSessionCloseGateway;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CloseWorkspaceNodeCommand {
    pub worktree_path: String,
    pub node_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkspaceNodeCloseTarget {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum CloseWorkspaceNodeError {
    #[error("Workspace node not found: {node_id}")]
    NodeNotFound { node_id: String },
    #[error("Workspace node cannot be closed: {node_id}")]
    CloseNotSupported { node_id: String },
    #[error("Failed to resolve Workspace node: {0}")]
    Resolution(WorkflowError),
    #[error("Failed to close Workspace node: {0}")]
    Close(WorkflowError),
}

pub(crate) trait WorkspaceNodeActionResolver: Send + Sync {
    fn resolve_close_target(
        &self,
        worktree_path: &str,
        node_id: &str,
    ) -> Result<WorkspaceNodeCloseTarget, CloseWorkspaceNodeError>;
}

pub(crate) struct WorkspaceNodeCommandUsecase {
    resolver: Arc<dyn WorkspaceNodeActionResolver>,
    sessions: Arc<dyn WorkspaceNodeSessionCloseGateway>,
}

impl WorkspaceNodeCommandUsecase {
    pub(crate) fn new(
        resolver: Arc<dyn WorkspaceNodeActionResolver>,
        sessions: Arc<dyn WorkspaceNodeSessionCloseGateway>,
    ) -> Self {
        Self { resolver, sessions }
    }

    pub(crate) async fn close_workspace_node(
        &self,
        command: CloseWorkspaceNodeCommand,
    ) -> Result<(), CloseWorkspaceNodeError> {
        let target = self
            .resolver
            .resolve_close_target(&command.worktree_path, &command.node_id)?;
        self.sessions
            .close_session(&target.session_id)
            .await
            .map_err(CloseWorkspaceNodeError::Close)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    struct FakeResolver {
        result: Result<WorkspaceNodeCloseTarget, CloseWorkspaceNodeError>,
        requests: Mutex<Vec<(String, String)>>,
    }

    impl WorkspaceNodeActionResolver for FakeResolver {
        fn resolve_close_target(
            &self,
            worktree_path: &str,
            node_id: &str,
        ) -> Result<WorkspaceNodeCloseTarget, CloseWorkspaceNodeError> {
            self.requests
                .lock()
                .unwrap()
                .push((worktree_path.to_string(), node_id.to_string()));
            self.result.clone()
        }
    }

    #[derive(Default)]
    struct FakeSessionCloseGateway {
        closed: Mutex<Vec<String>>,
        error: Mutex<Option<WorkflowError>>,
    }

    #[async_trait::async_trait]
    impl WorkspaceNodeSessionCloseGateway for FakeSessionCloseGateway {
        async fn close_session(&self, session_id: &str) -> Result<(), WorkflowError> {
            self.closed.lock().unwrap().push(session_id.to_string());
            match self.error.lock().unwrap().clone() {
                Some(error) => Err(error),
                None => Ok(()),
            }
        }
    }

    fn command() -> CloseWorkspaceNodeCommand {
        CloseWorkspaceNodeCommand {
            worktree_path: "/repo".to_string(),
            node_id: "opaque-node-id".to_string(),
        }
    }

    #[tokio::test]
    async fn close_workspace_node_resolves_opaque_node_before_closing_session() {
        let resolver = Arc::new(FakeResolver {
            result: Ok(WorkspaceNodeCloseTarget {
                session_id: "session-1".to_string(),
            }),
            requests: Mutex::new(Vec::new()),
        });
        let sessions = Arc::new(FakeSessionCloseGateway::default());
        let usecase = WorkspaceNodeCommandUsecase::new(resolver.clone(), sessions.clone());

        usecase.close_workspace_node(command()).await.unwrap();

        assert_eq!(
            *resolver.requests.lock().unwrap(),
            vec![("/repo".to_string(), "opaque-node-id".to_string())]
        );
        assert_eq!(sessions.closed.lock().unwrap().as_slice(), ["session-1"]);
    }

    #[tokio::test]
    async fn close_workspace_node_does_not_call_session_gateway_when_resolution_fails() {
        let resolver = Arc::new(FakeResolver {
            result: Err(CloseWorkspaceNodeError::CloseNotSupported {
                node_id: "opaque-node-id".to_string(),
            }),
            requests: Mutex::new(Vec::new()),
        });
        let sessions = Arc::new(FakeSessionCloseGateway::default());
        let usecase = WorkspaceNodeCommandUsecase::new(resolver, sessions.clone());

        let error = usecase.close_workspace_node(command()).await.unwrap_err();

        assert!(matches!(
            error,
            CloseWorkspaceNodeError::CloseNotSupported { .. }
        ));
        assert!(sessions.closed.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn close_workspace_node_preserves_session_close_failure() {
        let resolver = Arc::new(FakeResolver {
            result: Ok(WorkspaceNodeCloseTarget {
                session_id: "session-1".to_string(),
            }),
            requests: Mutex::new(Vec::new()),
        });
        let sessions = Arc::new(FakeSessionCloseGateway::default());
        *sessions.error.lock().unwrap() = Some(WorkflowError::external("runtime unavailable"));
        let usecase = WorkspaceNodeCommandUsecase::new(resolver, sessions);

        let error = usecase.close_workspace_node(command()).await.unwrap_err();

        assert_eq!(
            error,
            CloseWorkspaceNodeError::Close(WorkflowError::external("runtime unavailable"))
        );
    }
}
