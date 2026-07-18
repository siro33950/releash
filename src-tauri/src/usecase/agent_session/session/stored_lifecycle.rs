use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;

use crate::usecase::agent_session::notice::{
    AgentSessionNoticeOperation, AgentSessionNoticeUsecase,
};

use super::{
    lifecycle_controller::SessionLifecycleController, resolve_session_backend,
    validate_session_permission_mode, ChatSession, SessionBackendResolver, SessionStore,
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

#[async_trait]
pub(crate) trait WorkflowNodeSessionRestorer: Send + Sync {
    async fn try_open_tab(&self, session_id: &str) -> Result<Option<String>, String>;
    async fn try_close_tab(&self, session_id: &str) -> Result<Option<String>, String>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CloseSessionOutcome {
    StoredSessionClosed,
    WorkflowNodeTabClosed { worktree_path: String },
}

#[async_trait]
pub(crate) trait StoredSessionClosePort: Send + Sync {
    async fn close_session(
        &self,
        data_dir: &Path,
        session_id: &str,
    ) -> Result<CloseSessionOutcome, String>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RestoreSessionOutcome {
    StoredSessionRestored,
    WorkflowNodeTabRestored { worktree_path: String },
}

pub(crate) struct StoredSessionLifecycleUsecase {
    session_store: Arc<SessionStore>,
    backend_lifecycle: Arc<dyn AgentSessionBackendLifecycleGateway>,
    runtime_closer: Arc<dyn AgentSessionRuntimeCloser>,
    workflow_node_restorer: Arc<dyn WorkflowNodeSessionRestorer>,
    notice_usecase: Arc<AgentSessionNoticeUsecase>,
}

impl StoredSessionLifecycleUsecase {
    pub(crate) fn new(
        session_store: Arc<SessionStore>,
        backend_lifecycle: Arc<dyn AgentSessionBackendLifecycleGateway>,
        runtime_closer: Arc<dyn AgentSessionRuntimeCloser>,
        workflow_node_restorer: Arc<dyn WorkflowNodeSessionRestorer>,
        notice_usecase: Arc<AgentSessionNoticeUsecase>,
    ) -> Self {
        Self {
            session_store,
            backend_lifecycle,
            runtime_closer,
            workflow_node_restorer,
            notice_usecase,
        }
    }

    pub(crate) async fn archive_session(
        &self,
        data_dir: &Path,
        session_id: &str,
    ) -> Result<(), String> {
        let result = async {
            self.session_store.archive_session(data_dir, session_id)?;
            self.sync_archive(data_dir, session_id, "archive").await;
            Ok(())
        }
        .await;
        self.notice_usecase.record_operation_result(
            session_id,
            AgentSessionNoticeOperation::ArchiveSession,
            &result,
            "セッションアーカイブに失敗",
        );
        result
    }

    pub(crate) async fn archive_open_session(
        &self,
        data_dir: &Path,
        session_id: &str,
    ) -> Result<(), String> {
        let result = async {
            self.runtime_closer.close_agent_session(session_id).await?;
            self.session_store
                .archive_open_session(data_dir, session_id)?;
            self.sync_archive(data_dir, session_id, "open-thread archive")
                .await;
            Ok(())
        }
        .await;
        self.notice_usecase.record_operation_result(
            session_id,
            AgentSessionNoticeOperation::ArchiveSession,
            &result,
            "セッションアーカイブに失敗",
        );
        result
    }

    pub(crate) async fn close_session(
        &self,
        data_dir: &Path,
        session_id: &str,
    ) -> Result<CloseSessionOutcome, String> {
        let result = async {
            if let Some(worktree_path) = self
                .workflow_node_restorer
                .try_close_tab(session_id)
                .await?
            {
                return Ok(CloseSessionOutcome::WorkflowNodeTabClosed { worktree_path });
            }
            self.runtime_closer.close_agent_session(session_id).await?;
            SessionLifecycleController {
                session_store: &self.session_store,
                data_dir,
            }
            .close_session_state(session_id)?;
            Ok(CloseSessionOutcome::StoredSessionClosed)
        }
        .await;
        self.notice_usecase.record_operation_result(
            session_id,
            AgentSessionNoticeOperation::CloseSession,
            &result,
            "セッションクローズに失敗",
        );
        result
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
    ) -> Result<RestoreSessionOutcome, String> {
        let result = async {
            if let Some(worktree_path) =
                self.workflow_node_restorer.try_open_tab(session_id).await?
            {
                return Ok(RestoreSessionOutcome::WorkflowNodeTabRestored { worktree_path });
            }

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
            SessionLifecycleController {
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
            Ok(RestoreSessionOutcome::StoredSessionRestored)
        }
        .await;
        self.notice_usecase.record_operation_result(
            session_id,
            AgentSessionNoticeOperation::RestoreSession,
            &result,
            "セッション復元に失敗",
        );
        result
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

#[async_trait]
impl StoredSessionClosePort for StoredSessionLifecycleUsecase {
    async fn close_session(
        &self,
        data_dir: &Path,
        session_id: &str,
    ) -> Result<CloseSessionOutcome, String> {
        StoredSessionLifecycleUsecase::close_session(self, data_dir, session_id).await
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
    use crate::usecase::agent_session::notice_query_service::AgentSessionNoticeQueryService;
    use crate::usecase::agent_session::notice_state::new_shared_agent_session_notice_state;
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
        error: Mutex<Option<String>>,
    }

    #[derive(Default)]
    struct FakeWorkflowNodeRestorer {
        worktree_path: Mutex<Option<String>>,
        error: Mutex<Option<String>>,
    }

    #[async_trait]
    impl WorkflowNodeSessionRestorer for FakeWorkflowNodeRestorer {
        async fn try_open_tab(&self, _session_id: &str) -> Result<Option<String>, String> {
            if let Some(error) = self.error.lock().clone() {
                return Err(error);
            }
            Ok(self.worktree_path.lock().clone())
        }

        async fn try_close_tab(&self, _session_id: &str) -> Result<Option<String>, String> {
            if let Some(error) = self.error.lock().clone() {
                return Err(error);
            }
            Ok(self.worktree_path.lock().clone())
        }
    }

    #[async_trait]
    impl AgentSessionRuntimeCloser for FakeRuntimeCloser {
        async fn close_agent_session(&self, session_id: &str) -> Result<(), String> {
            self.closed.lock().push(session_id.to_string());
            match self.error.lock().clone() {
                Some(error) => Err(error),
                None => Ok(()),
            }
        }
    }

    fn codex_session(id: &str, state: SessionState) -> ChatSession {
        ChatSession {
            id: id.to_string(),
            worktree_path: "/repo".to_string(),
            messages: Vec::new(),
            state,
            error_reason: None,
            created_at: 1.0,
            updated_at: 1.0,
            agent_session_id: Some("thread-1".to_string()),
            context_carry: None,
            permission_mode: "edit".to_string(),
            plan_mode: false,
            selected_model: Some("gpt-5.1-codex".to_string()),
            permission_profile_id: Some("profile-1".to_string()),
            backend_id: Some("codex".to_string()),
            workflow_node_session: false,
            workflow_node_context: None,
            context_epoch: None,
        }
    }

    fn usecase(
        store: Arc<SessionStore>,
        backend_lifecycle: Arc<FakeBackendLifecycle>,
        runtime: Arc<FakeRuntimeCloser>,
    ) -> StoredSessionLifecycleUsecase {
        usecase_with_notice(
            store,
            backend_lifecycle,
            runtime,
            Arc::new(FakeWorkflowNodeRestorer::default()),
            Arc::new(AgentSessionNoticeUsecase::default()),
        )
    }

    fn usecase_with_notice(
        store: Arc<SessionStore>,
        backend_lifecycle: Arc<FakeBackendLifecycle>,
        runtime: Arc<FakeRuntimeCloser>,
        workflow_node_restorer: Arc<FakeWorkflowNodeRestorer>,
        notice_usecase: Arc<AgentSessionNoticeUsecase>,
    ) -> StoredSessionLifecycleUsecase {
        StoredSessionLifecycleUsecase::new(
            store,
            backend_lifecycle,
            runtime,
            workflow_node_restorer,
            notice_usecase,
        )
    }

    fn notice_services() -> (
        Arc<AgentSessionNoticeUsecase>,
        AgentSessionNoticeQueryService,
    ) {
        let state = new_shared_agent_session_notice_state();
        let query_service = AgentSessionNoticeQueryService::new(state.clone());
        (
            Arc::new(AgentSessionNoticeUsecase::new_for_test(state)),
            query_service,
        )
    }

    #[tokio::test]
    async fn archive_session_updates_store_before_thread_archive() {
        use crate::adaptor::controller::agent_session_notice_wiring::register_session_notice_cleanup_listener;
        use crate::usecase::agent_session::notice::AgentSessionNoticeUpdate;

        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(crate::test_support::build_session_store());
        let backend_lifecycle = Arc::new(FakeBackendLifecycle::new());
        let runtime = Arc::new(FakeRuntimeCloser::default());
        let (notice_usecase, notice_query) = notice_services();
        register_session_notice_cleanup_listener(store.as_ref(), notice_usecase.clone());
        let session_id = "a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d";
        store
            .save_full_session_for_migration_or_restore(
                tmp.path(),
                &codex_session(session_id, SessionState::Closed),
            )
            .unwrap();
        notice_usecase.update(
            session_id,
            AgentSessionNoticeUpdate::Failure {
                operation: AgentSessionNoticeOperation::Send,
                message: "send failed".to_string(),
            },
        );

        usecase_with_notice(
            store.clone(),
            backend_lifecycle.clone(),
            runtime,
            Arc::new(FakeWorkflowNodeRestorer::default()),
            notice_usecase.clone(),
        )
        .archive_session(tmp.path(), session_id)
        .await
        .unwrap();

        let saved = store
            .get_session_shell(tmp.path(), session_id)
            .unwrap()
            .unwrap();
        assert_eq!(saved.state, SessionState::Archived);
        assert_eq!(notice_query.get(session_id).notice, None);
        let archived = backend_lifecycle.archived.lock();
        assert_eq!(archived.len(), 1);
        assert_eq!(archived[0].backend_id.as_deref(), Some("codex"));
        assert_eq!(archived[0].agent_session_id.as_deref(), Some("thread-1"));
    }

    #[tokio::test]
    async fn close_session_stops_runtime_before_marking_session_closed() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(crate::test_support::build_session_store());
        let backend_lifecycle = Arc::new(FakeBackendLifecycle::new());
        let runtime = Arc::new(FakeRuntimeCloser::default());
        let session_id = "a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d";
        store
            .save_full_session_for_migration_or_restore(
                tmp.path(),
                &codex_session(session_id, SessionState::Idle),
            )
            .unwrap();

        let outcome = usecase(store.clone(), backend_lifecycle, runtime.clone())
            .close_session(tmp.path(), session_id)
            .await
            .unwrap();

        assert_eq!(outcome, CloseSessionOutcome::StoredSessionClosed);
        assert_eq!(runtime.closed.lock().as_slice(), [session_id]);
        assert_eq!(
            store
                .get_session_shell(tmp.path(), session_id)
                .unwrap()
                .unwrap()
                .state,
            SessionState::Closed
        );
    }

    #[tokio::test]
    async fn workspace_node_close_discards_notice_through_session_state_listener() {
        use crate::adaptor::controller::agent_session_notice_wiring::register_session_notice_cleanup_listener;
        use crate::usecase::agent_session::notice::AgentSessionNoticeUpdate;

        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(crate::test_support::build_session_store());
        let (notice_usecase, notice_query) = notice_services();
        register_session_notice_cleanup_listener(store.as_ref(), notice_usecase.clone());
        let session_id = "a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d";
        store
            .save_full_session_for_migration_or_restore(
                tmp.path(),
                &codex_session(session_id, SessionState::Idle),
            )
            .unwrap();
        notice_usecase.update(
            session_id,
            AgentSessionNoticeUpdate::Failure {
                operation: AgentSessionNoticeOperation::Send,
                message: "send failed".to_string(),
            },
        );

        usecase_with_notice(
            store,
            Arc::new(FakeBackendLifecycle::new()),
            Arc::new(FakeRuntimeCloser::default()),
            Arc::new(FakeWorkflowNodeRestorer::default()),
            notice_usecase.clone(),
        )
        .close_session(tmp.path(), session_id)
        .await
        .unwrap();

        assert_eq!(notice_query.get(session_id).notice, None);
    }

    #[tokio::test]
    async fn close_session_keeps_session_open_when_runtime_close_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(crate::test_support::build_session_store());
        let backend_lifecycle = Arc::new(FakeBackendLifecycle::new());
        let runtime = Arc::new(FakeRuntimeCloser::default());
        *runtime.error.lock() = Some("runtime unavailable".to_string());
        let session_id = "a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d";
        store
            .save_full_session_for_migration_or_restore(
                tmp.path(),
                &codex_session(session_id, SessionState::Idle),
            )
            .unwrap();

        let error = usecase(store.clone(), backend_lifecycle, runtime)
            .close_session(tmp.path(), session_id)
            .await
            .unwrap_err();

        assert_eq!(error, "runtime unavailable");
        assert_eq!(
            store
                .get_session_shell(tmp.path(), session_id)
                .unwrap()
                .unwrap()
                .state,
            SessionState::Idle
        );
    }

    #[tokio::test]
    async fn close_session_workflow_node_uses_same_notice_aware_lifecycle() {
        let (notice_usecase, notice_query) = notice_services();
        notice_usecase.update(
            "workflow-session",
            crate::usecase::agent_session::notice::AgentSessionNoticeUpdate::Failure {
                operation: AgentSessionNoticeOperation::CloseSession,
                message: "previous close failed".to_string(),
            },
        );
        let restorer = Arc::new(FakeWorkflowNodeRestorer::default());
        *restorer.worktree_path.lock() = Some("/repo".to_string());
        let runtime = Arc::new(FakeRuntimeCloser::default());

        let outcome = usecase_with_notice(
            Arc::new(crate::test_support::build_session_store()),
            Arc::new(FakeBackendLifecycle::new()),
            runtime.clone(),
            restorer,
            notice_usecase,
        )
        .close_session(Path::new("/unused"), "workflow-session")
        .await
        .unwrap();

        assert_eq!(
            outcome,
            CloseSessionOutcome::WorkflowNodeTabClosed {
                worktree_path: "/repo".to_string(),
            }
        );
        assert!(runtime.closed.lock().is_empty());
        assert!(notice_query.get("workflow-session").notice.is_none());
    }

    #[tokio::test]
    async fn close_session_failure_records_close_notice_in_shared_lifecycle() {
        let store = Arc::new(crate::test_support::build_session_store());
        let runtime = Arc::new(FakeRuntimeCloser::default());
        *runtime.error.lock() = Some("runtime unavailable".to_string());
        let (notice_usecase, notice_query) = notice_services();

        let error = usecase_with_notice(
            store,
            Arc::new(FakeBackendLifecycle::new()),
            runtime,
            Arc::new(FakeWorkflowNodeRestorer::default()),
            notice_usecase,
        )
        .close_session(Path::new("/unused"), "session-a")
        .await
        .unwrap_err();

        assert_eq!(error, "runtime unavailable");
        assert_eq!(
            notice_query.get("session-a").notice.unwrap().message,
            "セッションクローズに失敗: runtime unavailable"
        );
    }

    #[tokio::test]
    async fn archive_session_failure_records_session_history_notice() {
        use crate::usecase::agent_session::notice_state::AgentSessionNotice;

        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(crate::test_support::build_session_store());
        let (notice_usecase, notice_query) = notice_services();
        let session_id = "a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d";
        store
            .save_full_session_for_migration_or_restore(
                tmp.path(),
                &codex_session(session_id, SessionState::Idle),
            )
            .unwrap();

        usecase_with_notice(
            store,
            Arc::new(FakeBackendLifecycle::new()),
            Arc::new(FakeRuntimeCloser::default()),
            Arc::new(FakeWorkflowNodeRestorer::default()),
            notice_usecase.clone(),
        )
        .archive_session(tmp.path(), session_id)
        .await
        .unwrap_err();

        assert_eq!(
            notice_query.get(session_id).notice,
            Some(AgentSessionNotice {
                message: "セッションアーカイブに失敗: Only closed sessions can be archived"
                    .to_string(),
            })
        );
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

        let outcome = usecase(store.clone(), backend_lifecycle.clone(), runtime)
            .restore_session(
                tmp.path(),
                "a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d",
                &registry,
            )
            .await
            .unwrap();

        assert_eq!(outcome, RestoreSessionOutcome::StoredSessionRestored);
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

    #[tokio::test]
    async fn restore_session_failure_records_session_history_notice() {
        use crate::usecase::agent_session::notice_state::AgentSessionNotice;

        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(crate::test_support::build_session_store());
        let (notice_usecase, notice_query) = notice_services();
        let session_id = "a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d";
        let registry = AgentBackendRegistry::new();

        usecase_with_notice(
            store,
            Arc::new(FakeBackendLifecycle::new()),
            Arc::new(FakeRuntimeCloser::default()),
            Arc::new(FakeWorkflowNodeRestorer::default()),
            notice_usecase.clone(),
        )
        .restore_session(tmp.path(), session_id, &registry)
        .await
        .unwrap_err();

        assert_eq!(
            notice_query.get(session_id).notice,
            Some(AgentSessionNotice {
                message: format!("セッション復元に失敗: Session not found: {session_id}"),
            })
        );
    }

    #[tokio::test]
    async fn restore_workflow_node_success_clears_restore_notice_in_usecase() {
        use crate::usecase::agent_session::notice::AgentSessionNoticeUpdate;

        let store = Arc::new(crate::test_support::build_session_store());
        let backend_lifecycle = Arc::new(FakeBackendLifecycle::new());
        let (notice_usecase, notice_query) = notice_services();
        let restorer = Arc::new(FakeWorkflowNodeRestorer::default());
        *restorer.worktree_path.lock() = Some("/repo".to_string());
        notice_usecase.update(
            "workflow-session",
            AgentSessionNoticeUpdate::Failure {
                operation: AgentSessionNoticeOperation::RestoreSession,
                message: "previous restore failed".to_string(),
            },
        );

        let outcome = usecase_with_notice(
            store,
            backend_lifecycle.clone(),
            Arc::new(FakeRuntimeCloser::default()),
            restorer,
            notice_usecase.clone(),
        )
        .restore_session(
            Path::new("/unused"),
            "workflow-session",
            &AgentBackendRegistry::new(),
        )
        .await
        .unwrap();

        assert_eq!(
            outcome,
            RestoreSessionOutcome::WorkflowNodeTabRestored {
                worktree_path: "/repo".to_string(),
            }
        );
        assert!(notice_query.get("workflow-session").notice.is_none());
        assert!(backend_lifecycle.unarchived.lock().is_empty());
    }

    #[tokio::test]
    async fn restore_workflow_node_failure_records_restore_notice_in_usecase() {
        let (notice_usecase, notice_query) = notice_services();
        let restorer = Arc::new(FakeWorkflowNodeRestorer::default());
        *restorer.error.lock() = Some(
            "workflow_node_tab_operation_failed: workflow node tab operation failed".to_string(),
        );

        let error = usecase_with_notice(
            Arc::new(crate::test_support::build_session_store()),
            Arc::new(FakeBackendLifecycle::new()),
            Arc::new(FakeRuntimeCloser::default()),
            restorer,
            notice_usecase.clone(),
        )
        .restore_session(
            Path::new("/unused"),
            "workflow-session",
            &AgentBackendRegistry::new(),
        )
        .await
        .unwrap_err();

        assert_eq!(
            error,
            "workflow_node_tab_operation_failed: workflow node tab operation failed"
        );
        assert_eq!(
            notice_query
                .get("workflow-session")
                .notice
                .unwrap()
                .message,
            "セッション復元に失敗: workflow_node_tab_operation_failed: workflow node tab operation failed"
        );
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
