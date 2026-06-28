use std::sync::Arc;

use crate::usecase::agent_session::session::SessionStore;
use crate::usecase::agent_session::status::AgentStatusCenter;

pub(crate) fn register_agent_status_listener(
    app: tauri::AppHandle,
    broadcaster: Arc<crate::adaptor::gateway::shared::ws_broadcaster::WsBroadcaster>,
    session_store: Arc<SessionStore>,
    center: Arc<AgentStatusCenter>,
) {
    session_store.register_state_change_listener(Arc::new(
        move |session_id, _worktree_path, new_state| {
            let changes = center.on_session_state_changed(session_id, new_state.clone());
            crate::adaptor::presenter::agent_status::emit_agent_status_changes(
                &app,
                Some(&broadcaster),
                changes,
            );
        },
    ));
}
