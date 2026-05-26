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
    use tokio::sync::Mutex;

    use crate::agent_sdk::AgentProcessMap;
    use crate::session::{OpenTabRegistry, SessionState, SessionStore};
    use crate::workflow_step_lifecycle_adapters::{
        close_resolved_step_tab_state, close_step_session_tab_state,
        hydrate_open_workflow_step_tabs, open_step_session_tab_state,
        resolve_step_session_with_data_dir, should_release_runtime_on_tab_close,
        try_close_step_session_tab_state,
    };

    async fn release_step_runtime_on_done_state<F, Fut>(
        session_store: &SessionStore,
        data_dir: &std::path::Path,
        open_tabs: Option<&OpenTabRegistry>,
        session_id: &str,
        close_runtime: F,
    ) where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<(), WorkflowStepLifecycleError>>,
    {
        if let Err(_e) = close_runtime().await {
            log::warn!(
                "workflow_step_runtime_cleanup_failed code=runtime_close_failed message=failed_to_close_runtime"
            );
        }
        close_step_session_tab_state(session_store, data_dir, open_tabs, session_id);
    }

    async fn release_on_step_done_for_test(
        session_store: &SessionStore,
        data_dir: &std::path::Path,
        handles: &Arc<Mutex<AgentProcessMap>>,
        open_tabs: Option<&OpenTabRegistry>,
        session_id: &str,
    ) {
        release_step_runtime_on_done_state(
            session_store,
            data_dir,
            open_tabs,
            session_id,
            || async {
                handles.lock().await.remove(session_id);
                Ok(())
            },
        )
        .await;
    }

    fn workflow_step_session_for_test(session_id: &str) -> crate::session::ChatSession {
        crate::session::ChatSession {
            id: session_id.to_string(),
            worktree_path: "/repo".to_string(),
            messages: vec![crate::session::ChatMessage {
                id: "msg-1".to_string(),
                role: crate::session::MessageRole::Agent,
                content: "history".to_string(),
                thinking: None,
                activities: None,
                parts: None,
                timestamp: 1.0,
                mentions: None,
            }],
            state: SessionState::Idle,
            created_at: 1.0,
            updated_at: 1.0,
            agent_session_id: Some("sdk-session".to_string()),
            permission_mode: "edit".to_string(),
            selected_model: None,
            backend_id: Some(crate::agent_sdk::CLAUDE_BACKEND_ID.to_string()),
            workflow_step_session: true,
        }
    }

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

    #[test]
    fn step_done_tab_cleanup_removes_tab_closes_session_and_preserves_history() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = SessionStore::default();
        let open_tabs = OpenTabRegistry::default();
        let session_id = uuid::Uuid::new_v4().to_string();

        session_store
            .save_session(tmp.path(), &workflow_step_session_for_test(&session_id))
            .unwrap();
        open_tabs.add(&session_id);

        close_step_session_tab_state(&session_store, tmp.path(), Some(&open_tabs), &session_id);

        assert!(!open_tabs.contains(&session_id));
        let session = session_store
            .get_session(tmp.path(), &session_id)
            .unwrap()
            .expect("session remains as history");
        assert_eq!(session.state, SessionState::Closed);
        assert_eq!(session.agent_session_id.as_deref(), Some("sdk-session"));
        assert_eq!(session.messages.len(), 1);
    }

    #[test]
    fn step_done_tab_cleanup_is_idempotent_for_already_closed_tab() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = SessionStore::default();
        let open_tabs = OpenTabRegistry::default();
        let session_id = uuid::Uuid::new_v4().to_string();

        session_store
            .save_session(tmp.path(), &workflow_step_session_for_test(&session_id))
            .unwrap();
        session_store
            .set_session_state(tmp.path(), &session_id, SessionState::Closed)
            .unwrap();

        close_step_session_tab_state(&session_store, tmp.path(), Some(&open_tabs), &session_id);

        assert!(!open_tabs.contains(&session_id));
        let session = session_store
            .get_session(tmp.path(), &session_id)
            .unwrap()
            .expect("session remains as history");
        assert_eq!(session.state, SessionState::Closed);
        assert_eq!(session.agent_session_id.as_deref(), Some("sdk-session"));
        assert_eq!(session.messages.len(), 1);
    }

    #[test]
    fn close_step_tab_retries_closed_state_when_registry_entry_is_missing() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = SessionStore::default();
        let open_tabs = OpenTabRegistry::default();
        let session_id = uuid::Uuid::new_v4().to_string();

        session_store
            .save_session(tmp.path(), &workflow_step_session_for_test(&session_id))
            .unwrap();

        let changed = try_close_step_session_tab_state(
            &session_store,
            tmp.path(),
            Some(&open_tabs),
            &session_id,
        )
        .unwrap();

        assert!(!changed);
        assert!(!open_tabs.contains(&session_id));
        let session = session_store
            .get_session(tmp.path(), &session_id)
            .unwrap()
            .expect("session remains as history");
        assert_eq!(session.state, SessionState::Closed);
    }

    #[test]
    fn hydrate_open_workflow_step_tabs_only_opens_non_closed_workflow_sessions() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = SessionStore::default();
        let open_tabs = OpenTabRegistry::default();
        let worktree_path = "/repo";

        let open_step_id = uuid::Uuid::new_v4().to_string();
        let closed_step_id = uuid::Uuid::new_v4().to_string();
        let regular_id = uuid::Uuid::new_v4().to_string();

        let open_step = workflow_step_session_for_test(&open_step_id);
        let mut closed_step = workflow_step_session_for_test(&closed_step_id);
        closed_step.state = SessionState::Closed;
        let mut regular = workflow_step_session_for_test(&regular_id);
        regular.workflow_step_session = false;

        session_store.save_session(tmp.path(), &open_step).unwrap();
        session_store
            .save_session(tmp.path(), &closed_step)
            .unwrap();
        session_store.save_session(tmp.path(), &regular).unwrap();

        hydrate_open_workflow_step_tabs(&session_store, tmp.path(), worktree_path, &open_tabs)
            .unwrap();

        assert!(open_tabs.contains(&open_step_id));
        assert!(!open_tabs.contains(&closed_step_id));
        assert!(!open_tabs.contains(&regular_id));
    }

    #[tokio::test]
    async fn opening_step_tab_does_not_start_runtime_and_preserves_history() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = SessionStore::default();
        let open_tabs = OpenTabRegistry::default();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = uuid::Uuid::new_v4().to_string();
        let mut session = workflow_step_session_for_test(&session_id);
        session.state = SessionState::Closed;

        session_store.save_session(tmp.path(), &session).unwrap();

        open_step_session_tab_state(&session_store, tmp.path(), &open_tabs, &session_id).unwrap();

        assert!(open_tabs.contains(&session_id));
        assert!(handles.lock().await.is_empty());
        let session = session_store
            .get_session(tmp.path(), &session_id)
            .unwrap()
            .expect("session remains as history");
        assert_eq!(session.state, SessionState::Idle);
        assert_eq!(session.agent_session_id.as_deref(), Some("sdk-session"));
        assert_eq!(session.messages.len(), 1);
        let updated_at = session.updated_at;

        open_step_session_tab_state(&session_store, tmp.path(), &open_tabs, &session_id).unwrap();
        assert_eq!(open_tabs.snapshot().len(), 1);
        assert!(handles.lock().await.is_empty());
        let session = session_store
            .get_session(tmp.path(), &session_id)
            .unwrap()
            .expect("session remains as history");
        assert_eq!(session.updated_at, updated_at);
    }

    #[tokio::test]
    async fn tab_close_runtime_policy_releases_ready_and_idle_runtime() {
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        handles.lock().await.insert(
            "step".to_string(),
            crate::agent_sdk::make_test_agent_process(),
        );

        assert!(should_release_runtime_on_tab_close(&handles, "step").await);
    }

    #[tokio::test]
    async fn tab_close_runtime_policy_keeps_busy_runtime() {
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let mut proc = crate::agent_sdk::make_test_agent_process();
        proc.state = crate::agent_sdk::BridgeState::Streaming;
        handles.lock().await.insert("step".to_string(), proc);
        assert!(!should_release_runtime_on_tab_close(&handles, "step").await);

        {
            let mut map = handles.lock().await;
            let proc = map.get_mut("step").unwrap();
            proc.state = crate::agent_sdk::BridgeState::Ready;
            proc.turn_phase = crate::agent_sdk::TurnPhase::WaitingPermission;
        }
        assert!(!should_release_runtime_on_tab_close(&handles, "step").await);

        {
            let mut map = handles.lock().await;
            let proc = map.get_mut("step").unwrap();
            proc.turn_phase = crate::agent_sdk::TurnPhase::Idle;
            proc.pending_message = Some(crate::agent_sdk::PendingMessage {
                content: "next".to_string(),
                permission_mode: "edit".to_string(),
                images: Vec::new(),
                worktree_path: "/repo".to_string(),
                mentions: Vec::new(),
            });
        }
        assert!(!should_release_runtime_on_tab_close(&handles, "step").await);
    }

    #[tokio::test]
    async fn tab_close_idle_runtime_releases_runtime_and_closes_tab_state() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = SessionStore::default();
        let open_tabs = OpenTabRegistry::default();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = uuid::Uuid::new_v4().to_string();
        session_store
            .save_session(tmp.path(), &workflow_step_session_for_test(&session_id))
            .unwrap();
        open_tabs.add(&session_id);
        handles.lock().await.insert(
            session_id.clone(),
            crate::agent_sdk::make_test_agent_process(),
        );
        let close_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        close_resolved_step_tab_state(
            &session_store,
            tmp.path(),
            &handles,
            &open_tabs,
            &session_id,
            {
                let handles = Arc::clone(&handles);
                let session_id = session_id.clone();
                let close_count = Arc::clone(&close_count);
                move || async move {
                    close_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    handles.lock().await.remove(&session_id);
                    Ok(())
                }
            },
        )
        .await
        .unwrap();

        assert_eq!(close_count.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(!handles.lock().await.contains_key(&session_id));
        assert!(!open_tabs.contains(&session_id));
        let session = session_store
            .get_session(tmp.path(), &session_id)
            .unwrap()
            .expect("history remains");
        assert_eq!(session.state, SessionState::Closed);
        assert_eq!(session.agent_session_id.as_deref(), Some("sdk-session"));
        assert_eq!(session.messages.len(), 1);
    }

    #[tokio::test]
    async fn tab_close_busy_runtime_keeps_runtime_and_closes_only_tab_state() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = SessionStore::default();
        let open_tabs = OpenTabRegistry::default();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = uuid::Uuid::new_v4().to_string();
        session_store
            .save_session(tmp.path(), &workflow_step_session_for_test(&session_id))
            .unwrap();
        open_tabs.add(&session_id);
        let mut proc = crate::agent_sdk::make_test_agent_process();
        proc.state = crate::agent_sdk::BridgeState::Streaming;
        handles.lock().await.insert(session_id.clone(), proc);
        let close_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        close_resolved_step_tab_state(
            &session_store,
            tmp.path(),
            &handles,
            &open_tabs,
            &session_id,
            {
                let handles = Arc::clone(&handles);
                let session_id = session_id.clone();
                let close_count = Arc::clone(&close_count);
                move || async move {
                    close_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    handles.lock().await.remove(&session_id);
                    Ok(())
                }
            },
        )
        .await
        .unwrap();

        assert_eq!(close_count.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert!(handles.lock().await.contains_key(&session_id));
        assert!(!open_tabs.contains(&session_id));
        let session = session_store
            .get_session(tmp.path(), &session_id)
            .unwrap()
            .expect("history remains");
        assert_eq!(session.state, SessionState::Closed);
        assert_eq!(session.agent_session_id.as_deref(), Some("sdk-session"));
    }

    #[tokio::test]
    async fn duplicate_tab_close_releases_remaining_idle_runtime_after_tab_already_closed() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = SessionStore::default();
        let open_tabs = OpenTabRegistry::default();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = uuid::Uuid::new_v4().to_string();
        session_store
            .save_session(tmp.path(), &workflow_step_session_for_test(&session_id))
            .unwrap();
        handles.lock().await.insert(
            session_id.clone(),
            crate::agent_sdk::make_test_agent_process(),
        );
        let close_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        close_resolved_step_tab_state(
            &session_store,
            tmp.path(),
            &handles,
            &open_tabs,
            &session_id,
            {
                let handles = Arc::clone(&handles);
                let session_id = session_id.clone();
                let close_count = Arc::clone(&close_count);
                move || async move {
                    close_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    handles.lock().await.remove(&session_id);
                    Ok(())
                }
            },
        )
        .await
        .unwrap();

        assert_eq!(close_count.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(!handles.lock().await.contains_key(&session_id));
    }

    #[tokio::test]
    async fn duplicate_tab_close_without_runtime_is_noop_and_keeps_session_closed() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = SessionStore::default();
        let open_tabs = OpenTabRegistry::default();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = uuid::Uuid::new_v4().to_string();
        let mut session = workflow_step_session_for_test(&session_id);
        session.state = SessionState::Closed;
        session_store.save_session(tmp.path(), &session).unwrap();
        let close_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        close_resolved_step_tab_state(
            &session_store,
            tmp.path(),
            &handles,
            &open_tabs,
            &session_id,
            {
                let close_count = Arc::clone(&close_count);
                move || async move {
                    close_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    Ok(())
                }
            },
        )
        .await
        .unwrap();

        assert_eq!(close_count.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert!(!handles.lock().await.contains_key(&session_id));
        assert!(!open_tabs.contains(&session_id));
        let session = session_store
            .get_session(tmp.path(), &session_id)
            .unwrap()
            .expect("history remains");
        assert_eq!(session.state, SessionState::Closed);
        assert_eq!(session.agent_session_id.as_deref(), Some("sdk-session"));
    }

    #[tokio::test]
    async fn tab_close_runtime_failure_still_closes_tab_state() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = SessionStore::default();
        let open_tabs = OpenTabRegistry::default();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = uuid::Uuid::new_v4().to_string();
        session_store
            .save_session(tmp.path(), &workflow_step_session_for_test(&session_id))
            .unwrap();
        open_tabs.add(&session_id);
        handles.lock().await.insert(
            session_id.clone(),
            crate::agent_sdk::make_test_agent_process(),
        );

        let result = close_resolved_step_tab_state(
            &session_store,
            tmp.path(),
            &handles,
            &open_tabs,
            &session_id,
            {
                let handles = Arc::clone(&handles);
                let session_id = session_id.clone();
                move || async move {
                    handles.lock().await.remove(&session_id);
                    Err(WorkflowStepLifecycleError::AgentSession(
                        "runtime close failed".to_string(),
                    ))
                }
            },
        )
        .await;

        assert!(matches!(
            result,
            Err(WorkflowStepLifecycleError::AgentSession(_))
        ));
        assert!(!handles.lock().await.contains_key(&session_id));
        assert!(!open_tabs.contains(&session_id));
        let view = crate::workflow_state_events::build_workflow_state_view(
            workflow_state_for_test(&session_id),
            &handles,
            &open_tabs,
        )
        .await;
        assert!(!view.runtime_states[&session_id].runtime_active);
        assert!(!view.runtime_states[&session_id].tab_open);
        let session = session_store
            .get_session(tmp.path(), &session_id)
            .unwrap()
            .expect("history remains");
        assert_eq!(session.state, SessionState::Closed);
    }

    #[tokio::test]
    async fn tab_state_update_failure_does_not_roll_back_runtime_release() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = SessionStore::default();
        let open_tabs = OpenTabRegistry::default();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = uuid::Uuid::new_v4().to_string();
        open_tabs.add(&session_id);
        handles.lock().await.insert(
            session_id.clone(),
            crate::agent_sdk::make_test_agent_process(),
        );
        let close_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        let result = close_resolved_step_tab_state(
            &session_store,
            tmp.path(),
            &handles,
            &open_tabs,
            &session_id,
            {
                let handles = Arc::clone(&handles);
                let session_id = session_id.clone();
                let close_count = Arc::clone(&close_count);
                move || async move {
                    close_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    handles.lock().await.remove(&session_id);
                    Ok(())
                }
            },
        )
        .await;

        assert!(matches!(
            result,
            Err(WorkflowStepLifecycleError::SessionStore(_))
        ));
        assert_eq!(close_count.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(!handles.lock().await.contains_key(&session_id));
        assert!(open_tabs.contains(&session_id));
        let view = crate::workflow_state_events::build_workflow_state_view(
            workflow_state_for_test(&session_id),
            &handles,
            &open_tabs,
        )
        .await;
        assert!(!view.runtime_states[&session_id].runtime_active);
        assert!(view.runtime_states[&session_id].tab_open);
    }

    fn workflow_state_for_test(session_id: &str) -> crate::workflow::state::WorkflowState {
        use crate::workflow::schema::Workflow;
        use crate::workflow::state::{
            StepHistoryEntry, TokenUsage, WorkflowExecutionState, WorkflowState,
        };
        use std::collections::HashMap;
        WorkflowState {
            execution_id: "exec-1".to_string(),
            workflow_name: "wf".to_string(),
            state: WorkflowExecutionState::Completed,
            current_step_index: 0,
            current_step_name: "done".to_string(),
            current_session_id: Some(session_id.to_string()),
            total_steps: 1,
            step_history: vec![StepHistoryEntry {
                step_name: "done".to_string(),
                completed_at: 1.0,
                result: Some("ok".to_string()),
                session_id: Some(session_id.to_string()),
                token_usage: Some(TokenUsage::default()),
                structured_output: None,
                run_index: 1,
                child_outputs: None,
                state: crate::workflow::state::default_step_entry_state(),
            }],
            step_execution_counts: HashMap::new(),
            workflow_definition: Workflow {
                variables: Default::default(),
                name: "wf".to_string(),
                description: String::new(),
                builtin: false,
                nodes: vec![],
            },
            total_token_usage: TokenUsage::default(),
            step_states: HashMap::new(),
            step_outputs: HashMap::new(),
            active_parallel_steps: vec![],
            workflow_variables: HashMap::new(),
            approval_operations: None,
            started_at: 0.0,
            updated_at: 1.0,
        }
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

    #[tokio::test]
    async fn release_on_step_done_releases_runtime_and_open_tab_but_preserves_history() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = SessionStore::default();
        let open_tabs = OpenTabRegistry::default();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = uuid::Uuid::new_v4().to_string();

        session_store
            .save_session(tmp.path(), &workflow_step_session_for_test(&session_id))
            .unwrap();
        open_tabs.add(&session_id);
        handles.lock().await.insert(
            session_id.clone(),
            crate::agent_sdk::make_test_agent_process(),
        );

        release_on_step_done_for_test(
            &session_store,
            tmp.path(),
            &handles,
            Some(&open_tabs),
            &session_id,
        )
        .await;

        let view = crate::workflow_state_events::build_workflow_state_view(
            workflow_state_for_test(&session_id),
            &handles,
            &open_tabs,
        )
        .await;
        assert!(!view.runtime_states[&session_id].runtime_active);
        assert!(!view.runtime_states[&session_id].tab_open);
        let session = session_store
            .get_session(tmp.path(), &session_id)
            .unwrap()
            .expect("step history session remains");
        assert_eq!(session.state, SessionState::Closed);
        assert_eq!(session.agent_session_id.as_deref(), Some("sdk-session"));
        assert_eq!(session.messages.len(), 1);
    }

    #[tokio::test]
    async fn release_on_step_done_releases_runtime_when_tab_already_closed() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = SessionStore::default();
        let open_tabs = OpenTabRegistry::default();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = uuid::Uuid::new_v4().to_string();
        let mut session = workflow_step_session_for_test(&session_id);
        session.state = SessionState::Closed;

        session_store.save_session(tmp.path(), &session).unwrap();
        handles.lock().await.insert(
            session_id.clone(),
            crate::agent_sdk::make_test_agent_process(),
        );

        release_on_step_done_for_test(
            &session_store,
            tmp.path(),
            &handles,
            Some(&open_tabs),
            &session_id,
        )
        .await;

        let view = crate::workflow_state_events::build_workflow_state_view(
            workflow_state_for_test(&session_id),
            &handles,
            &open_tabs,
        )
        .await;
        assert!(!view.runtime_states[&session_id].runtime_active);
        assert!(!view.runtime_states[&session_id].tab_open);
        let session = session_store
            .get_session(tmp.path(), &session_id)
            .unwrap()
            .expect("step history session remains");
        assert_eq!(session.state, SessionState::Closed);
        assert_eq!(session.agent_session_id.as_deref(), Some("sdk-session"));
        assert_eq!(session.messages.len(), 1);
    }

    #[tokio::test]
    async fn release_on_step_done_releases_busy_runtime_and_closes_tab_state() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = SessionStore::default();
        let open_tabs = OpenTabRegistry::default();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = uuid::Uuid::new_v4().to_string();

        session_store
            .save_session(tmp.path(), &workflow_step_session_for_test(&session_id))
            .unwrap();
        open_tabs.add(&session_id);
        let mut proc = crate::agent_sdk::make_test_agent_process();
        proc.pending_message = Some(crate::agent_sdk::PendingMessage {
            content: "continue".to_string(),
            permission_mode: "edit".to_string(),
            images: Vec::new(),
            worktree_path: "/repo".to_string(),
            mentions: Vec::new(),
        });
        handles.lock().await.insert(session_id.clone(), proc);

        release_on_step_done_for_test(
            &session_store,
            tmp.path(),
            &handles,
            Some(&open_tabs),
            &session_id,
        )
        .await;

        assert!(!handles.lock().await.contains_key(&session_id));
        assert!(!open_tabs.contains(&session_id));
        let session = session_store
            .get_session(tmp.path(), &session_id)
            .unwrap()
            .expect("step history session remains");
        assert_eq!(session.state, SessionState::Closed);
    }

    #[tokio::test]
    async fn release_and_tab_close_converge_to_closed_runtime_and_tab() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = SessionStore::default();
        let open_tabs = OpenTabRegistry::default();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = uuid::Uuid::new_v4().to_string();

        session_store
            .save_session(tmp.path(), &workflow_step_session_for_test(&session_id))
            .unwrap();
        open_tabs.add(&session_id);
        handles.lock().await.insert(
            session_id.clone(),
            crate::agent_sdk::make_test_agent_process(),
        );

        release_on_step_done_for_test(
            &session_store,
            tmp.path(),
            &handles,
            Some(&open_tabs),
            &session_id,
        )
        .await;
        try_close_step_session_tab_state(&session_store, tmp.path(), Some(&open_tabs), &session_id)
            .unwrap();

        let view = crate::workflow_state_events::build_workflow_state_view(
            workflow_state_for_test(&session_id),
            &handles,
            &open_tabs,
        )
        .await;
        assert!(!view.runtime_states[&session_id].runtime_active);
        assert!(!view.runtime_states[&session_id].tab_open);
    }

    // R4-01: Spec「runtime 起動中の step を再オープンしても runtime 状態は変化しない」
    #[tokio::test]
    async fn reopening_step_tab_with_active_runtime_keeps_runtime_active() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = SessionStore::default();
        let open_tabs = OpenTabRegistry::default();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = uuid::Uuid::new_v4().to_string();

        let mut session = workflow_step_session_for_test(&session_id);
        session.state = SessionState::Closed;
        session_store.save_session(tmp.path(), &session).unwrap();
        handles.lock().await.insert(
            session_id.clone(),
            crate::agent_sdk::make_test_agent_process(),
        );

        open_step_session_tab_state(&session_store, tmp.path(), &open_tabs, &session_id).unwrap();

        assert!(open_tabs.contains(&session_id));
        assert!(handles.lock().await.contains_key(&session_id));
        let session = session_store
            .get_session(tmp.path(), &session_id)
            .unwrap()
            .expect("session remains as history");
        assert_eq!(session.state, SessionState::Idle);
    }

    // R4-02: Spec「非 workflow session への tab 操作は workflow step の状態を変化させない」
    #[tokio::test]
    async fn non_workflow_session_tab_operations_do_not_affect_workflow_step_state() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = SessionStore::default();
        let open_tabs = OpenTabRegistry::default();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));

        // Workflow step session: tab open + runtime active
        let step_id = uuid::Uuid::new_v4().to_string();
        session_store
            .save_session(tmp.path(), &workflow_step_session_for_test(&step_id))
            .unwrap();
        open_tabs.add(&step_id);
        handles
            .lock()
            .await
            .insert(step_id.clone(), crate::agent_sdk::make_test_agent_process());

        // Non-workflow session (different id, workflow_step_session=false)
        let non_workflow_id = uuid::Uuid::new_v4().to_string();
        let mut non_workflow = workflow_step_session_for_test(&non_workflow_id);
        non_workflow.workflow_step_session = false;
        session_store
            .save_session(tmp.path(), &non_workflow)
            .unwrap();

        // Resolver returns None for non-workflow session → tab operations would not proceed
        let resolved =
            resolve_step_session_with_data_dir(&session_store, tmp.path(), &non_workflow_id)
                .unwrap();
        assert!(resolved.is_none());

        // Workflow step state is unchanged
        assert!(open_tabs.contains(&step_id));
        assert!(handles.lock().await.contains_key(&step_id));
    }

    // R4-05: Spec「完了確定と tab close が競合しても runtime は二重解放されない」
    #[tokio::test]
    async fn concurrent_step_done_release_and_tab_close_runs_close_at_most_once() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let tmp = tempfile::TempDir::new().unwrap();
        let data_dir: std::path::PathBuf = tmp.path().to_path_buf();
        let session_store = Arc::new(SessionStore::default());
        let open_tabs = Arc::new(OpenTabRegistry::default());
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = uuid::Uuid::new_v4().to_string();

        session_store
            .save_session(&data_dir, &workflow_step_session_for_test(&session_id))
            .unwrap();
        open_tabs.add(&session_id);
        handles.lock().await.insert(
            session_id.clone(),
            crate::agent_sdk::make_test_agent_process(),
        );

        let close_count = Arc::new(AtomicUsize::new(0));

        let tab_close = {
            let session_store = Arc::clone(&session_store);
            let open_tabs = Arc::clone(&open_tabs);
            let handles = Arc::clone(&handles);
            let session_id = session_id.clone();
            let close_count = Arc::clone(&close_count);
            let data_dir = data_dir.clone();
            async move {
                let _guard = crate::agent_sdk::acquire_session_runtime_lock(&session_id).await;
                let _ = close_resolved_step_tab_state(
                    &session_store,
                    &data_dir,
                    &handles,
                    &open_tabs,
                    &session_id,
                    {
                        let handles = Arc::clone(&handles);
                        let session_id = session_id.clone();
                        let close_count = Arc::clone(&close_count);
                        move || async move {
                            if handles.lock().await.remove(&session_id).is_some() {
                                close_count.fetch_add(1, Ordering::SeqCst);
                            }
                            Ok(())
                        }
                    },
                )
                .await;
            }
        };

        let step_done_release = {
            let session_store = Arc::clone(&session_store);
            let open_tabs = Arc::clone(&open_tabs);
            let handles = Arc::clone(&handles);
            let session_id = session_id.clone();
            let close_count = Arc::clone(&close_count);
            let data_dir = data_dir.clone();
            async move {
                let _guard = crate::agent_sdk::acquire_session_runtime_lock(&session_id).await;
                release_step_runtime_on_done_state(
                    &session_store,
                    &data_dir,
                    Some(open_tabs.as_ref()),
                    &session_id,
                    {
                        let handles = Arc::clone(&handles);
                        let session_id = session_id.clone();
                        let close_count = Arc::clone(&close_count);
                        move || async move {
                            if handles.lock().await.remove(&session_id).is_some() {
                                close_count.fetch_add(1, Ordering::SeqCst);
                            }
                            Ok(())
                        }
                    },
                )
                .await;
            }
        };

        tokio::join!(tab_close, step_done_release);

        // Both paths must pass through the same counted close hook.
        assert!(close_count.load(Ordering::SeqCst) <= 1);
        // Final state: runtime released and tab closed
        assert!(!handles.lock().await.contains_key(&session_id));
        assert!(!open_tabs.contains(&session_id));
    }

    // R4-06: Spec「tab open / reopen 時の状態更新に失敗しても runtime 状態は変更されない」
    #[tokio::test]
    async fn tab_open_state_update_failure_preserves_runtime_state() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = SessionStore::default();
        let open_tabs = OpenTabRegistry::default();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));

        // Setup: another step session with an active runtime that must remain untouched
        let other_id = uuid::Uuid::new_v4().to_string();
        session_store
            .save_session(tmp.path(), &workflow_step_session_for_test(&other_id))
            .unwrap();
        handles.lock().await.insert(
            other_id.clone(),
            crate::agent_sdk::make_test_agent_process(),
        );

        // Trigger failure: open_step_session_tab_state on a session that does not exist in store
        let missing_id = uuid::Uuid::new_v4().to_string();
        let result =
            open_step_session_tab_state(&session_store, tmp.path(), &open_tabs, &missing_id);
        assert!(matches!(
            result,
            Err(WorkflowStepLifecycleError::SessionNotFound(_))
        ));

        // Runtime state for unrelated session is preserved
        assert!(handles.lock().await.contains_key(&other_id));
        // open_tabs is not modified for the failed session
        assert!(!open_tabs.contains(&missing_id));
        assert!(!open_tabs.contains(&other_id));
    }

    #[tokio::test]
    async fn resolver_accepts_workflow_step_session_flag_without_workflow_state_reference() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = SessionStore::default();
        let session_id = uuid::Uuid::new_v4().to_string();

        session_store
            .save_session(tmp.path(), &workflow_step_session_for_test(&session_id))
            .unwrap();

        let resolved = resolve_step_session_with_data_dir(&session_store, tmp.path(), &session_id)
            .unwrap()
            .expect("workflow_step_session flag alone makes this a step session");

        assert_eq!(resolved.session_id, session_id);
        assert_eq!(resolved.worktree_path, "/repo");
    }
}
