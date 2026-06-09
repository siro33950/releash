use std::future::Future;
use std::sync::Arc;

use tauri::Manager;
use tokio::sync::Mutex;

use crate::app_data_dir::resolve_data_dir;
use crate::infrastructure::agent_session::runtime::AgentProcessMap;
use crate::usecase::agent_session::session::{
    now_timestamp, OpenTabRegistry, SessionState, SessionStore,
};
use crate::workflow_step_lifecycle::{
    release_step_runtime_on_done_with_gateways, ResolvedWorkflowStepSession, WorkflowStepLifecycle,
    WorkflowStepLifecycleError, WorkflowStepRuntimeGateway, WorkflowStepSessionGateway,
};

pub(crate) fn resolve_step_session_with_data_dir(
    session_store: &SessionStore,
    data_dir: &std::path::Path,
    session_id: &str,
) -> Result<Option<ResolvedWorkflowStepSession>, WorkflowStepLifecycleError> {
    let Some(session) = session_store
        .get_session(data_dir, session_id)
        .map_err(|e| WorkflowStepLifecycleError::SessionStore(format!("get_session: {e}")))?
    else {
        return Ok(None);
    };
    if !session.workflow_step_session {
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
        if session.workflow_step_session && session.state != SessionState::Closed {
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
    let mut session = session_store
        .get_session(data_dir, session_id)
        .map_err(|e| WorkflowStepLifecycleError::SessionStore(format!("get_session: {e}")))?
        .ok_or_else(|| WorkflowStepLifecycleError::SessionNotFound(session_id.to_string()))?;
    if open_tabs.contains(session_id) && session.state == SessionState::Idle {
        return Ok(());
    }
    session.state = SessionState::Idle;
    session.updated_at = now_timestamp();
    session_store
        .save_session(data_dir, &session)
        .map_err(|e| WorkflowStepLifecycleError::SessionStore(format!("save_session: {e}")))?;
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
            .get_session(data_dir, session_id)
            .map_err(|e| WorkflowStepLifecycleError::SessionStore(format!("get_session: {e}")))?
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

struct TauriWorkflowStepSessionGateway<'a, R: tauri::Runtime> {
    app: &'a tauri::AppHandle<R>,
    session_store: &'a SessionStore,
    open_tabs: &'a OpenTabRegistry,
}

impl<R: tauri::Runtime> WorkflowStepSessionGateway for TauriWorkflowStepSessionGateway<'_, R> {
    fn resolve_step_session(
        &self,
        session_id: &str,
    ) -> Result<Option<ResolvedWorkflowStepSession>, WorkflowStepLifecycleError> {
        let data_dir = resolve_data_dir(self.app).map_err(|e| {
            WorkflowStepLifecycleError::SessionStore(format!("resolve_data_dir: {e}"))
        })?;
        resolve_step_session_with_data_dir(self.session_store, &data_dir, session_id)
    }

    fn open_step_tab(&self, session_id: &str) -> Result<(), WorkflowStepLifecycleError> {
        let data_dir = resolve_data_dir(self.app).map_err(|e| {
            WorkflowStepLifecycleError::SessionStore(format!("resolve_data_dir: {e}"))
        })?;
        open_step_session_tab_state(self.session_store, &data_dir, self.open_tabs, session_id)
    }

    fn close_step_tab(&self, session_id: &str) -> Result<bool, WorkflowStepLifecycleError> {
        let data_dir = resolve_data_dir(self.app).map_err(|e| {
            WorkflowStepLifecycleError::SessionStore(format!("resolve_data_dir: {e}"))
        })?;
        try_close_step_session_tab_state(
            self.session_store,
            &data_dir,
            Some(self.open_tabs),
            session_id,
        )
    }
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

pub(crate) struct TauriWorkflowStepLifecycle<'a, R: tauri::Runtime> {
    sessions: TauriWorkflowStepSessionGateway<'a, R>,
    runtime: TauriWorkflowStepRuntimeGateway<'a, R>,
}

impl<'a, R: tauri::Runtime> TauriWorkflowStepLifecycle<'a, R> {
    pub(crate) fn new(
        app: &'a tauri::AppHandle<R>,
        session_store: &'a SessionStore,
        handles: &'a Arc<Mutex<AgentProcessMap>>,
        open_tabs: &'a OpenTabRegistry,
    ) -> Self {
        Self {
            sessions: TauriWorkflowStepSessionGateway {
                app,
                session_store,
                open_tabs,
            },
            runtime: TauriWorkflowStepRuntimeGateway { app, handles },
        }
    }

    fn usecase(&self) -> WorkflowStepLifecycle<'_> {
        WorkflowStepLifecycle {
            sessions: &self.sessions,
            runtime: &self.runtime,
        }
    }

    pub(crate) async fn close_tab_target(
        &self,
        session_id: &str,
    ) -> Result<Option<ResolvedWorkflowStepSession>, WorkflowStepLifecycleError> {
        self.usecase().close_tab_target(session_id).await
    }

    pub(crate) async fn try_open_tab(
        &self,
        session_id: &str,
    ) -> Result<Option<ResolvedWorkflowStepSession>, WorkflowStepLifecycleError> {
        self.usecase().try_open_tab(session_id).await
    }

    pub(crate) async fn open_tab(
        &self,
        session_id: &str,
    ) -> Result<ResolvedWorkflowStepSession, WorkflowStepLifecycleError> {
        self.usecase().open_tab(session_id).await
    }
}

pub(crate) struct StoredWorkflowStepSessionGateway<'a> {
    pub(crate) session_store: &'a SessionStore,
    pub(crate) data_dir: &'a std::path::Path,
    pub(crate) open_tabs: Option<&'a OpenTabRegistry>,
}

impl WorkflowStepSessionGateway for StoredWorkflowStepSessionGateway<'_> {
    fn resolve_step_session(
        &self,
        session_id: &str,
    ) -> Result<Option<ResolvedWorkflowStepSession>, WorkflowStepLifecycleError> {
        resolve_step_session_with_data_dir(self.session_store, self.data_dir, session_id)
    }

    fn open_step_tab(&self, session_id: &str) -> Result<(), WorkflowStepLifecycleError> {
        let Some(open_tabs) = self.open_tabs else {
            return Err(WorkflowStepLifecycleError::SessionStore(
                "open tab registry unavailable".to_string(),
            ));
        };
        open_step_session_tab_state(self.session_store, self.data_dir, open_tabs, session_id)
    }

    fn close_step_tab(&self, session_id: &str) -> Result<bool, WorkflowStepLifecycleError> {
        try_close_step_session_tab_state(
            self.session_store,
            self.data_dir,
            self.open_tabs,
            session_id,
        )
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
    open_tabs: Option<&OpenTabRegistry>,
    session_id: &str,
) {
    if let Ok(data_dir) = resolve_data_dir(app) {
        let sessions = StoredWorkflowStepSessionGateway {
            session_store,
            data_dir: &data_dir,
            open_tabs,
        };
        let runtime = TauriWorkflowStepRuntimeGateway { app, handles };
        release_step_runtime_on_done_with_gateways(&sessions, &runtime, session_id).await;
    }
    crate::infrastructure::agent_session::runtime::notify_status_transition(
        app,
        session_store,
        session_id,
        crate::infrastructure::agent_session::runtime::TurnPhase::Idle,
        Some(SessionState::Closed),
    );
}
