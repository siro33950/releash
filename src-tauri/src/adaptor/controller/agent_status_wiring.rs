use std::sync::Arc;

use crate::usecase::agent_session::session::SessionStore;
use crate::usecase::agent_session::status::{AgentStatusCenter, AgentStatusNotifier};

pub(crate) fn register_agent_status_listener(
    session_store: Arc<SessionStore>,
    center: Arc<AgentStatusCenter>,
    notifier: Arc<dyn AgentStatusNotifier>,
) {
    session_store.register_state_change_listener(Arc::new(
        move |session_id, _worktree_path, new_state| {
            let changes = center.on_session_state_changed(session_id, new_state.clone());
            notifier.status_changed(changes);
        },
    ));
}
