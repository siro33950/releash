use std::sync::Arc;

use super::{ChatSession, RestoreSessionResponse, SessionState, SessionStore};

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
        session: ChatSession,
    ) -> Result<RestoreSessionResponse, String> {
        self.session_store
            .set_session_state(self.data_dir, &session.id, SessionState::Idle)?;
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
        let store = Arc::new(crate::test_support::build_session_store());
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
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.state, SessionState::Closed);
    }

    #[test]
    fn restore_session_state_does_not_mark_context_carry_before_runtime_start() {
        let temp = tempfile::tempdir().unwrap();
        let store = Arc::new(crate::test_support::build_session_store());
        let session = super::super::create_session_internal(
            &store,
            temp.path(),
            "/repo",
            Some("claude".to_string()),
        )
        .unwrap();
        super::super::add_message_internal(
            &store,
            temp.path(),
            &session.id,
            super::super::MessageRole::Human,
            "remember alpha",
            None,
            None,
        )
        .unwrap();
        let session = store
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();

        let controller = SessionLifecycleController {
            session_store: &store,
            data_dir: temp.path(),
        };
        let session_id = session.id.clone();
        controller.restore_session_state(session).unwrap();

        let loaded = store
            .load_full_session_for_restore(temp.path(), &session_id)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.context_carry, None);
        assert_eq!(loaded.messages.len(), 1);
        assert_eq!(loaded.messages[0].content, "remember alpha");
    }

    #[test]
    fn restore_session_state_preserves_existing_context_carry() {
        let temp = tempfile::tempdir().unwrap();
        let store = Arc::new(crate::test_support::build_session_store());
        let mut session = super::super::create_session_internal(
            &store,
            temp.path(),
            "/repo",
            Some("claude".to_string()),
        )
        .unwrap();
        session.agent_session_id = Some("sdk-session".to_string());
        session.context_carry = Some(super::super::ContextCarryState::Resumed);
        store
            .save_full_session_for_migration_or_restore(temp.path(), &session)
            .unwrap();

        let controller = SessionLifecycleController {
            session_store: &store,
            data_dir: temp.path(),
        };
        let session_id = session.id.clone();
        controller.restore_session_state(session).unwrap();

        let loaded = store
            .load_full_session_for_restore(temp.path(), &session_id)
            .unwrap()
            .unwrap();
        assert_eq!(
            loaded.context_carry,
            Some(super::super::ContextCarryState::Resumed)
        );
    }
}
