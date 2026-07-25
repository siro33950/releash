use std::sync::Arc;

#[cfg(test)]
use crate::usecase::agent_session::event_log::AgentSessionEvent;
use crate::usecase::agent_session::event_log::TurnEventLog;

#[cfg(test)]
use super::now_timestamp;
use super::{ChatSession, SessionState, SessionStore};

pub struct SessionLifecycleController<'a> {
    pub session_store: &'a Arc<SessionStore>,
    pub data_dir: &'a std::path::Path,
}

impl<'a> SessionLifecycleController<'a> {
    #[cfg(test)]
    pub fn close_session_state(&self, session_id: &str) -> Result<(), String> {
        self.session_store
            .append_session_event_and_project_state(
                self.data_dir,
                session_id,
                AgentSessionEvent::SessionClosed {
                    at: now_timestamp(),
                },
            )
            .map(|_| ())
    }

    pub fn restore_session_state(&self, session: ChatSession) -> Result<(), String> {
        let projected_state = self.project_session_state(&session.id)?;
        if projected_state != session.state {
            self.session_store.set_session_state_from_user(
                self.data_dir,
                &session.id,
                projected_state,
            )?;
        }
        self.session_store.set_session_state_from_user(
            self.data_dir,
            &session.id,
            SessionState::Idle,
        )?;
        Ok(())
    }

    fn project_session_state(&self, session_id: &str) -> Result<SessionState, String> {
        let events = self
            .session_store
            .load_session_events(self.data_dir, session_id)?;
        Ok(TurnEventLog::from_events(events)
            .project()
            .status
            .session_state)
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

        let events = store.load_session_events(temp.path(), &session.id).unwrap();
        assert!(events
            .iter()
            .any(|event| matches!(event, AgentSessionEvent::SessionClosed { .. })));
        let projected_state = TurnEventLog::from_events(events)
            .project()
            .status
            .session_state;
        assert_eq!(projected_state, SessionState::Closed);
    }

    #[test]
    fn close_session_state_persists_closed_projection_for_fresh_store_restore() {
        let temp = tempfile::tempdir().unwrap();
        let store = Arc::new(SessionStore::new(Arc::new(
            crate::adaptor::gateway::agent_session::FileSessionStorage::default(),
        )));
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

        let fresh_store = SessionStore::new(Arc::new(
            crate::adaptor::gateway::agent_session::FileSessionStorage::default(),
        ));
        let loaded = fresh_store
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.state, SessionState::Closed);
        let events = fresh_store
            .load_session_events(temp.path(), &session.id)
            .unwrap();
        let projected_state = TurnEventLog::from_events(events)
            .project()
            .status
            .session_state;
        assert_eq!(projected_state, SessionState::Closed);
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
    fn restore_session_state_recovers_closed_from_event_projection_before_idle() {
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
        store
            .set_session_state(temp.path(), &session.id, SessionState::Active)
            .unwrap();
        let session = store
            .load_full_session_for_restore(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(session.state, SessionState::Active);
        let session_id = session.id.clone();

        let projected_state =
            TurnEventLog::from_events(store.load_session_events(temp.path(), &session_id).unwrap())
                .project()
                .status
                .session_state;
        assert_eq!(projected_state, SessionState::Closed);

        let captured = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let captured_for_listener = captured.clone();
        store.register_state_change_listener(Arc::new(move |_, _, state, _| {
            captured_for_listener.lock().push(state.clone());
        }));

        controller.restore_session_state(session).unwrap();

        let loaded = store
            .load_full_session_for_restore(temp.path(), &session_id)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.state, SessionState::Idle);
        assert_eq!(
            captured.lock().as_slice(),
            [SessionState::Closed, SessionState::Idle]
        );
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
            .save_full_session_for_restore(temp.path(), &session)
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
