use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;

use super::{
    lifecycle_controller::SessionLifecycleController, resolve_session_backend,
    validate_session_permission_mode, ChatSession, RestoreSessionResponse, SessionBackendResolver,
    SessionStore,
};

const CODEX_BACKEND_ID: &str = "codex";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CodexThreadForkRequest {
    pub thread_id: String,
    pub cwd: String,
    pub model: Option<String>,
    pub permission_mode: String,
    pub plan_mode: bool,
    pub permission_profile_id: Option<String>,
}

#[async_trait]
pub(crate) trait CodexThreadLifecycleGateway: Send + Sync {
    async fn archive_thread(&self, thread_id: &str) -> Result<(), String>;
    async fn unarchive_thread(&self, thread_id: &str) -> Result<(), String>;
    async fn fork_thread(&self, request: CodexThreadForkRequest) -> Result<String, String>;
}

#[async_trait]
pub(crate) trait AgentSessionRuntimeCloser: Send + Sync {
    async fn close_agent_session(&self, session_id: &str) -> Result<(), String>;
}

pub(crate) struct StoredSessionLifecycleUsecase {
    session_store: Arc<SessionStore>,
    thread_lifecycle: Arc<dyn CodexThreadLifecycleGateway>,
    runtime_closer: Arc<dyn AgentSessionRuntimeCloser>,
}

impl StoredSessionLifecycleUsecase {
    pub(crate) fn new(
        session_store: Arc<SessionStore>,
        thread_lifecycle: Arc<dyn CodexThreadLifecycleGateway>,
        runtime_closer: Arc<dyn AgentSessionRuntimeCloser>,
    ) -> Self {
        Self {
            session_store,
            thread_lifecycle,
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
        let Some(thread_id) = saved_codex_thread_id(&source_session) else {
            return Ok(forked);
        };

        let request = CodexThreadForkRequest {
            thread_id,
            cwd: source_session.worktree_path.clone(),
            model: source_session.selected_model.clone(),
            permission_mode: source_session.permission_mode.clone(),
            plan_mode: source_session.plan_mode,
            permission_profile_id: source_session.permission_profile_id.clone(),
        };
        match self.thread_lifecycle.fork_thread(request).await {
            Ok(thread_id) => {
                forked.agent_session_id = Some(thread_id);
                self.session_store.update_agent_session_id(
                    data_dir,
                    &forked.id,
                    forked.agent_session_id.clone(),
                )?;
            }
            Err(err) => {
                log::debug!("skipped Codex runtime thread fork sync for {session_id}: {err}");
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
        let codex_thread_id = saved_codex_thread_id(&session);
        let response = SessionLifecycleController {
            session_store: &self.session_store,
            data_dir,
        }
        .restore_session_state(session)?;
        if let Some(thread_id) = codex_thread_id {
            if let Err(err) = self.thread_lifecycle.unarchive_thread(&thread_id).await {
                log::debug!("skipped Codex runtime thread unarchive sync for {session_id}: {err}");
            }
        }
        Ok(response)
    }

    async fn sync_archive(&self, data_dir: &Path, session_id: &str, label: &str) {
        let codex_thread_id = self
            .session_store
            .get_session_shell(data_dir, session_id)
            .ok()
            .flatten()
            .and_then(|session| saved_codex_thread_id(&session));
        if let Some(thread_id) = codex_thread_id {
            if let Err(err) = self.thread_lifecycle.archive_thread(&thread_id).await {
                log::debug!("skipped Codex runtime thread {label} sync for {session_id}: {err}");
            }
        }
    }
}

pub(crate) fn saved_codex_thread_id(session: &ChatSession) -> Option<String> {
    if session.backend_id.as_deref() != Some(CODEX_BACKEND_ID) {
        return None;
    }
    session
        .agent_session_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usecase::agent_session::session::SessionState;
    use parking_lot::Mutex;

    struct FakeThreadLifecycle {
        archived: Mutex<Vec<String>>,
        unarchived: Mutex<Vec<String>>,
        forked: Mutex<Vec<CodexThreadForkRequest>>,
        fork_result: Mutex<Result<String, String>>,
    }

    impl FakeThreadLifecycle {
        fn new() -> Self {
            Self {
                archived: Mutex::new(Vec::new()),
                unarchived: Mutex::new(Vec::new()),
                forked: Mutex::new(Vec::new()),
                fork_result: Mutex::new(Ok("forked-thread".to_string())),
            }
        }
    }

    #[async_trait]
    impl CodexThreadLifecycleGateway for FakeThreadLifecycle {
        async fn archive_thread(&self, thread_id: &str) -> Result<(), String> {
            self.archived.lock().push(thread_id.to_string());
            Ok(())
        }

        async fn unarchive_thread(&self, thread_id: &str) -> Result<(), String> {
            self.unarchived.lock().push(thread_id.to_string());
            Ok(())
        }

        async fn fork_thread(&self, request: CodexThreadForkRequest) -> Result<String, String> {
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
            backend_id: Some(CODEX_BACKEND_ID.to_string()),
            workflow_step_session: false,
            workflow_step_context: None,
        }
    }

    fn usecase(
        store: Arc<SessionStore>,
        threads: Arc<FakeThreadLifecycle>,
        runtime: Arc<FakeRuntimeCloser>,
    ) -> StoredSessionLifecycleUsecase {
        StoredSessionLifecycleUsecase::new(store, threads, runtime)
    }

    #[tokio::test]
    async fn archive_session_updates_store_before_thread_archive() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(crate::test_support::build_session_store());
        let threads = Arc::new(FakeThreadLifecycle::new());
        let runtime = Arc::new(FakeRuntimeCloser::default());
        store
            .save_full_session_for_migration_or_restore(
                tmp.path(),
                &codex_session("a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d", SessionState::Closed),
            )
            .unwrap();

        usecase(store.clone(), threads.clone(), runtime)
            .archive_session(tmp.path(), "a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d")
            .await
            .unwrap();

        let saved = store
            .get_session_shell(tmp.path(), "a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d")
            .unwrap()
            .unwrap();
        assert_eq!(saved.state, SessionState::Archived);
        assert_eq!(threads.archived.lock().as_slice(), ["thread-1"]);
    }

    #[tokio::test]
    async fn fork_session_updates_forked_thread_id_after_local_fork() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(crate::test_support::build_session_store());
        let threads = Arc::new(FakeThreadLifecycle::new());
        let runtime = Arc::new(FakeRuntimeCloser::default());
        store
            .save_full_session_for_migration_or_restore(
                tmp.path(),
                &codex_session("a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d", SessionState::Idle),
            )
            .unwrap();

        let forked = usecase(store.clone(), threads.clone(), runtime)
            .fork_session(tmp.path(), "a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d")
            .await
            .unwrap();

        assert_eq!(forked.agent_session_id.as_deref(), Some("forked-thread"));
        let saved = store
            .get_session_shell(tmp.path(), &forked.id)
            .unwrap()
            .unwrap();
        assert_eq!(saved.agent_session_id.as_deref(), Some("forked-thread"));
        assert_eq!(threads.forked.lock()[0].thread_id, "thread-1");
    }

    #[test]
    fn saved_codex_thread_id_requires_codex_backend_and_non_empty_id() {
        let mut session = codex_session("session-1", SessionState::Closed);
        session.agent_session_id = Some(" thread-1 ".to_string());
        assert_eq!(
            saved_codex_thread_id(&session),
            Some("thread-1".to_string())
        );

        session.backend_id = Some("claude".to_string());
        assert_eq!(saved_codex_thread_id(&session), None);

        session.backend_id = Some(CODEX_BACKEND_ID.to_string());
        session.agent_session_id = Some("   ".to_string());
        assert_eq!(saved_codex_thread_id(&session), None);
    }
}
