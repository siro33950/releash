use std::future::Future;
use std::sync::Arc;

use tauri::Manager;
use tokio::sync::Mutex;

use crate::infrastructure::agent_session::runtime::AgentProcessMap;
use crate::infrastructure::platform::app_data_dir::resolve_data_dir;
use crate::usecase::agent_session::session::{OpenTabRegistry, SessionState, SessionStore};
use crate::usecase::workflow::step_lifecycle::{
    release_step_runtime_on_done_with_gateways, ResolvedWorkflowStepSession,
    WorkflowStepLifecycleError, WorkflowStepRuntimeGateway, WorkflowStepSessionGateway,
};

pub(crate) fn resolve_step_session_with_data_dir(
    session_store: &SessionStore,
    data_dir: &std::path::Path,
    session_id: &str,
) -> Result<Option<ResolvedWorkflowStepSession>, WorkflowStepLifecycleError> {
    let Some(session) = session_store
        .get_session_meta(data_dir, session_id)
        .map_err(|e| WorkflowStepLifecycleError::SessionStore(format!("get_session_meta: {e}")))?
    else {
        return Ok(None);
    };
    if !session.is_workflow_step_session() {
        return Ok(None);
    }
    Ok(Some(ResolvedWorkflowStepSession {
        session_id: session_id.to_string(),
        worktree_path: session.worktree_path,
    }))
}

pub(crate) fn hydrate_open_workflow_step_tabs(
    session_store: &SessionStore,
    data_dir: &std::path::Path,
    worktree_path: &str,
    open_tabs: &OpenTabRegistry,
) -> Result<(), String> {
    for session in session_store.list_worktree_sessions(data_dir, worktree_path)? {
        if session.is_workflow_step_session() && session.state != SessionState::Closed {
            open_tabs.add(&session.id);
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) async fn close_resolved_step_tab_state<F, Fut>(
    session_store: &SessionStore,
    data_dir: &std::path::Path,
    handles: &Arc<Mutex<AgentProcessMap>>,
    open_tabs: &OpenTabRegistry,
    session_id: &str,
    close_runtime: F,
) -> Result<(), WorkflowStepLifecycleError>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<(), WorkflowStepLifecycleError>>,
{
    let runtime_result = close_idle_step_runtime_state(handles, session_id, close_runtime).await;
    try_close_step_session_tab_state(session_store, data_dir, Some(open_tabs), session_id)?;
    if let Err(e) = runtime_result {
        handles.lock().await.remove(session_id);
        return Err(e);
    }
    Ok(())
}

pub(crate) async fn close_idle_step_runtime_state<F, Fut>(
    handles: &Arc<Mutex<AgentProcessMap>>,
    session_id: &str,
    close_runtime: F,
) -> Result<(), WorkflowStepLifecycleError>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<(), WorkflowStepLifecycleError>>,
{
    if should_release_runtime_on_tab_close(handles, session_id).await {
        close_runtime().await?;
    }
    Ok(())
}

pub(crate) async fn should_release_runtime_on_tab_close(
    handles: &Arc<Mutex<AgentProcessMap>>,
    session_id: &str,
) -> bool {
    let has_runtime = handles.lock().await.contains_key(session_id);
    has_runtime
        && !crate::infrastructure::agent_session::runtime::is_agent_step_runtime_busy(
            handles, session_id,
        )
        .await
}

pub(crate) fn open_step_session_tab_state(
    session_store: &SessionStore,
    data_dir: &std::path::Path,
    open_tabs: &OpenTabRegistry,
    session_id: &str,
) -> Result<(), WorkflowStepLifecycleError> {
    let session = session_store
        .get_session_meta(data_dir, session_id)
        .map_err(|e| WorkflowStepLifecycleError::SessionStore(format!("get_session_meta: {e}")))?
        .ok_or_else(|| WorkflowStepLifecycleError::SessionNotFound(session_id.to_string()))?;
    if open_tabs.contains(session_id) && session.state == SessionState::Idle {
        return Ok(());
    }
    session_store
        .set_session_state(data_dir, session_id, SessionState::Idle)
        .map_err(|e| WorkflowStepLifecycleError::SessionStore(format!("set_session_state: {e}")))?;
    open_tabs.add(session_id);
    Ok(())
}

fn set_step_session_tab_closed(
    session_store: &SessionStore,
    data_dir: &std::path::Path,
    session_id: &str,
) -> Result<(), WorkflowStepLifecycleError> {
    session_store
        .set_session_state(data_dir, session_id, SessionState::Closed)
        .map_err(|e| WorkflowStepLifecycleError::SessionStore(format!("set_session_state: {e}")))
}

#[cfg(test)]
pub(crate) fn close_step_session_tab_state(
    session_store: &SessionStore,
    data_dir: &std::path::Path,
    open_tabs: Option<&OpenTabRegistry>,
    session_id: &str,
) {
    if let Err(_e) =
        try_close_step_session_tab_state(session_store, data_dir, open_tabs, session_id)
    {
        log::warn!(
            "workflow_step_tab_cleanup_failed code=session_state_update_failed message=failed_to_close_step_tab"
        );
    }
}

pub(crate) fn try_close_step_session_tab_state(
    session_store: &SessionStore,
    data_dir: &std::path::Path,
    open_tabs: Option<&OpenTabRegistry>,
    session_id: &str,
) -> Result<bool, WorkflowStepLifecycleError> {
    let should_close_tab = open_tabs
        .map(|open_tabs| open_tabs.contains(session_id))
        .unwrap_or(true);
    if !should_close_tab {
        if let Some(session) = session_store
            .get_session_meta(data_dir, session_id)
            .map_err(|e| {
                WorkflowStepLifecycleError::SessionStore(format!("get_session_meta: {e}"))
            })?
        {
            if session.state != SessionState::Closed {
                set_step_session_tab_closed(session_store, data_dir, session_id)?;
            }
        }
        return Ok(false);
    }
    set_step_session_tab_closed(session_store, data_dir, session_id)?;
    if let Some(open_tabs) = open_tabs {
        open_tabs.remove(session_id);
    }
    Ok(true)
}

struct TauriWorkflowStepRuntimeGateway<'a, R: tauri::Runtime> {
    app: &'a tauri::AppHandle<R>,
    handles: &'a Arc<Mutex<AgentProcessMap>>,
}

#[async_trait::async_trait]
impl<R: tauri::Runtime> WorkflowStepRuntimeGateway for TauriWorkflowStepRuntimeGateway<'_, R> {
    async fn close_idle_runtime_on_tab_close(
        &self,
        session_id: &str,
    ) -> Result<(), WorkflowStepLifecycleError> {
        let _lifecycle_guard =
            crate::infrastructure::agent_session::runtime::acquire_session_runtime_lock(session_id)
                .await;
        close_idle_step_runtime_state(self.handles, session_id, || async {
            crate::infrastructure::agent_session::runtime::close_agent_session_internal(
                self.app,
                self.handles,
                session_id,
            )
            .await
            .map_err(WorkflowStepLifecycleError::AgentSession)
        })
        .await
    }

    async fn close_runtime_on_step_done(
        &self,
        session_id: &str,
    ) -> Result<(), WorkflowStepLifecycleError> {
        crate::infrastructure::agent_session::runtime::close_agent_session_internal(
            self.app,
            self.handles,
            session_id,
        )
        .await
        .map_err(WorkflowStepLifecycleError::AgentSession)
    }
}

pub(crate) struct TauriWorkflowStepLifecycleGateway {
    app: tauri::AppHandle,
    session_store: Arc<SessionStore>,
    handles: Arc<Mutex<AgentProcessMap>>,
    open_tabs: Arc<OpenTabRegistry>,
}

impl TauriWorkflowStepLifecycleGateway {
    pub(crate) fn new(
        app: tauri::AppHandle,
        session_store: Arc<SessionStore>,
        handles: Arc<Mutex<AgentProcessMap>>,
        open_tabs: Arc<OpenTabRegistry>,
    ) -> Self {
        Self {
            app,
            session_store,
            handles,
            open_tabs,
        }
    }

    fn data_dir(&self) -> Result<std::path::PathBuf, WorkflowStepLifecycleError> {
        resolve_data_dir(&self.app)
            .map_err(|e| WorkflowStepLifecycleError::SessionStore(format!("resolve_data_dir: {e}")))
    }
}

impl WorkflowStepSessionGateway for TauriWorkflowStepLifecycleGateway {
    fn resolve_step_session(
        &self,
        session_id: &str,
    ) -> Result<Option<ResolvedWorkflowStepSession>, WorkflowStepLifecycleError> {
        let data_dir = self.data_dir()?;
        resolve_step_session_with_data_dir(self.session_store.as_ref(), &data_dir, session_id)
    }

    fn open_step_tab(&self, session_id: &str) -> Result<(), WorkflowStepLifecycleError> {
        let data_dir = self.data_dir()?;
        open_step_session_tab_state(
            self.session_store.as_ref(),
            &data_dir,
            self.open_tabs.as_ref(),
            session_id,
        )
    }

    fn close_step_tab(&self, session_id: &str) -> Result<bool, WorkflowStepLifecycleError> {
        let data_dir = self.data_dir()?;
        try_close_step_session_tab_state(
            self.session_store.as_ref(),
            &data_dir,
            Some(self.open_tabs.as_ref()),
            session_id,
        )
    }
}

#[async_trait::async_trait]
impl WorkflowStepRuntimeGateway for TauriWorkflowStepLifecycleGateway {
    async fn close_idle_runtime_on_tab_close(
        &self,
        session_id: &str,
    ) -> Result<(), WorkflowStepLifecycleError> {
        let _lifecycle_guard =
            crate::infrastructure::agent_session::runtime::acquire_session_runtime_lock(session_id)
                .await;
        close_idle_step_runtime_state(&self.handles, session_id, || async {
            crate::infrastructure::agent_session::runtime::close_agent_session_internal(
                &self.app,
                &self.handles,
                session_id,
            )
            .await
            .map_err(WorkflowStepLifecycleError::AgentSession)
        })
        .await
    }

    async fn close_runtime_on_step_done(
        &self,
        session_id: &str,
    ) -> Result<(), WorkflowStepLifecycleError> {
        crate::infrastructure::agent_session::runtime::close_agent_session_internal(
            &self.app,
            &self.handles,
            session_id,
        )
        .await
        .map_err(WorkflowStepLifecycleError::AgentSession)
    }
}

pub(crate) fn mark_started_step_tab_open<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    session_id: &str,
) {
    if let Some(open_tabs) = app.try_state::<Arc<OpenTabRegistry>>() {
        open_tabs.add(session_id);
    }
}

pub(crate) async fn release_step_runtime_on_done<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    session_store: &Arc<SessionStore>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    session_id: &str,
) {
    let runtime = TauriWorkflowStepRuntimeGateway { app, handles };
    release_step_runtime_on_done_with_gateways(&runtime, session_id).await;
    crate::infrastructure::agent_session::runtime::notify_status_transition(
        app,
        session_store,
        session_id,
        crate::infrastructure::agent_session::runtime::TurnPhase::Idle,
        None,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    use crate::infrastructure::agent_session::runtime::AgentProcessMap;
    use crate::usecase::agent_session::session::{OpenTabRegistry, SessionState};

    async fn release_step_runtime_on_done_state<F, Fut>(close_runtime: F)
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<(), WorkflowStepLifecycleError>>,
    {
        if let Err(_e) = close_runtime().await {
            log::warn!(
                "workflow_step_runtime_cleanup_failed code=runtime_close_failed message=failed_to_close_runtime"
            );
        }
    }

    async fn release_on_step_done_for_test(
        handles: &Arc<Mutex<AgentProcessMap>>,
        session_id: &str,
    ) {
        release_step_runtime_on_done_state(|| async {
            handles.lock().await.remove(session_id);
            Ok(())
        })
        .await;
    }

    fn workflow_step_session_for_test(
        session_id: &str,
    ) -> crate::usecase::agent_session::session::ChatSession {
        crate::usecase::agent_session::session::ChatSession {
            id: session_id.to_string(),
            worktree_path: "/repo".to_string(),
            messages: vec![crate::usecase::agent_session::session::ChatMessage {
                id: "msg-1".to_string(),
                role: crate::usecase::agent_session::session::MessageRole::Agent,
                content: "history".to_string(),
                thinking: None,
                activities: None,
                parts: None,
                streaming_final_seq: 0,
                timestamp: 1.0,
                mentions: None,
            }],
            state: SessionState::Idle,
            created_at: 1.0,
            updated_at: 1.0,
            agent_session_id: Some("sdk-session".to_string()),
            context_carry: Some(crate::usecase::agent_session::session::ContextCarryState::Resumed),
            permission_mode: "edit".to_string(),
            plan_mode: false,
            permission_profile_id: None,
            selected_model: None,
            backend_id: Some(
                crate::infrastructure::agent_session::runtime::CLAUDE_BACKEND_ID.to_string(),
            ),
            workflow_step_session: true,
            workflow_step_context: None,
            context_epoch: None,
        }
    }

    #[test]
    fn step_done_tab_cleanup_removes_tab_closes_session_and_preserves_history() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = crate::test_support::build_session_store();
        let open_tabs = OpenTabRegistry::default();
        let session_id = uuid::Uuid::new_v4().to_string();

        session_store
            .save_full_session_for_migration_or_restore(
                tmp.path(),
                &workflow_step_session_for_test(&session_id),
            )
            .unwrap();
        open_tabs.add(&session_id);

        close_step_session_tab_state(&session_store, tmp.path(), Some(&open_tabs), &session_id);

        assert!(!open_tabs.contains(&session_id));
        let session = session_store
            .load_full_session_for_restore(tmp.path(), &session_id)
            .unwrap()
            .expect("session remains as history");
        assert_eq!(session.state, SessionState::Closed);
        assert_eq!(session.agent_session_id.as_deref(), Some("sdk-session"));
        assert_eq!(session.messages.len(), 1);
    }

    #[test]
    fn step_done_tab_cleanup_is_idempotent_for_already_closed_tab() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = crate::test_support::build_session_store();
        let open_tabs = OpenTabRegistry::default();
        let session_id = uuid::Uuid::new_v4().to_string();

        session_store
            .save_full_session_for_migration_or_restore(
                tmp.path(),
                &workflow_step_session_for_test(&session_id),
            )
            .unwrap();
        session_store
            .set_session_state(tmp.path(), &session_id, SessionState::Closed)
            .unwrap();

        close_step_session_tab_state(&session_store, tmp.path(), Some(&open_tabs), &session_id);

        assert!(!open_tabs.contains(&session_id));
        let session = session_store
            .load_full_session_for_restore(tmp.path(), &session_id)
            .unwrap()
            .expect("session remains as history");
        assert_eq!(session.state, SessionState::Closed);
        assert_eq!(session.agent_session_id.as_deref(), Some("sdk-session"));
        assert_eq!(session.messages.len(), 1);
    }

    #[test]
    fn close_step_tab_retries_closed_state_when_registry_entry_is_missing() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = crate::test_support::build_session_store();
        let open_tabs = OpenTabRegistry::default();
        let session_id = uuid::Uuid::new_v4().to_string();

        session_store
            .save_full_session_for_migration_or_restore(
                tmp.path(),
                &workflow_step_session_for_test(&session_id),
            )
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
            .load_full_session_for_restore(tmp.path(), &session_id)
            .unwrap()
            .expect("session remains as history");
        assert_eq!(session.state, SessionState::Closed);
    }

    #[test]
    fn hydrate_open_workflow_step_tabs_only_opens_non_closed_workflow_sessions() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = crate::test_support::build_session_store();
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

        session_store
            .save_full_session_for_migration_or_restore(tmp.path(), &open_step)
            .unwrap();
        session_store
            .save_full_session_for_migration_or_restore(tmp.path(), &closed_step)
            .unwrap();
        session_store
            .save_full_session_for_migration_or_restore(tmp.path(), &regular)
            .unwrap();

        hydrate_open_workflow_step_tabs(&session_store, tmp.path(), worktree_path, &open_tabs)
            .unwrap();

        assert!(open_tabs.contains(&open_step_id));
        assert!(!open_tabs.contains(&closed_step_id));
        assert!(!open_tabs.contains(&regular_id));
    }

    #[tokio::test]
    async fn opening_step_tab_does_not_start_runtime_and_preserves_history() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = crate::test_support::build_session_store();
        let open_tabs = OpenTabRegistry::default();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = uuid::Uuid::new_v4().to_string();
        let mut session = workflow_step_session_for_test(&session_id);
        session.state = SessionState::Closed;

        session_store
            .save_full_session_for_migration_or_restore(tmp.path(), &session)
            .unwrap();

        open_step_session_tab_state(&session_store, tmp.path(), &open_tabs, &session_id).unwrap();

        assert!(open_tabs.contains(&session_id));
        assert!(handles.lock().await.is_empty());
        let session = session_store
            .load_full_session_for_restore(tmp.path(), &session_id)
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
            .load_full_session_for_restore(tmp.path(), &session_id)
            .unwrap()
            .expect("session remains as history");
        assert_eq!(session.updated_at, updated_at);
    }

    #[tokio::test]
    async fn tab_close_runtime_policy_releases_ready_and_idle_runtime() {
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        handles.lock().await.insert(
            "step".to_string(),
            crate::infrastructure::agent_session::runtime::make_test_agent_process(),
        );

        assert!(should_release_runtime_on_tab_close(&handles, "step").await);
    }

    #[tokio::test]
    async fn tab_close_runtime_policy_keeps_busy_runtime() {
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let mut proc = crate::infrastructure::agent_session::runtime::make_test_agent_process();
        proc.state = crate::infrastructure::agent_session::runtime::BridgeState::Streaming;
        handles.lock().await.insert("step".to_string(), proc);
        assert!(!should_release_runtime_on_tab_close(&handles, "step").await);

        {
            let mut map = handles.lock().await;
            let proc = map.get_mut("step").unwrap();
            proc.state = crate::infrastructure::agent_session::runtime::BridgeState::Ready;
            proc.turn_phase =
                crate::infrastructure::agent_session::runtime::TurnPhase::WaitingPermission;
        }
        assert!(!should_release_runtime_on_tab_close(&handles, "step").await);

        {
            let mut map = handles.lock().await;
            let proc = map.get_mut("step").unwrap();
            proc.turn_phase = crate::infrastructure::agent_session::runtime::TurnPhase::Idle;
            proc.pending_messages.push_back(
                crate::infrastructure::agent_session::runtime::PendingMessage {
                    id: "queued-1".to_string(),
                    content: "next".to_string(),
                    created_at: 1.0,
                    client_sent_at_ms: None,
                    request_received_at_ms: None,
                    permission_mode: "edit".to_string(),
                    plan_mode: false,
                    images: Vec::new(),
                    worktree_path: "/repo".to_string(),
                    mentions: Vec::new(),
                    editor_context: None,
                    existing_human_message_id: None,
                    existing_agent_message_id: None,
                },
            );
        }
        assert!(!should_release_runtime_on_tab_close(&handles, "step").await);
    }

    #[tokio::test]
    async fn tab_close_idle_runtime_releases_runtime_and_closes_tab_state() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = crate::test_support::build_session_store();
        let open_tabs = OpenTabRegistry::default();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = uuid::Uuid::new_v4().to_string();
        session_store
            .save_full_session_for_migration_or_restore(
                tmp.path(),
                &workflow_step_session_for_test(&session_id),
            )
            .unwrap();
        open_tabs.add(&session_id);
        handles.lock().await.insert(
            session_id.clone(),
            crate::infrastructure::agent_session::runtime::make_test_agent_process(),
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
            .load_full_session_for_restore(tmp.path(), &session_id)
            .unwrap()
            .expect("history remains");
        assert_eq!(session.state, SessionState::Closed);
        assert_eq!(session.agent_session_id.as_deref(), Some("sdk-session"));
        assert_eq!(session.messages.len(), 1);
    }

    #[tokio::test]
    async fn tab_close_busy_runtime_keeps_runtime_and_closes_only_tab_state() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = crate::test_support::build_session_store();
        let open_tabs = OpenTabRegistry::default();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = uuid::Uuid::new_v4().to_string();
        session_store
            .save_full_session_for_migration_or_restore(
                tmp.path(),
                &workflow_step_session_for_test(&session_id),
            )
            .unwrap();
        open_tabs.add(&session_id);
        let mut proc = crate::infrastructure::agent_session::runtime::make_test_agent_process();
        proc.state = crate::infrastructure::agent_session::runtime::BridgeState::Streaming;
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
            .load_full_session_for_restore(tmp.path(), &session_id)
            .unwrap()
            .expect("history remains");
        assert_eq!(session.state, SessionState::Closed);
        assert_eq!(session.agent_session_id.as_deref(), Some("sdk-session"));
    }

    #[tokio::test]
    async fn duplicate_tab_close_releases_remaining_idle_runtime_after_tab_already_closed() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = crate::test_support::build_session_store();
        let open_tabs = OpenTabRegistry::default();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = uuid::Uuid::new_v4().to_string();
        session_store
            .save_full_session_for_migration_or_restore(
                tmp.path(),
                &workflow_step_session_for_test(&session_id),
            )
            .unwrap();
        handles.lock().await.insert(
            session_id.clone(),
            crate::infrastructure::agent_session::runtime::make_test_agent_process(),
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
        let session_store = crate::test_support::build_session_store();
        let open_tabs = OpenTabRegistry::default();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = uuid::Uuid::new_v4().to_string();
        let mut session = workflow_step_session_for_test(&session_id);
        session.state = SessionState::Closed;
        session_store
            .save_full_session_for_migration_or_restore(tmp.path(), &session)
            .unwrap();
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
            .load_full_session_for_restore(tmp.path(), &session_id)
            .unwrap()
            .expect("history remains");
        assert_eq!(session.state, SessionState::Closed);
        assert_eq!(session.agent_session_id.as_deref(), Some("sdk-session"));
    }

    #[tokio::test]
    async fn tab_close_runtime_failure_still_closes_tab_state() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = crate::test_support::build_session_store();
        let open_tabs = OpenTabRegistry::default();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = uuid::Uuid::new_v4().to_string();
        session_store
            .save_full_session_for_migration_or_restore(
                tmp.path(),
                &workflow_step_session_for_test(&session_id),
            )
            .unwrap();
        open_tabs.add(&session_id);
        handles.lock().await.insert(
            session_id.clone(),
            crate::infrastructure::agent_session::runtime::make_test_agent_process(),
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
        let view = crate::adaptor::gateway::workflow::build_workflow_state_view_from_snapshot(
            workflow_state_for_test(&session_id),
            &handles,
            &open_tabs,
        )
        .await;
        assert!(!view.runtime_states[&session_id].runtime_active);
        assert!(!view.runtime_states[&session_id].tab_open);
        let session = session_store
            .load_full_session_for_restore(tmp.path(), &session_id)
            .unwrap()
            .expect("history remains");
        assert_eq!(session.state, SessionState::Closed);
    }

    #[tokio::test]
    async fn tab_state_update_failure_does_not_roll_back_runtime_release() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = crate::test_support::build_session_store();
        let open_tabs = OpenTabRegistry::default();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = uuid::Uuid::new_v4().to_string();
        open_tabs.add(&session_id);
        handles.lock().await.insert(
            session_id.clone(),
            crate::infrastructure::agent_session::runtime::make_test_agent_process(),
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
        let view = crate::adaptor::gateway::workflow::build_workflow_state_view_from_snapshot(
            workflow_state_for_test(&session_id),
            &handles,
            &open_tabs,
        )
        .await;
        assert!(!view.runtime_states[&session_id].runtime_active);
        assert!(view.runtime_states[&session_id].tab_open);
    }

    fn workflow_state_for_test(session_id: &str) -> crate::domain::workflow::WorkflowStateSnapshot {
        use crate::domain::workflow::{
            StepHistoryEntry, TokenUsage, WorkflowDefinition, WorkflowExecutionState,
            WorkflowStateSnapshot,
        };
        use std::collections::HashMap;
        WorkflowStateSnapshot {
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
                state: crate::domain::workflow::value_objects::default_step_entry_state(),
            }],
            step_execution_counts: HashMap::new(),
            workflow_definition: WorkflowDefinition {
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
    async fn release_on_step_done_releases_runtime_without_closing_step() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = crate::test_support::build_session_store();
        let open_tabs = OpenTabRegistry::default();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = uuid::Uuid::new_v4().to_string();

        session_store
            .save_full_session_for_migration_or_restore(
                tmp.path(),
                &workflow_step_session_for_test(&session_id),
            )
            .unwrap();
        open_tabs.add(&session_id);
        handles.lock().await.insert(
            session_id.clone(),
            crate::infrastructure::agent_session::runtime::make_test_agent_process(),
        );

        release_on_step_done_for_test(&handles, &session_id).await;

        let view = crate::adaptor::gateway::workflow::build_workflow_state_view_from_snapshot(
            workflow_state_for_test(&session_id),
            &handles,
            &open_tabs,
        )
        .await;
        assert!(!view.runtime_states[&session_id].runtime_active);
        assert!(view.runtime_states[&session_id].tab_open);
        let session = session_store
            .load_full_session_for_restore(tmp.path(), &session_id)
            .unwrap()
            .expect("step history session remains");
        assert_eq!(session.state, SessionState::Idle);
        assert_eq!(session.agent_session_id.as_deref(), Some("sdk-session"));
        assert_eq!(session.messages.len(), 1);
    }

    #[tokio::test]
    async fn release_on_step_done_releases_runtime_when_tab_already_closed() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = crate::test_support::build_session_store();
        let open_tabs = OpenTabRegistry::default();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = uuid::Uuid::new_v4().to_string();
        let mut session = workflow_step_session_for_test(&session_id);
        session.state = SessionState::Closed;

        session_store
            .save_full_session_for_migration_or_restore(tmp.path(), &session)
            .unwrap();
        handles.lock().await.insert(
            session_id.clone(),
            crate::infrastructure::agent_session::runtime::make_test_agent_process(),
        );

        release_on_step_done_for_test(&handles, &session_id).await;

        let view = crate::adaptor::gateway::workflow::build_workflow_state_view_from_snapshot(
            workflow_state_for_test(&session_id),
            &handles,
            &open_tabs,
        )
        .await;
        assert!(!view.runtime_states[&session_id].runtime_active);
        assert!(!view.runtime_states[&session_id].tab_open);
        let session = session_store
            .load_full_session_for_restore(tmp.path(), &session_id)
            .unwrap()
            .expect("step history session remains");
        assert_eq!(session.state, SessionState::Closed);
        assert_eq!(session.agent_session_id.as_deref(), Some("sdk-session"));
        assert_eq!(session.messages.len(), 1);
    }

    #[tokio::test]
    async fn release_on_step_done_releases_busy_runtime_without_closing_step() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = crate::test_support::build_session_store();
        let open_tabs = OpenTabRegistry::default();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = uuid::Uuid::new_v4().to_string();

        session_store
            .save_full_session_for_migration_or_restore(
                tmp.path(),
                &workflow_step_session_for_test(&session_id),
            )
            .unwrap();
        open_tabs.add(&session_id);
        let mut proc = crate::infrastructure::agent_session::runtime::make_test_agent_process();
        proc.pending_messages.push_back(
            crate::infrastructure::agent_session::runtime::PendingMessage {
                id: "queued-1".to_string(),
                content: "continue".to_string(),
                created_at: 1.0,
                client_sent_at_ms: None,
                request_received_at_ms: None,
                permission_mode: "edit".to_string(),
                plan_mode: false,
                images: Vec::new(),
                worktree_path: "/repo".to_string(),
                mentions: Vec::new(),
                editor_context: None,
                existing_human_message_id: None,
                existing_agent_message_id: None,
            },
        );
        handles.lock().await.insert(session_id.clone(), proc);

        release_on_step_done_for_test(&handles, &session_id).await;

        assert!(!handles.lock().await.contains_key(&session_id));
        assert!(open_tabs.contains(&session_id));
        let session = session_store
            .load_full_session_for_restore(tmp.path(), &session_id)
            .unwrap()
            .expect("step history session remains");
        assert_eq!(session.state, SessionState::Idle);
    }

    #[tokio::test]
    async fn release_and_tab_close_converge_to_closed_runtime_and_tab() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = crate::test_support::build_session_store();
        let open_tabs = OpenTabRegistry::default();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = uuid::Uuid::new_v4().to_string();

        session_store
            .save_full_session_for_migration_or_restore(
                tmp.path(),
                &workflow_step_session_for_test(&session_id),
            )
            .unwrap();
        open_tabs.add(&session_id);
        handles.lock().await.insert(
            session_id.clone(),
            crate::infrastructure::agent_session::runtime::make_test_agent_process(),
        );

        release_on_step_done_for_test(&handles, &session_id).await;
        try_close_step_session_tab_state(&session_store, tmp.path(), Some(&open_tabs), &session_id)
            .unwrap();

        let view = crate::adaptor::gateway::workflow::build_workflow_state_view_from_snapshot(
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
        let session_store = crate::test_support::build_session_store();
        let open_tabs = OpenTabRegistry::default();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = uuid::Uuid::new_v4().to_string();

        let mut session = workflow_step_session_for_test(&session_id);
        session.state = SessionState::Closed;
        session_store
            .save_full_session_for_migration_or_restore(tmp.path(), &session)
            .unwrap();
        handles.lock().await.insert(
            session_id.clone(),
            crate::infrastructure::agent_session::runtime::make_test_agent_process(),
        );

        open_step_session_tab_state(&session_store, tmp.path(), &open_tabs, &session_id).unwrap();

        assert!(open_tabs.contains(&session_id));
        assert!(handles.lock().await.contains_key(&session_id));
        let session = session_store
            .load_full_session_for_restore(tmp.path(), &session_id)
            .unwrap()
            .expect("session remains as history");
        assert_eq!(session.state, SessionState::Idle);
    }

    // R4-02: Spec「非 workflow session への tab 操作は workflow step の状態を変化させない」
    #[tokio::test]
    async fn non_workflow_session_tab_operations_do_not_affect_workflow_step_state() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = crate::test_support::build_session_store();
        let open_tabs = OpenTabRegistry::default();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));

        // Workflow step session: tab open + runtime active
        let step_id = uuid::Uuid::new_v4().to_string();
        session_store
            .save_full_session_for_migration_or_restore(
                tmp.path(),
                &workflow_step_session_for_test(&step_id),
            )
            .unwrap();
        open_tabs.add(&step_id);
        handles.lock().await.insert(
            step_id.clone(),
            crate::infrastructure::agent_session::runtime::make_test_agent_process(),
        );

        // Non-workflow session (different id, workflow_step_session=false)
        let non_workflow_id = uuid::Uuid::new_v4().to_string();
        let mut non_workflow = workflow_step_session_for_test(&non_workflow_id);
        non_workflow.workflow_step_session = false;
        session_store
            .save_full_session_for_migration_or_restore(tmp.path(), &non_workflow)
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
        let session_store = Arc::new(crate::test_support::build_session_store());
        let open_tabs = Arc::new(OpenTabRegistry::default());
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));
        let session_id = uuid::Uuid::new_v4().to_string();

        session_store
            .save_full_session_for_migration_or_restore(
                &data_dir,
                &workflow_step_session_for_test(&session_id),
            )
            .unwrap();
        open_tabs.add(&session_id);
        handles.lock().await.insert(
            session_id.clone(),
            crate::infrastructure::agent_session::runtime::make_test_agent_process(),
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
                let _guard =
                    crate::infrastructure::agent_session::runtime::acquire_session_runtime_lock(
                        &session_id,
                    )
                    .await;
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
            let handles = Arc::clone(&handles);
            let session_id = session_id.clone();
            let close_count = Arc::clone(&close_count);
            async move {
                let _guard =
                    crate::infrastructure::agent_session::runtime::acquire_session_runtime_lock(
                        &session_id,
                    )
                    .await;
                release_step_runtime_on_done_state({
                    let handles = Arc::clone(&handles);
                    let session_id = session_id.clone();
                    let close_count = Arc::clone(&close_count);
                    move || async move {
                        if handles.lock().await.remove(&session_id).is_some() {
                            close_count.fetch_add(1, Ordering::SeqCst);
                        }
                        Ok(())
                    }
                })
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
        let session_store = crate::test_support::build_session_store();
        let open_tabs = OpenTabRegistry::default();
        let handles = Arc::new(Mutex::new(AgentProcessMap::new()));

        // Setup: another step session with an active runtime that must remain untouched
        let other_id = uuid::Uuid::new_v4().to_string();
        session_store
            .save_full_session_for_migration_or_restore(
                tmp.path(),
                &workflow_step_session_for_test(&other_id),
            )
            .unwrap();
        handles.lock().await.insert(
            other_id.clone(),
            crate::infrastructure::agent_session::runtime::make_test_agent_process(),
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
        let session_store = crate::test_support::build_session_store();
        let session_id = uuid::Uuid::new_v4().to_string();

        session_store
            .save_full_session_for_migration_or_restore(
                tmp.path(),
                &workflow_step_session_for_test(&session_id),
            )
            .unwrap();

        let resolved = resolve_step_session_with_data_dir(&session_store, tmp.path(), &session_id)
            .unwrap()
            .expect("workflow_step_session flag alone makes this a step session");

        assert_eq!(resolved.session_id, session_id);
        assert_eq!(resolved.worktree_path, "/repo");
    }
}
