use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;

use super::{
    lifecycle_controller::SessionLifecycleController, resolve_session_backend,
    validate_session_permission_mode, ChatSession, RestoreSessionResponse, SessionBackendResolver,
    SessionStore,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BackendSessionLifecycleRequest {
    pub backend_id: Option<String>,
    pub agent_session_id: Option<String>,
    pub cwd: String,
    pub model: Option<String>,
    pub permission_mode: String,
    pub plan_mode: bool,
    pub permission_profile_id: Option<String>,
}

#[async_trait]
pub(crate) trait AgentSessionBackendLifecycleGateway: Send + Sync {
    async fn archive_backend_session(
        &self,
        request: BackendSessionLifecycleRequest,
    ) -> Result<(), String>;
    async fn unarchive_backend_session(
        &self,
        request: BackendSessionLifecycleRequest,
    ) -> Result<(), String>;
    async fn fork_backend_session(
        &self,
        request: BackendSessionLifecycleRequest,
    ) -> Result<Option<String>, String>;
}

#[async_trait]
pub(crate) trait AgentSessionRuntimeCloser: Send + Sync {
    async fn close_agent_session(&self, session_id: &str) -> Result<(), String>;
}

pub(crate) struct StoredSessionLifecycleUsecase {
    session_store: Arc<SessionStore>,
    backend_lifecycle: Arc<dyn AgentSessionBackendLifecycleGateway>,
    runtime_closer: Arc<dyn AgentSessionRuntimeCloser>,
}

impl StoredSessionLifecycleUsecase {
    pub(crate) fn new(
        session_store: Arc<SessionStore>,
        backend_lifecycle: Arc<dyn AgentSessionBackendLifecycleGateway>,
        runtime_closer: Arc<dyn AgentSessionRuntimeCloser>,
    ) -> Self {
        Self {
            session_store,
            backend_lifecycle,
            runtime_closer,
        }
    }

    pub(crate) async fn archive_session(
        &self,
        data_dir: &Path,
        session_id: &str,
    ) -> Result<(), String> {
        self.session_store.archive_session(data_dir, session_id)?;
        self.sync_archive(data_dir, session_id, "archive").await;
        Ok(())
    }

    pub(crate) async fn archive_open_session(
        &self,
        data_dir: &Path,
        session_id: &str,
    ) -> Result<(), String> {
        self.runtime_closer.close_agent_session(session_id).await?;
        self.session_store
            .archive_open_session(data_dir, session_id)?;
        self.sync_archive(data_dir, session_id, "open-thread archive")
            .await;
        Ok(())
    }

    pub(crate) async fn fork_session(
        &self,
        data_dir: &Path,
        session_id: &str,
    ) -> Result<ChatSession, String> {
        let source_session = self
            .session_store
            .get_session_shell(data_dir, session_id)?
            .ok_or_else(|| format!("Session not found: {session_id}"))?;
        let mut forked = self.session_store.fork_session(data_dir, session_id)?;
        match self
            .backend_lifecycle
            .fork_backend_session(BackendSessionLifecycleRequest::from_session(
                &source_session,
            ))
            .await
        {
            Ok(Some(agent_session_id)) => {
                forked.agent_session_id = Some(agent_session_id);
                self.session_store.update_agent_session_id(
                    data_dir,
                    &forked.id,
                    forked.agent_session_id.clone(),
                )?;
            }
            Ok(None) => {}
            Err(err) => {
                log::debug!("skipped backend runtime fork sync for {session_id}: {err}");
            }
        }
        Ok(forked)
    }

    pub(crate) async fn restore_session(
        &self,
        data_dir: &Path,
        session_id: &str,
        registry: &impl SessionBackendResolver,
    ) -> Result<RestoreSessionResponse, String> {
        let mut session = self
            .session_store
            .get_session_shell(data_dir, session_id)?
            .ok_or_else(|| format!("Session not found: {session_id}"))?;
        validate_session_permission_mode(&session)?;
        let original_backend_id = session.backend_id.clone();
        resolve_session_backend(&mut session, registry)?;
        if session.backend_id != original_backend_id {
            self.session_store.update_backend_selection(
                data_dir,
                session_id,
                session
                    .backend_id
                    .clone()
                    .ok_or_else(|| format!("Session backend was not resolved: {session_id}"))?,
                session.selected_model.clone(),
            )?;
        }
        let backend_request = BackendSessionLifecycleRequest::from_session(&session);
        let response = SessionLifecycleController {
            session_store: &self.session_store,
            data_dir,
        }
        .restore_session_state(session)?;
        if let Err(err) = self
            .backend_lifecycle
            .unarchive_backend_session(backend_request)
            .await
        {
            log::debug!("skipped backend runtime unarchive sync for {session_id}: {err}");
        }
        Ok(response)
    }

    async fn sync_archive(&self, data_dir: &Path, session_id: &str, label: &str) {
        let backend_request = self
            .session_store
            .get_session_shell(data_dir, session_id)
            .ok()
            .flatten()
            .map(|session| BackendSessionLifecycleRequest::from_session(&session));
        if let Some(request) = backend_request {
            if let Err(err) = self
                .backend_lifecycle
                .archive_backend_session(request)
                .await
            {
                log::debug!("skipped backend runtime {label} sync for {session_id}: {err}");
            }
        }
    }
}

impl BackendSessionLifecycleRequest {
    pub(crate) fn from_session(session: &ChatSession) -> Self {
        Self {
            backend_id: session.backend_id.clone(),
            agent_session_id: session
                .agent_session_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string),
            cwd: session.worktree_path.clone(),
            model: session.selected_model.clone(),
            permission_mode: session.permission_mode.clone(),
            plan_mode: session.plan_mode,
            permission_profile_id: session.permission_profile_id.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agent_session::gateway::{
        AgentBackend, AgentBackendError, AgentSessionRuntime, ForkSessionRequest, SessionSpec,
    };
    use crate::domain::agent_session::value_objects::{
        BackendCapabilities, ModelDescriptor, ModelId, SkillEntry,
    };
    use crate::usecase::agent_session::backend_registry::AgentBackendRegistry;
    use crate::usecase::agent_session::session::SessionState;
    use parking_lot::Mutex;
    use std::path::Path;

    struct RegistryMockBackend {
        id: &'static str,
        model: &'static str,
    }

    #[async_trait]
    impl AgentBackend for RegistryMockBackend {
        fn id(&self) -> &str {
            self.id
        }

        fn name(&self) -> &str {
            self.id
        }

        fn available_models(&self) -> Vec<ModelDescriptor> {
            vec![ModelDescriptor {
                id: ModelId::parse(self.model).unwrap(),
                display_name: self.model.to_string(),
            }]
        }

        fn capabilities(&self) -> BackendCapabilities {
            BackendCapabilities { steering: false }
        }

        async fn open_session(
            &self,
            _spec: SessionSpec,
        ) -> Result<Box<dyn AgentSessionRuntime>, AgentBackendError> {
            Err(AgentBackendError::Other("not used".to_string()))
        }

        async fn archive_session(
            &self,
            _backend_session_id: &str,
            _cwd: &str,
        ) -> Result<(), AgentBackendError> {
            Ok(())
        }

        async fn unarchive_session(
            &self,
            _backend_session_id: &str,
            _cwd: &str,
        ) -> Result<(), AgentBackendError> {
            Ok(())
        }

        async fn fork_session(
            &self,
            _req: ForkSessionRequest,
        ) -> Result<Option<String>, AgentBackendError> {
            Ok(None)
        }

        async fn skill_catalog(
            &self,
            _cwd: &Path,
            _query: Option<&str>,
            _limit: Option<usize>,
        ) -> Result<Vec<SkillEntry>, AgentBackendError> {
            Ok(Vec::new())
        }

        async fn fuzzy_file_search(
            &self,
            _root: &Path,
            _query: &str,
            _limit: usize,
        ) -> Result<Option<Vec<String>>, AgentBackendError> {
            Ok(None)
        }
    }

    struct FakeBackendLifecycle {
        archived: Mutex<Vec<BackendSessionLifecycleRequest>>,
        unarchived: Mutex<Vec<BackendSessionLifecycleRequest>>,
        forked: Mutex<Vec<BackendSessionLifecycleRequest>>,
        fork_result: Mutex<Result<Option<String>, String>>,
    }

    impl FakeBackendLifecycle {
        fn new() -> Self {
            Self {
                archived: Mutex::new(Vec::new()),
                unarchived: Mutex::new(Vec::new()),
                forked: Mutex::new(Vec::new()),
                fork_result: Mutex::new(Ok(Some("forked-thread".to_string()))),
            }
        }
    }

    #[async_trait]
    impl AgentSessionBackendLifecycleGateway for FakeBackendLifecycle {
        async fn archive_backend_session(
            &self,
            request: BackendSessionLifecycleRequest,
        ) -> Result<(), String> {
            self.archived.lock().push(request);
            Ok(())
        }

        async fn unarchive_backend_session(
            &self,
            request: BackendSessionLifecycleRequest,
        ) -> Result<(), String> {
            self.unarchived.lock().push(request);
            Ok(())
        }

        async fn fork_backend_session(
            &self,
            request: BackendSessionLifecycleRequest,
        ) -> Result<Option<String>, String> {
            self.forked.lock().push(request);
            self.fork_result.lock().clone()
        }
    }

    #[derive(Default)]
    struct FakeRuntimeCloser {
        closed: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl AgentSessionRuntimeCloser for FakeRuntimeCloser {
        async fn close_agent_session(&self, session_id: &str) -> Result<(), String> {
            self.closed.lock().push(session_id.to_string());
            Ok(())
        }
    }

    fn codex_session(id: &str, state: SessionState) -> ChatSession {
        ChatSession {
            id: id.to_string(),
            worktree_path: "/repo".to_string(),
            messages: Vec::new(),
            state,
            created_at: 1.0,
            updated_at: 1.0,
            agent_session_id: Some("thread-1".to_string()),
            context_carry: None,
            permission_mode: "edit".to_string(),
            plan_mode: false,
            selected_model: Some("gpt-5.1-codex".to_string()),
            permission_profile_id: Some("profile-1".to_string()),
            backend_id: Some("codex".to_string()),
            workflow_step_session: false,
            workflow_step_context: None,
            context_epoch: None,
        }
    }

    fn usecase(
        store: Arc<SessionStore>,
        backend_lifecycle: Arc<FakeBackendLifecycle>,
        runtime: Arc<FakeRuntimeCloser>,
    ) -> StoredSessionLifecycleUsecase {
        StoredSessionLifecycleUsecase::new(store, backend_lifecycle, runtime)
    }

    #[tokio::test]
    async fn archive_session_updates_store_before_thread_archive() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(crate::test_support::build_session_store());
        let backend_lifecycle = Arc::new(FakeBackendLifecycle::new());
        let runtime = Arc::new(FakeRuntimeCloser::default());
        store
            .save_full_session_for_migration_or_restore(
                tmp.path(),
                &codex_session("a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d", SessionState::Closed),
            )
            .unwrap();

        usecase(store.clone(), backend_lifecycle.clone(), runtime)
            .archive_session(tmp.path(), "a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d")
            .await
            .unwrap();

        let saved = store
            .get_session_shell(tmp.path(), "a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d")
            .unwrap()
            .unwrap();
        assert_eq!(saved.state, SessionState::Archived);
        let archived = backend_lifecycle.archived.lock();
        assert_eq!(archived.len(), 1);
        assert_eq!(archived[0].backend_id.as_deref(), Some("codex"));
        assert_eq!(archived[0].agent_session_id.as_deref(), Some("thread-1"));
    }

    #[tokio::test]
    async fn fork_session_updates_forked_thread_id_after_local_fork() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(crate::test_support::build_session_store());
        let backend_lifecycle = Arc::new(FakeBackendLifecycle::new());
        let runtime = Arc::new(FakeRuntimeCloser::default());
        store
            .save_full_session_for_migration_or_restore(
                tmp.path(),
                &codex_session("a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d", SessionState::Idle),
            )
            .unwrap();

        let forked = usecase(store.clone(), backend_lifecycle.clone(), runtime)
            .fork_session(tmp.path(), "a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d")
            .await
            .unwrap();

        assert_eq!(forked.agent_session_id.as_deref(), Some("forked-thread"));
        let saved = store
            .get_session_shell(tmp.path(), &forked.id)
            .unwrap()
            .unwrap();
        assert_eq!(saved.agent_session_id.as_deref(), Some("forked-thread"));
        let forked_requests = backend_lifecycle.forked.lock();
        assert_eq!(forked_requests[0].backend_id.as_deref(), Some("codex"));
        assert_eq!(
            forked_requests[0].agent_session_id.as_deref(),
            Some("thread-1")
        );
    }

    #[tokio::test]
    async fn restore_session_restores_idle_state_and_unarchives_selected_backend() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(crate::test_support::build_session_store());
        let backend_lifecycle = Arc::new(FakeBackendLifecycle::new());
        let runtime = Arc::new(FakeRuntimeCloser::default());
        store
            .save_full_session_for_migration_or_restore(
                tmp.path(),
                &codex_session("a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d", SessionState::Closed),
            )
            .unwrap();
        let mut registry = AgentBackendRegistry::new();
        registry.register(Arc::new(RegistryMockBackend {
            id: "claude",
            model: "claude-opus-4-8",
        }));
        registry.register(Arc::new(RegistryMockBackend {
            id: "codex",
            model: "gpt-5.1-codex",
        }));
        registry.set_default(Some("claude".to_string()));

        let response = usecase(store.clone(), backend_lifecycle.clone(), runtime)
            .restore_session(
                tmp.path(),
                "a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d",
                &registry,
            )
            .await
            .unwrap();

        assert!(!response.restored_workflow_step);
        let saved = store
            .get_session_shell(tmp.path(), "a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d")
            .unwrap()
            .unwrap();
        assert_eq!(saved.state, SessionState::Idle);
        let unarchived = backend_lifecycle.unarchived.lock();
        assert_eq!(unarchived.len(), 1);
        assert_eq!(unarchived[0].backend_id.as_deref(), Some("codex"));
        assert_eq!(unarchived[0].agent_session_id.as_deref(), Some("thread-1"));
    }

    #[test]
    fn backend_lifecycle_request_trims_non_empty_agent_session_id() {
        let mut session = codex_session("session-1", SessionState::Closed);
        session.agent_session_id = Some(" thread-1 ".to_string());
        assert_eq!(
            BackendSessionLifecycleRequest::from_session(&session).agent_session_id,
            Some("thread-1".to_string())
        );

        session.backend_id = Some("claude".to_string());
        let request = BackendSessionLifecycleRequest::from_session(&session);
        assert_eq!(request.backend_id.as_deref(), Some("claude"));
        assert_eq!(request.agent_session_id.as_deref(), Some("thread-1"));

        session.agent_session_id = Some("   ".to_string());
        assert_eq!(
            BackendSessionLifecycleRequest::from_session(&session).agent_session_id,
            None
        );
    }
}
