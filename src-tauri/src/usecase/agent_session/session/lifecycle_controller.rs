use std::sync::Arc;

use super::{now_timestamp, ChatSession, RestoreSessionResponse, SessionState, SessionStore};

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
    ) -> Result<RestoreSessionResponse, String> {
        session.state = SessionState::Idle;
        session.updated_at = now_timestamp();
        self.session_store.save_session(self.data_dir, &session)?;
        Ok(RestoreSessionResponse {
            restored_workflow_step: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_session_state_marks_session_closed() {
        let temp = tempfile::tempdir().unwrap();
        let store = Arc::new(SessionStore::default());
        let session = super::super::create_session_internal(
            &store,
            temp.path(),
            "/repo",
            Some("claude".to_string()),
        )
        .unwrap();

        let controller = SessionLifecycleController {
            session_store: &store,
            data_dir: temp.path(),
        };
        controller.close_session_state(&session.id).unwrap();

        let loaded = store
            .get_session(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.state, SessionState::Closed);
    }
}
