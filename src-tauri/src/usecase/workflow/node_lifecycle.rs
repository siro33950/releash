use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct ResolvedWorkflowNodeSession {
    pub session_id: String,
    pub worktree_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeExecutionLifecycleError {
    SessionNotFound(String),
    SessionStore(String),
    AgentSession(String),
}

impl std::fmt::Display for NodeExecutionLifecycleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SessionNotFound(session_id) => write!(f, "ChatSession not found: {session_id}"),
            Self::SessionStore(message) | Self::AgentSession(message) => write!(f, "{message}"),
        }
    }
}

pub(crate) trait WorkflowNodeSessionGateway: Send + Sync {
    fn resolve_node_session(
        &self,
        session_id: &str,
    ) -> Result<Option<ResolvedWorkflowNodeSession>, NodeExecutionLifecycleError>;

    fn open_node_tab(&self, session_id: &str) -> Result<(), NodeExecutionLifecycleError>;

    #[cfg(test)]
    fn close_node_tab(&self, session_id: &str) -> Result<bool, NodeExecutionLifecycleError>;
}

#[async_trait::async_trait]
pub(crate) trait NodeExecutionRuntimeGateway: Send + Sync {
    async fn close_runtime_on_node_done(
        &self,
        session_id: &str,
    ) -> Result<(), NodeExecutionLifecycleError>;
}

pub(crate) struct NodeExecutionLifecycle<'a> {
    pub(crate) sessions: &'a dyn WorkflowNodeSessionGateway,
}

#[derive(Clone)]
pub(crate) struct NodeExecutionLifecycleUsecase {
    sessions: Arc<dyn WorkflowNodeSessionGateway>,
}

impl NodeExecutionLifecycleUsecase {
    pub(crate) fn new(sessions: Arc<dyn WorkflowNodeSessionGateway>) -> Self {
        Self { sessions }
    }

    fn lifecycle(&self) -> NodeExecutionLifecycle<'_> {
        NodeExecutionLifecycle {
            sessions: self.sessions.as_ref(),
        }
    }

    pub async fn try_open_tab(
        &self,
        session_id: &str,
    ) -> Result<Option<ResolvedWorkflowNodeSession>, NodeExecutionLifecycleError> {
        self.lifecycle().try_open_tab(session_id).await
    }
}

impl<'a> NodeExecutionLifecycle<'a> {
    fn resolve_node_session(
        &self,
        session_id: &str,
    ) -> Result<Option<ResolvedWorkflowNodeSession>, NodeExecutionLifecycleError> {
        self.sessions.resolve_node_session(session_id)
    }

    pub async fn try_open_tab(
        &self,
        session_id: &str,
    ) -> Result<Option<ResolvedWorkflowNodeSession>, NodeExecutionLifecycleError> {
        let Some(target) = self.resolve_node_session(session_id)? else {
            return Ok(None);
        };
        self.sessions.open_node_tab(&target.session_id)?;
        Ok(Some(target))
    }

    #[cfg(test)]
    pub async fn close_tab_target(
        &self,
        session_id: &str,
    ) -> Result<Option<ResolvedWorkflowNodeSession>, NodeExecutionLifecycleError> {
        let Some(target) = self.resolve_node_session(session_id)? else {
            return Ok(None);
        };
        self.sessions.close_node_tab(&target.session_id)?;
        Ok(Some(target))
    }
}

pub(crate) async fn release_node_runtime_on_done_with_gateways(
    runtime: &dyn NodeExecutionRuntimeGateway,
    session_id: &str,
) {
    // The turn_complete handler holds session_runtime_lock across workflow
    // completion cleanup. Runtime gateways must not re-acquire it on this path.
    if let Err(_e) = runtime.close_runtime_on_node_done(session_id).await {
        log::warn!(
            "workflow_node_runtime_cleanup_failed code=runtime_close_failed message=failed_to_close_runtime"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::Mutex as StdMutex;

    #[derive(Debug)]
    struct FakeLifecycleState {
        resolved: bool,
        runtime_active: bool,
        tab_open: bool,
        history_len: usize,
        runtime_done_close_calls: usize,
        tab_close_calls: usize,
        fail_done_runtime_close: bool,
    }

    impl FakeLifecycleState {
        fn open_runtime_and_tab() -> Self {
            Self {
                resolved: true,
                runtime_active: true,
                tab_open: true,
                history_len: 1,
                runtime_done_close_calls: 0,
                tab_close_calls: 0,
                fail_done_runtime_close: false,
            }
        }
    }

    struct FakeWorkflowNodeSessionGateway {
        state: Arc<StdMutex<FakeLifecycleState>>,
    }

    impl WorkflowNodeSessionGateway for FakeWorkflowNodeSessionGateway {
        fn resolve_node_session(
            &self,
            session_id: &str,
        ) -> Result<Option<ResolvedWorkflowNodeSession>, NodeExecutionLifecycleError> {
            if !self.state.lock().unwrap().resolved {
                return Ok(None);
            }
            Ok(Some(ResolvedWorkflowNodeSession {
                session_id: session_id.to_string(),
                worktree_path: "/repo".to_string(),
            }))
        }

        fn open_node_tab(&self, _session_id: &str) -> Result<(), NodeExecutionLifecycleError> {
            self.state.lock().unwrap().tab_open = true;
            Ok(())
        }

        fn close_node_tab(&self, _session_id: &str) -> Result<bool, NodeExecutionLifecycleError> {
            let mut state = self.state.lock().unwrap();
            state.tab_close_calls += 1;
            let was_open = state.tab_open;
            state.tab_open = false;
            Ok(was_open)
        }
    }

    struct FakeNodeExecutionRuntimeGateway {
        state: Arc<StdMutex<FakeLifecycleState>>,
    }

    #[async_trait::async_trait]
    impl NodeExecutionRuntimeGateway for FakeNodeExecutionRuntimeGateway {
        async fn close_runtime_on_node_done(
            &self,
            _session_id: &str,
        ) -> Result<(), NodeExecutionLifecycleError> {
            let mut state = self.state.lock().unwrap();
            state.runtime_done_close_calls += 1;
            state.runtime_active = false;
            if state.fail_done_runtime_close {
                return Err(NodeExecutionLifecycleError::AgentSession(
                    "runtime close failed".to_string(),
                ));
            }
            Ok(())
        }
    }

    fn fake_lifecycle_gateways(
        state: Arc<StdMutex<FakeLifecycleState>>,
    ) -> (
        FakeWorkflowNodeSessionGateway,
        FakeNodeExecutionRuntimeGateway,
    ) {
        (
            FakeWorkflowNodeSessionGateway {
                state: Arc::clone(&state),
            },
            FakeNodeExecutionRuntimeGateway { state },
        )
    }

    #[tokio::test]
    async fn release_node_runtime_on_done_with_gateways_closes_runtime_only() {
        let state = Arc::new(StdMutex::new(FakeLifecycleState::open_runtime_and_tab()));
        let (_, runtime) = fake_lifecycle_gateways(Arc::clone(&state));

        release_node_runtime_on_done_with_gateways(&runtime, "node").await;

        let state = state.lock().unwrap();
        assert_eq!(state.runtime_done_close_calls, 1);
        assert_eq!(state.tab_close_calls, 0);
        assert!(!state.runtime_active);
        assert!(state.tab_open);
        assert_eq!(state.history_len, 1);
    }

    #[tokio::test]
    async fn release_node_runtime_on_done_with_gateways_keeps_tab_after_runtime_error() {
        let state = Arc::new(StdMutex::new(FakeLifecycleState {
            fail_done_runtime_close: true,
            ..FakeLifecycleState::open_runtime_and_tab()
        }));
        let (_, runtime) = fake_lifecycle_gateways(Arc::clone(&state));

        release_node_runtime_on_done_with_gateways(&runtime, "node").await;

        let state = state.lock().unwrap();
        assert_eq!(state.runtime_done_close_calls, 1);
        assert_eq!(state.tab_close_calls, 0);
        assert!(!state.runtime_active);
        assert!(state.tab_open);
        assert_eq!(state.history_len, 1);
    }

    #[tokio::test]
    async fn close_quit_workflow_node_tab_close_is_view_only() {
        let state = Arc::new(StdMutex::new(FakeLifecycleState::open_runtime_and_tab()));
        let (sessions, _) = fake_lifecycle_gateways(Arc::clone(&state));
        let lifecycle = NodeExecutionLifecycle {
            sessions: &sessions,
        };

        let result = lifecycle.close_tab_target("node").await;

        assert!(result.is_ok());
        let state = state.lock().unwrap();
        assert_eq!(state.tab_close_calls, 1);
        assert!(state.runtime_active);
        assert!(!state.tab_open);
        assert_eq!(state.history_len, 1);
    }
}
