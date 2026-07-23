use std::sync::Arc;

use crate::infrastructure::platform::app_data_dir::resolve_data_dir;
use crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase;
use crate::usecase::agent_session::session::{OpenTabRegistry, SessionStore};
use crate::usecase::workflow::node_lifecycle::{
    release_node_runtime_on_done_with_gateways, NodeExecutionLifecycleError,
    NodeExecutionRuntimeGateway, ResolvedWorkflowNodeSession, WorkflowNodeSessionGateway,
};

pub(crate) fn resolve_node_session_with_data_dir(
    session_store: &SessionStore,
    data_dir: &std::path::Path,
    session_id: &str,
) -> Result<Option<ResolvedWorkflowNodeSession>, NodeExecutionLifecycleError> {
    let Some(session) = session_store
        .get_session_meta(data_dir, session_id)
        .map_err(|e| NodeExecutionLifecycleError::SessionStore(format!("get_session_meta: {e}")))?
    else {
        return Ok(None);
    };
    if !session.is_workflow_node_session() {
        return Ok(None);
    }
    Ok(Some(ResolvedWorkflowNodeSession {
        session_id: session_id.to_string(),
        worktree_path: session.worktree_path,
    }))
}

pub(crate) fn open_node_session_tab_state(open_tabs: &OpenTabRegistry, session_id: &str) {
    open_tabs.add(session_id);
}

#[cfg(test)]
pub(crate) fn close_node_session_tab_state(open_tabs: Option<&OpenTabRegistry>, session_id: &str) {
    try_close_node_session_tab_state(open_tabs, session_id);
}

pub(crate) fn try_close_node_session_tab_state(
    open_tabs: Option<&OpenTabRegistry>,
    session_id: &str,
) -> bool {
    open_tabs.is_some_and(|open_tabs| open_tabs.remove(session_id))
}

struct TauriNodeExecutionRuntimeGateway<'a> {
    runtime: &'a AgentSessionRuntimeUsecase,
}

#[async_trait::async_trait]
impl NodeExecutionRuntimeGateway for TauriNodeExecutionRuntimeGateway<'_> {
    async fn close_runtime_on_node_done(
        &self,
        session_id: &str,
    ) -> Result<(), NodeExecutionLifecycleError> {
        self.runtime
            .force_close_session(session_id)
            .await
            .map_err(|e| NodeExecutionLifecycleError::AgentSession(e.to_string()))
    }
}

pub(crate) struct TauriNodeExecutionLifecycleGateway {
    app: tauri::AppHandle,
    session_store: Arc<SessionStore>,
    runtime: Arc<AgentSessionRuntimeUsecase>,
    open_tabs: Arc<OpenTabRegistry>,
}

impl TauriNodeExecutionLifecycleGateway {
    pub(crate) fn new(
        app: tauri::AppHandle,
        session_store: Arc<SessionStore>,
        runtime: Arc<AgentSessionRuntimeUsecase>,
        open_tabs: Arc<OpenTabRegistry>,
    ) -> Self {
        Self {
            app,
            session_store,
            runtime,
            open_tabs,
        }
    }

    fn data_dir(&self) -> Result<std::path::PathBuf, NodeExecutionLifecycleError> {
        resolve_data_dir(&self.app).map_err(|e| {
            NodeExecutionLifecycleError::SessionStore(format!("resolve_data_dir: {e}"))
        })
    }
}

impl WorkflowNodeSessionGateway for TauriNodeExecutionLifecycleGateway {
    fn resolve_node_session(
        &self,
        session_id: &str,
    ) -> Result<Option<ResolvedWorkflowNodeSession>, NodeExecutionLifecycleError> {
        let data_dir = self.data_dir()?;
        resolve_node_session_with_data_dir(self.session_store.as_ref(), &data_dir, session_id)
    }

    fn open_node_tab(&self, session_id: &str) -> Result<(), NodeExecutionLifecycleError> {
        open_node_session_tab_state(self.open_tabs.as_ref(), session_id);
        Ok(())
    }

    fn close_node_tab(&self, session_id: &str) -> Result<bool, NodeExecutionLifecycleError> {
        Ok(try_close_node_session_tab_state(
            Some(self.open_tabs.as_ref()),
            session_id,
        ))
    }
}

#[async_trait::async_trait]
impl NodeExecutionRuntimeGateway for TauriNodeExecutionLifecycleGateway {
    async fn close_runtime_on_node_done(
        &self,
        session_id: &str,
    ) -> Result<(), NodeExecutionLifecycleError> {
        self.runtime
            .close_session(session_id)
            .await
            .map_err(|e| NodeExecutionLifecycleError::AgentSession(e.to_string()))
    }
}

pub(crate) fn mark_started_node_tab_open(open_tabs: &OpenTabRegistry, session_id: &str) {
    open_tabs.add(session_id);
}

pub(crate) async fn release_node_runtime_on_done(
    runtime: &Arc<AgentSessionRuntimeUsecase>,
    session_id: &str,
) {
    let runtime_gateway = TauriNodeExecutionRuntimeGateway {
        runtime: runtime.as_ref(),
    };
    release_node_runtime_on_done_with_gateways(&runtime_gateway, session_id).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::usecase::agent_session::session::{OpenTabRegistry, SessionState};
    use crate::usecase::agent_session::status::TurnPhase;

    async fn release_node_runtime_on_done_state<F, Fut>(close_runtime: F)
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<(), NodeExecutionLifecycleError>>,
    {
        if let Err(_e) = close_runtime().await {
            log::warn!(
                "workflow_node_runtime_cleanup_failed code=runtime_close_failed message=failed_to_close_runtime"
            );
        }
    }

    fn runtime_for_test() -> Arc<AgentSessionRuntimeUsecase> {
        let data_dir =
            std::env::temp_dir().join(format!("releash-node-lifecycle-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&data_dir).unwrap();
        crate::test_support::build_agent_runtime_usecase(
            Arc::new(crate::test_support::build_session_store()),
            data_dir,
        )
    }

    async fn insert_runtime(
        runtime: &AgentSessionRuntimeUsecase,
        session_id: &str,
        phase: TurnPhase,
        queued: bool,
    ) {
        runtime
            .insert_runtime_state_for_test(session_id, phase, queued)
            .await;
    }

    async fn release_on_node_done_for_test(
        runtime: &Arc<AgentSessionRuntimeUsecase>,
        session_id: &str,
    ) {
        release_node_runtime_on_done_state(|| async {
            runtime
                .force_close_session(session_id)
                .await
                .map_err(|error| NodeExecutionLifecycleError::AgentSession(error.to_string()))?;
            Ok(())
        })
        .await;
    }

    fn workflow_node_session_for_test(
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
            error_reason: None,
            created_at: 1.0,
            updated_at: 1.0,
            agent_session_id: Some("sdk-session".to_string()),
            context_carry: Some(crate::usecase::agent_session::session::ContextCarryState::Resumed),
            permission_mode: "edit".to_string(),
            plan_mode: false,
            permission_profile_id: None,
            selected_model: None,
            backend_id: Some(
                crate::infrastructure::agent_session::claude::CLAUDE_BACKEND_ID.to_string(),
            ),
            workflow_node_session: true,
            workflow_node_context: None,
            context_epoch: None,
        }
    }

    #[test]
    fn workflow_node_tab_close_is_view_only_and_preserves_session_projection() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = crate::test_support::build_session_store();
        let open_tabs = OpenTabRegistry::default();
        let session_id = uuid::Uuid::new_v4().to_string();

        session_store
            .save_full_session_for_migration_or_restore(
                tmp.path(),
                &workflow_node_session_for_test(&session_id),
            )
            .unwrap();
        open_tabs.add(&session_id);

        close_node_session_tab_state(Some(&open_tabs), &session_id);

        assert!(!open_tabs.contains(&session_id));
        let session = session_store
            .load_full_session_for_restore(tmp.path(), &session_id)
            .unwrap()
            .expect("session remains as history");
        assert_eq!(session.state, SessionState::Idle);
        assert_eq!(session.agent_session_id.as_deref(), Some("sdk-session"));
        assert_eq!(session.messages.len(), 1);
    }

    #[test]
    fn node_done_tab_cleanup_is_idempotent_for_already_closed_tab() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = crate::test_support::build_session_store();
        let open_tabs = OpenTabRegistry::default();
        let session_id = uuid::Uuid::new_v4().to_string();

        session_store
            .save_full_session_for_migration_or_restore(
                tmp.path(),
                &workflow_node_session_for_test(&session_id),
            )
            .unwrap();
        session_store
            .set_session_state(tmp.path(), &session_id, SessionState::Closed)
            .unwrap();

        close_node_session_tab_state(Some(&open_tabs), &session_id);

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
    fn close_node_tab_with_missing_registry_entry_does_not_mutate_projection() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = crate::test_support::build_session_store();
        let open_tabs = OpenTabRegistry::default();
        let session_id = uuid::Uuid::new_v4().to_string();

        session_store
            .save_full_session_for_migration_or_restore(
                tmp.path(),
                &workflow_node_session_for_test(&session_id),
            )
            .unwrap();

        let changed = try_close_node_session_tab_state(Some(&open_tabs), &session_id);

        assert!(!changed);
        assert!(!open_tabs.contains(&session_id));
        let session = session_store
            .load_full_session_for_restore(tmp.path(), &session_id)
            .unwrap()
            .expect("session remains as history");
        assert_eq!(session.state, SessionState::Idle);
    }

    #[tokio::test]
    async fn opening_node_tab_does_not_start_runtime_and_preserves_history() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = crate::test_support::build_session_store();
        let open_tabs = OpenTabRegistry::default();
        let handles = runtime_for_test();
        let session_id = uuid::Uuid::new_v4().to_string();
        let mut session = workflow_node_session_for_test(&session_id);
        session.state = SessionState::Closed;

        session_store
            .save_full_session_for_migration_or_restore(tmp.path(), &session)
            .unwrap();

        open_node_session_tab_state(&open_tabs, &session_id);

        assert!(open_tabs.contains(&session_id));
        assert!(!handles.has_live_runtime(&session_id).await);
        let session = session_store
            .load_full_session_for_restore(tmp.path(), &session_id)
            .unwrap()
            .expect("session remains as history");
        assert_eq!(session.state, SessionState::Closed);
        assert_eq!(session.agent_session_id.as_deref(), Some("sdk-session"));
        assert_eq!(session.messages.len(), 1);
        let updated_at = session.updated_at;

        open_node_session_tab_state(&open_tabs, &session_id);
        assert_eq!(open_tabs.snapshot().len(), 1);
        assert!(!handles.has_live_runtime(&session_id).await);
        let session = session_store
            .load_full_session_for_restore(tmp.path(), &session_id)
            .unwrap()
            .expect("session remains as history");
        assert_eq!(session.updated_at, updated_at);
    }

    #[tokio::test]
    async fn release_on_node_done_releases_runtime_without_closing_node() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = crate::test_support::build_session_store();
        let open_tabs = OpenTabRegistry::default();
        let handles = runtime_for_test();
        let session_id = uuid::Uuid::new_v4().to_string();

        session_store
            .save_full_session_for_migration_or_restore(
                tmp.path(),
                &workflow_node_session_for_test(&session_id),
            )
            .unwrap();
        open_tabs.add(&session_id);
        insert_runtime(&handles, &session_id, TurnPhase::Idle, false).await;

        release_on_node_done_for_test(&handles, &session_id).await;

        assert!(!handles.has_live_runtime(&session_id).await);
        assert!(open_tabs.contains(&session_id));
        let session = session_store
            .load_full_session_for_restore(tmp.path(), &session_id)
            .unwrap()
            .expect("node history session remains");
        assert_eq!(session.state, SessionState::Idle);
        assert_eq!(session.agent_session_id.as_deref(), Some("sdk-session"));
        assert_eq!(session.messages.len(), 1);
    }

    #[tokio::test]
    async fn release_on_node_done_releases_runtime_when_tab_already_closed() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = crate::test_support::build_session_store();
        let open_tabs = OpenTabRegistry::default();
        let handles = runtime_for_test();
        let session_id = uuid::Uuid::new_v4().to_string();
        let mut session = workflow_node_session_for_test(&session_id);
        session.state = SessionState::Closed;

        session_store
            .save_full_session_for_migration_or_restore(tmp.path(), &session)
            .unwrap();
        insert_runtime(&handles, &session_id, TurnPhase::Idle, false).await;

        release_on_node_done_for_test(&handles, &session_id).await;

        assert!(!handles.has_live_runtime(&session_id).await);
        assert!(!open_tabs.contains(&session_id));
        let session = session_store
            .load_full_session_for_restore(tmp.path(), &session_id)
            .unwrap()
            .expect("node history session remains");
        assert_eq!(session.state, SessionState::Closed);
        assert_eq!(session.agent_session_id.as_deref(), Some("sdk-session"));
        assert_eq!(session.messages.len(), 1);
    }

    #[tokio::test]
    async fn release_on_node_done_releases_busy_runtime_without_closing_node() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = crate::test_support::build_session_store();
        let open_tabs = OpenTabRegistry::default();
        let handles = runtime_for_test();
        let session_id = uuid::Uuid::new_v4().to_string();

        session_store
            .save_full_session_for_migration_or_restore(
                tmp.path(),
                &workflow_node_session_for_test(&session_id),
            )
            .unwrap();
        open_tabs.add(&session_id);
        insert_runtime(&handles, &session_id, TurnPhase::Idle, true).await;

        release_on_node_done_for_test(&handles, &session_id).await;

        assert!(!handles.has_live_runtime(&session_id).await);
        assert!(open_tabs.contains(&session_id));
        let session = session_store
            .load_full_session_for_restore(tmp.path(), &session_id)
            .unwrap()
            .expect("node history session remains");
        assert_eq!(session.state, SessionState::Idle);
    }

    #[tokio::test]
    async fn release_and_tab_close_converge_to_closed_runtime_and_tab() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = crate::test_support::build_session_store();
        let open_tabs = OpenTabRegistry::default();
        let handles = runtime_for_test();
        let session_id = uuid::Uuid::new_v4().to_string();

        session_store
            .save_full_session_for_migration_or_restore(
                tmp.path(),
                &workflow_node_session_for_test(&session_id),
            )
            .unwrap();
        open_tabs.add(&session_id);
        insert_runtime(&handles, &session_id, TurnPhase::Idle, false).await;

        release_on_node_done_for_test(&handles, &session_id).await;
        try_close_node_session_tab_state(Some(&open_tabs), &session_id);

        assert!(!handles.has_live_runtime(&session_id).await);
        assert!(!open_tabs.contains(&session_id));
    }

    // R4-01: Spec「runtime 起動中の node を再オープンしても runtime 状態は変化しない」
    #[tokio::test]
    async fn reopening_node_tab_with_active_runtime_keeps_runtime_active() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = crate::test_support::build_session_store();
        let open_tabs = OpenTabRegistry::default();
        let handles = runtime_for_test();
        let session_id = uuid::Uuid::new_v4().to_string();

        let mut session = workflow_node_session_for_test(&session_id);
        session.state = SessionState::Closed;
        session_store
            .save_full_session_for_migration_or_restore(tmp.path(), &session)
            .unwrap();
        insert_runtime(&handles, &session_id, TurnPhase::Idle, false).await;

        open_node_session_tab_state(&open_tabs, &session_id);

        assert!(open_tabs.contains(&session_id));
        assert!(handles.has_live_runtime(&session_id).await);
        let session = session_store
            .load_full_session_for_restore(tmp.path(), &session_id)
            .unwrap()
            .expect("session remains as history");
        assert_eq!(session.state, SessionState::Closed);
    }

    // R4-02: Spec「非 workflow session への tab 操作は workflow node の状態を変化させない」
    #[tokio::test]
    async fn non_workflow_session_tab_operations_do_not_affect_workflow_node_state() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = crate::test_support::build_session_store();
        let open_tabs = OpenTabRegistry::default();
        let handles = runtime_for_test();

        // Workflow node session: tab open + runtime active
        let node_id = uuid::Uuid::new_v4().to_string();
        session_store
            .save_full_session_for_migration_or_restore(
                tmp.path(),
                &workflow_node_session_for_test(&node_id),
            )
            .unwrap();
        open_tabs.add(&node_id);
        insert_runtime(&handles, &node_id, TurnPhase::Idle, false).await;

        // Non-workflow session (different id, workflow_node_session=false)
        let non_workflow_id = uuid::Uuid::new_v4().to_string();
        let mut non_workflow = workflow_node_session_for_test(&non_workflow_id);
        non_workflow.workflow_node_session = false;
        session_store
            .save_full_session_for_migration_or_restore(tmp.path(), &non_workflow)
            .unwrap();

        // Resolver returns None for non-workflow session → tab operations would not proceed
        let resolved =
            resolve_node_session_with_data_dir(&session_store, tmp.path(), &non_workflow_id)
                .unwrap();
        assert!(resolved.is_none());

        // Workflow node state is unchanged
        assert!(open_tabs.contains(&node_id));
        assert!(handles.has_live_runtime(&node_id).await);
    }

    // B-052 / R-014: projection が無くても view-local open は durable storage や runtime に触れない。
    #[tokio::test]
    async fn tab_open_without_session_projection_is_view_only() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = crate::test_support::build_session_store();
        let open_tabs = OpenTabRegistry::default();
        let handles = runtime_for_test();

        // Setup: another node session with an active runtime that must remain untouched
        let other_id = uuid::Uuid::new_v4().to_string();
        session_store
            .save_full_session_for_migration_or_restore(
                tmp.path(),
                &workflow_node_session_for_test(&other_id),
            )
            .unwrap();
        insert_runtime(&handles, &other_id, TurnPhase::Idle, false).await;

        // The view registry deliberately does not consult the durable session projection.
        let missing_id = uuid::Uuid::new_v4().to_string();
        open_node_session_tab_state(&open_tabs, &missing_id);

        // Runtime state for unrelated session is preserved
        assert!(handles.has_live_runtime(&other_id).await);
        assert!(open_tabs.contains(&missing_id));
        assert!(!open_tabs.contains(&other_id));
    }

    #[tokio::test]
    async fn resolver_accepts_workflow_node_session_flag_without_embedded_execution_snapshot() {
        let tmp = tempfile::TempDir::new().unwrap();
        let session_store = crate::test_support::build_session_store();
        let session_id = uuid::Uuid::new_v4().to_string();

        session_store
            .save_full_session_for_migration_or_restore(
                tmp.path(),
                &workflow_node_session_for_test(&session_id),
            )
            .unwrap();

        let resolved = resolve_node_session_with_data_dir(&session_store, tmp.path(), &session_id)
            .unwrap()
            .expect("workflow_node_session flag alone makes this a node session");

        assert_eq!(resolved.session_id, session_id);
        assert_eq!(resolved.worktree_path, "/repo");
    }
}
