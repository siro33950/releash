use std::sync::Arc;

use super::{now_timestamp, ChatSession, RestoreSessionResponse, SessionState, SessionStore};

#[derive(Debug, Clone)]
pub struct RestoreSessionOutcome {
    pub response: RestoreSessionResponse,
}

pub struct SessionLifecycleController<'a> {
    pub session_store: &'a Arc<SessionStore>,
    pub data_dir: &'a std::path::Path,
}

impl<'a> SessionLifecycleController<'a> {
    pub fn close_session_state(&self, session_id: &str) -> Result<(), String> {
        self.session_store
            .set_session_state(self.data_dir, session_id, SessionState::Closed)
    }

    pub fn restore_session_state(
        &self,
        mut session: ChatSession,
    ) -> Result<RestoreSessionOutcome, String> {
        session.state = SessionState::Idle;
        session.updated_at = now_timestamp();
        self.session_store.save_session(self.data_dir, &session)?;
        Ok(RestoreSessionOutcome {
            response: RestoreSessionResponse {
                restored_workflow_step: false,
            },
        })
    }
}
