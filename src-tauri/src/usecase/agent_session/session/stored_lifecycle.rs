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

#[async_trait]
pub(crate) trait WorkflowNodeSessionRestorer: Send + Sync {
    async fn try_open_tab(&self, session_id: &str) -> Result<Option<String>, String>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RestoreSessionOutcome {
    StoredSessionRestored,
    WorkflowNodeTabRestored { worktree_path: String },
}

pub(crate) struct StoredSessionLifecycleUsecase {
    session_store: Arc<SessionStore>,
    workflow_node_restorer: Arc<dyn WorkflowNodeSessionRestorer>,
    notice_usecase: Arc<AgentSessionNoticeUsecase>,
}

impl StoredSessionLifecycleUsecase {
    pub(crate) fn new(
        session_store: Arc<SessionStore>,
        workflow_node_restorer: Arc<dyn WorkflowNodeSessionRestorer>,
        notice_usecase: Arc<AgentSessionNoticeUsecase>,
    ) -> Self {
        Self {
            session_store,
            workflow_node_restorer,
            notice_usecase,
        }
    }

    #[cfg(test)]
    pub(crate) async fn archive_session(
        &self,
        data_dir: &Path,
        session_id: &str,
    ) -> Result<(), String> {
        let result = async {
            self.session_store.archive_session(data_dir, session_id)?;
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

    pub(crate) async fn fork_session(
        &self,
        data_dir: &Path,
        session_id: &str,
    ) -> Result<ChatSession, String> {
        self.session_store.fork_session(data_dir, session_id)
    }

    pub(crate) async fn restore_session(
        &self,
        data_dir: &Path,
        session_id: &str,
        registry: &impl SessionBackendResolver,
    ) -> Result<RestoreSessionOutcome, String> {
        if let Some(worktree_path) = self.workflow_node_restorer.try_open_tab(session_id).await? {
            return Ok(RestoreSessionOutcome::WorkflowNodeTabRestored { worktree_path });
        }
        let result = async {
            let mut session = self
                .session_store
                .get_session_shell(data_dir, session_id)?
                .ok_or_else(|| format!("Session not found: {session_id}"))?;
            validate_session_permission_mode(&session)?;
            let original_backend_id = session.backend_id.clone();
            resolve_session_backend(&mut session, registry)?;
            if session.backend_id != original_backend_id {
                self.session_store.update_backend_selection_from_user(
                    data_dir,
                    session_id,
                    session
                        .backend_id
                        .clone()
                        .ok_or_else(|| format!("Session backend was not resolved: {session_id}"))?,
                    session.selected_model.clone(),
                )?;
            }
            SessionLifecycleController {
                session_store: &self.session_store,
                data_dir,
            }
            .restore_session_state(session)?;
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agent_session::gateway::{
        AgentBackend, AgentBackendError, AgentSessionRuntime, SessionSpec,
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

    fn usecase(store: Arc<SessionStore>) -> StoredSessionLifecycleUsecase {
        usecase_with_notice(
            store,
            Arc::new(FakeWorkflowNodeRestorer::default()),
            Arc::new(AgentSessionNoticeUsecase::default()),
        )
    }

    fn usecase_with_notice(
        store: Arc<SessionStore>,
        workflow_node_restorer: Arc<FakeWorkflowNodeRestorer>,
        notice_usecase: Arc<AgentSessionNoticeUsecase>,
    ) -> StoredSessionLifecycleUsecase {
        StoredSessionLifecycleUsecase::new(store, workflow_node_restorer, notice_usecase)
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
    async fn archive_session_preserves_transient_notice() {
        use crate::usecase::agent_session::notice::AgentSessionNoticeUpdate;

        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(crate::test_support::build_session_store());
        let (notice_usecase, notice_query) = notice_services();
        let session_id = "a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d";
        store
            .save_full_session_for_restore(
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
        assert_eq!(
            notice_query.get(session_id).notice.unwrap().message,
            "send failed"
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
            .save_full_session_for_restore(
                tmp.path(),
                &codex_session(session_id, SessionState::Idle),
            )
            .unwrap();

        usecase_with_notice(
            store,
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
    async fn fork_session_is_local_only_and_requires_fresh_provider_establishment() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(crate::test_support::build_session_store());
        store
            .save_full_session_for_restore(
                tmp.path(),
                &codex_session("a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d", SessionState::Idle),
            )
            .unwrap();

        let forked = usecase(store.clone())
            .fork_session(tmp.path(), "a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d")
            .await
            .unwrap();

        assert_eq!(forked.agent_session_id, None);
        assert_eq!(forked.context_carry, None);
        let saved = store
            .get_session_shell(tmp.path(), &forked.id)
            .unwrap()
            .unwrap();
        assert_eq!(saved.agent_session_id, None);
        assert_eq!(saved.context_carry, None);
    }

    #[tokio::test]
    async fn restore_session_restores_idle_state_without_eager_provider_effect() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(crate::test_support::build_session_store());
        store
            .save_full_session_for_restore(
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

        let outcome = usecase(store.clone())
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
    async fn restore_workflow_node_is_view_only_and_preserves_notice() {
        use crate::usecase::agent_session::notice::AgentSessionNoticeUpdate;

        let store = Arc::new(crate::test_support::build_session_store());
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

        let outcome = usecase_with_notice(store, restorer, notice_usecase.clone())
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
        assert_eq!(
            notice_query.get("workflow-session").notice.unwrap().message,
            "previous restore failed"
        );
    }

    #[tokio::test]
    async fn restore_workflow_node_failure_does_not_create_feedback() {
        let (notice_usecase, notice_query) = notice_services();
        let restorer = Arc::new(FakeWorkflowNodeRestorer::default());
        *restorer.error.lock() = Some(
            "workflow_node_tab_operation_failed: workflow node tab operation failed".to_string(),
        );

        let error = usecase_with_notice(
            Arc::new(crate::test_support::build_session_store()),
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
        assert!(notice_query.get("workflow-session").notice.is_none());
    }
}
