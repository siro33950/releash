use std::sync::Arc;

use crate::usecase::agent_session::event_log::{AgentSessionEvent, TurnEventLog};

use super::{now_timestamp, ChatSession, RestoreSessionResponse, SessionState, SessionStore};

pub struct SessionLifecycleController<'a> {
    pub session_store: &'a Arc<SessionStore>,
    pub data_dir: &'a std::path::Path,
}

impl<'a> SessionLifecycleController<'a> {
    pub fn complete_turn_state(
        &self,
        session_id: &str,
        exit_code: i64,
        interrupted: bool,
    ) -> Result<SessionState, String> {
        let session_state = Self::completed_turn_session_state(exit_code, interrupted);
        super::update_session_state_in_data_dir(
            self.session_store,
            self.data_dir,
            session_id,
            session_state,
        )?;
        let meta = self
            .session_store
            .get_session_meta(self.data_dir, session_id)?
            .ok_or_else(|| format!("Session not found: {session_id}"))?;
        Ok(meta.state)
    }

    pub fn completed_turn_session_state(exit_code: i64, interrupted: bool) -> SessionState {
        if interrupted {
            SessionState::Idle
        } else if exit_code == 0 {
            SessionState::Done
        } else {
            SessionState::Error
        }
    }

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

    pub fn restore_session_state(
        &self,
        session: ChatSession,
    ) -> Result<RestoreSessionResponse, String> {
        let projected_state = self.project_session_state(&session.id)?;
        if projected_state != session.state {
            self.session_store
                .set_session_state(self.data_dir, &session.id, projected_state)?;
        }
        self.session_store
            .set_session_state(self.data_dir, &session.id, SessionState::Idle)?;
        Ok(RestoreSessionResponse {
            restored_workflow_step: false,
        })
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
    fn completed_turn_session_state_matches_terminal_outcome() {
        assert_eq!(
            SessionLifecycleController::completed_turn_session_state(0, false),
            SessionState::Done
        );
        assert_eq!(
            SessionLifecycleController::completed_turn_session_state(1, false),
            SessionState::Error
        );
        assert_eq!(
            SessionLifecycleController::completed_turn_session_state(0, true),
            SessionState::Idle
        );
        assert_eq!(
            SessionLifecycleController::completed_turn_session_state(1, true),
            SessionState::Idle
        );
    }

    #[test]
    fn complete_turn_state_updates_session_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let store = Arc::new(crate::test_support::build_session_store());
        let session =
            super::super::create_session_internal(&store, temp.path(), "/repo", None).unwrap();

        let controller = SessionLifecycleController {
            session_store: &store,
            data_dir: temp.path(),
        };
        let saved_state = controller
            .complete_turn_state(&session.id, 0, false)
            .unwrap();

        assert_eq!(saved_state, SessionState::Done);
        let meta = store
            .get_session_meta(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(meta.state, SessionState::Done);
    }

    #[test]
    fn complete_turn_state_returns_existing_state_when_guard_skips_update() {
        let temp = tempfile::tempdir().unwrap();
        let store = Arc::new(crate::test_support::build_session_store());
        let mut session =
            super::super::create_session_internal(&store, temp.path(), "/repo", None).unwrap();
        session.workflow_step_session = true;
        session.state = SessionState::Closed;
        store
            .save_full_session_for_migration_or_restore(temp.path(), &session)
            .unwrap();

        let controller = SessionLifecycleController {
            session_store: &store,
            data_dir: temp.path(),
        };
        let saved_state = controller
            .complete_turn_state(&session.id, 0, false)
            .unwrap();

        assert_eq!(saved_state, SessionState::Closed);
        let meta = store
            .get_session_meta(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(meta.state, SessionState::Closed);
    }

    #[test]
    fn complete_turn_state_returns_error_when_session_missing() {
        let temp = tempfile::tempdir().unwrap();
        let store = Arc::new(crate::test_support::build_session_store());
        let controller = SessionLifecycleController {
            session_store: &store,
            data_dir: temp.path(),
        };

        let result = controller.complete_turn_state("missing-session", 0, false);

        assert!(result.is_err());
    }

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
        store.register_state_change_listener(Arc::new(move |_, _, state| {
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
