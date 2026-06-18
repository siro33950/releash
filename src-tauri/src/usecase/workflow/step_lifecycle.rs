#[derive(Debug, Clone)]
pub struct ResolvedWorkflowStepSession {
    pub session_id: String,
    pub worktree_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkflowStepLifecycleError {
    SessionNotFound(String),
    SessionStore(String),
    AgentSession(String),
}

impl std::fmt::Display for WorkflowStepLifecycleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SessionNotFound(session_id) => write!(f, "ChatSession not found: {session_id}"),
            Self::SessionStore(message) | Self::AgentSession(message) => write!(f, "{message}"),
        }
    }
}

pub(crate) trait WorkflowStepSessionGateway: Send + Sync {
    fn resolve_step_session(
        &self,
        session_id: &str,
    ) -> Result<Option<ResolvedWorkflowStepSession>, WorkflowStepLifecycleError>;

    fn open_step_tab(&self, session_id: &str) -> Result<(), WorkflowStepLifecycleError>;

    fn close_step_tab(&self, session_id: &str) -> Result<bool, WorkflowStepLifecycleError>;
}

#[async_trait::async_trait]
pub(crate) trait WorkflowStepRuntimeGateway: Send + Sync {
    async fn close_idle_runtime_on_tab_close(
        &self,
        session_id: &str,
    ) -> Result<(), WorkflowStepLifecycleError>;

    async fn close_runtime_on_step_done(
        &self,
        session_id: &str,
    ) -> Result<(), WorkflowStepLifecycleError>;
}

pub(crate) struct WorkflowStepLifecycle<'a> {
    pub(crate) sessions: &'a dyn WorkflowStepSessionGateway,
    pub(crate) runtime: &'a dyn WorkflowStepRuntimeGateway,
}

impl<'a> WorkflowStepLifecycle<'a> {
    fn resolve_step_session(
        &self,
        session_id: &str,
    ) -> Result<Option<ResolvedWorkflowStepSession>, WorkflowStepLifecycleError> {
        self.sessions.resolve_step_session(session_id)
    }

    pub async fn open_tab(
        &self,
        session_id: &str,
    ) -> Result<ResolvedWorkflowStepSession, WorkflowStepLifecycleError> {
        let target = self
            .resolve_step_session(session_id)?
            .ok_or_else(|| WorkflowStepLifecycleError::SessionNotFound(session_id.to_string()))?;
        self.sessions.open_step_tab(&target.session_id)?;
        Ok(target)
    }

    pub async fn try_open_tab(
        &self,
        session_id: &str,
    ) -> Result<Option<ResolvedWorkflowStepSession>, WorkflowStepLifecycleError> {
        let Some(target) = self.resolve_step_session(session_id)? else {
            return Ok(None);
        };
        self.sessions.open_step_tab(&target.session_id)?;
        Ok(Some(target))
    }

    pub async fn close_tab_target(
        &self,
        session_id: &str,
    ) -> Result<Option<ResolvedWorkflowStepSession>, WorkflowStepLifecycleError> {
        let Some(target) = self.resolve_step_session(session_id)? else {
            return Ok(None);
        };
        let runtime_result = self
            .runtime
            .close_idle_runtime_on_tab_close(&target.session_id)
            .await;
        let tab_result = self.sessions.close_step_tab(&target.session_id);
        runtime_result?;
        tab_result?;
        Ok(Some(target))
    }
}

pub(crate) async fn release_step_runtime_on_done_with_gateways(
    sessions: &dyn WorkflowStepSessionGateway,
    runtime: &dyn WorkflowStepRuntimeGateway,
    session_id: &str,
) {
    // The turn_complete handler holds session_runtime_lock across workflow
    // completion cleanup. Runtime gateways must not re-acquire it on this path.
    if let Err(_e) = runtime.close_runtime_on_step_done(session_id).await {
        log::warn!(
            "workflow_step_runtime_cleanup_failed code=runtime_close_failed message=failed_to_close_runtime"
        );
    }
    if let Err(_e) = sessions.close_step_tab(session_id) {
        log::warn!(
            "workflow_step_tab_cleanup_failed code=session_state_update_failed message=failed_to_close_step_tab"
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
        runtime_tab_close_calls: usize,
        tab_close_calls: usize,
        fail_done_runtime_close: bool,
        fail_tab_runtime_close: bool,
    }

    impl FakeLifecycleState {
        fn open_runtime_and_tab() -> Self {
            Self {
                resolved: true,
                runtime_active: true,
                tab_open: true,
                history_len: 1,
                runtime_done_close_calls: 0,
                runtime_tab_close_calls: 0,
                tab_close_calls: 0,
                fail_done_runtime_close: false,
                fail_tab_runtime_close: false,
            }
        }
    }

    struct FakeWorkflowStepSessionGateway {
        state: Arc<StdMutex<FakeLifecycleState>>,
    }

    impl WorkflowStepSessionGateway for FakeWorkflowStepSessionGateway {
        fn resolve_step_session(
            &self,
            session_id: &str,
        ) -> Result<Option<ResolvedWorkflowStepSession>, WorkflowStepLifecycleError> {
            if !self.state.lock().unwrap().resolved {
                return Ok(None);
            }
            Ok(Some(ResolvedWorkflowStepSession {
                session_id: session_id.to_string(),
                worktree_path: "/repo".to_string(),
            }))
        }

        fn open_step_tab(&self, _session_id: &str) -> Result<(), WorkflowStepLifecycleError> {
            self.state.lock().unwrap().tab_open = true;
            Ok(())
        }

        fn close_step_tab(&self, _session_id: &str) -> Result<bool, WorkflowStepLifecycleError> {
            let mut state = self.state.lock().unwrap();
            state.tab_close_calls += 1;
            let was_open = state.tab_open;
            state.tab_open = false;
            Ok(was_open)
        }
    }

    struct FakeWorkflowStepRuntimeGateway {
        state: Arc<StdMutex<FakeLifecycleState>>,
    }

    #[async_trait::async_trait]
    impl WorkflowStepRuntimeGateway for FakeWorkflowStepRuntimeGateway {
        async fn close_idle_runtime_on_tab_close(
            &self,
            _session_id: &str,
        ) -> Result<(), WorkflowStepLifecycleError> {
            let mut state = self.state.lock().unwrap();
            state.runtime_tab_close_calls += 1;
            state.runtime_active = false;
            if state.fail_tab_runtime_close {
                return Err(WorkflowStepLifecycleError::AgentSession(
                    "runtime close failed".to_string(),
                ));
            }
            Ok(())
        }

        async fn close_runtime_on_step_done(
            &self,
            _session_id: &str,
        ) -> Result<(), WorkflowStepLifecycleError> {
            let mut state = self.state.lock().unwrap();
            state.runtime_done_close_calls += 1;
            state.runtime_active = false;
            if state.fail_done_runtime_close {
                return Err(WorkflowStepLifecycleError::AgentSession(
                    "runtime close failed".to_string(),
                ));
            }
            Ok(())
        }
    }

    fn fake_lifecycle_gateways(
        state: Arc<StdMutex<FakeLifecycleState>>,
    ) -> (
        FakeWorkflowStepSessionGateway,
        FakeWorkflowStepRuntimeGateway,
    ) {
        (
            FakeWorkflowStepSessionGateway {
                state: Arc::clone(&state),
            },
            FakeWorkflowStepRuntimeGateway { state },
        )
    }

    #[tokio::test]
    async fn release_step_runtime_on_done_with_gateways_closes_runtime_and_tab() {
        let state = Arc::new(StdMutex::new(FakeLifecycleState::open_runtime_and_tab()));
        let (sessions, runtime) = fake_lifecycle_gateways(Arc::clone(&state));

        release_step_runtime_on_done_with_gateways(&sessions, &runtime, "step").await;

        let state = state.lock().unwrap();
        assert_eq!(state.runtime_done_close_calls, 1);
        assert_eq!(state.tab_close_calls, 1);
        assert!(!state.runtime_active);
        assert!(!state.tab_open);
        assert_eq!(state.history_len, 1);
    }

    #[tokio::test]
    async fn release_step_runtime_on_done_with_gateways_still_closes_tab_after_runtime_error() {
        let state = Arc::new(StdMutex::new(FakeLifecycleState {
            fail_done_runtime_close: true,
            ..FakeLifecycleState::open_runtime_and_tab()
        }));
        let (sessions, runtime) = fake_lifecycle_gateways(Arc::clone(&state));

        release_step_runtime_on_done_with_gateways(&sessions, &runtime, "step").await;

        let state = state.lock().unwrap();
        assert_eq!(state.runtime_done_close_calls, 1);
        assert_eq!(state.tab_close_calls, 1);
        assert!(!state.runtime_active);
        assert!(!state.tab_open);
        assert_eq!(state.history_len, 1);
    }

    #[tokio::test]
    async fn close_tab_target_closes_tab_and_converges_state_after_runtime_error() {
        let state = Arc::new(StdMutex::new(FakeLifecycleState {
            fail_tab_runtime_close: true,
            ..FakeLifecycleState::open_runtime_and_tab()
        }));
        let (sessions, runtime) = fake_lifecycle_gateways(Arc::clone(&state));
        let lifecycle = WorkflowStepLifecycle {
            sessions: &sessions,
            runtime: &runtime,
        };

        let result = lifecycle.close_tab_target("step").await;

        assert!(matches!(
            result,
            Err(WorkflowStepLifecycleError::AgentSession(_))
        ));
        let state = state.lock().unwrap();
        assert_eq!(state.runtime_tab_close_calls, 1);
        assert_eq!(state.tab_close_calls, 1);
        assert!(!state.runtime_active);
        assert!(!state.tab_open);
        assert_eq!(state.history_len, 1);
    }
}
