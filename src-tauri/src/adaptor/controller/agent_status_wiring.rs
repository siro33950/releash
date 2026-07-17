use std::sync::Arc;

use crate::usecase::agent_session::session::SessionStore;
use crate::usecase::agent_session::status::{AgentStatusCenter, AgentStatusNotifier};

pub(crate) fn register_agent_status_listener(
    session_store: Arc<SessionStore>,
    center: Arc<AgentStatusCenter>,
    notifier: Arc<dyn AgentStatusNotifier>,
) {
    session_store.register_state_change_listener(Arc::new(
        move |session_id, _worktree_path, new_state, state_revision| {
            let changes =
                center.on_session_state_changed(session_id, new_state.clone(), state_revision);
            notifier.status_changed(changes);
        },
    ));
}

#[cfg(test)]
mod tests {
    use std::sync::{Barrier, Mutex};

    use super::*;
    use crate::usecase::agent_session::session::{
        create_session_internal, ErrorEpisodeInput, SessionState,
    };
    use crate::usecase::agent_session::status::{
        AgentStatusChanges, SessionStatus, TurnPhase, TurnPhaseRepr,
    };

    #[derive(Default)]
    struct RecordingStatusNotifier {
        changes: Mutex<Vec<AgentStatusChanges>>,
    }

    impl AgentStatusNotifier for RecordingStatusNotifier {
        fn status_changed(&self, changes: AgentStatusChanges) {
            self.changes.lock().unwrap().push(changes);
        }
    }

    #[test]
    fn error_reason_change_republishes_session_status_for_backend_refetch() {
        let app_data_dir = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let session = create_session_internal(
            &session_store,
            app_data_dir.path(),
            "/repo",
            Some("codex".to_string()),
        )
        .unwrap();
        let center = Arc::new(AgentStatusCenter::new());
        center.update_session(SessionStatus {
            chat_session_id: session.id.clone(),
            worktree_id: session.worktree_path.clone(),
            worktree_path: session.worktree_path.clone(),
            pty_id: None,
            agent_state: AgentStatusCenter::derive_agent_state(
                TurnPhase::Idle,
                SessionState::Error,
            ),
            turn_phase: TurnPhaseRepr::Idle,
            session_state: SessionState::Error,
            pending_permission: false,
            pending_permission_request: None,
            last_activity_at: 0.0,
            workflow_node: None,
            workflow_execution_status: None,
            workflow_execution_id: None,
            node_execution_id: None,
            workflow_attempt: None,
            notice: None,
            workflow_node_progress: None,
        });
        let notifier = Arc::new(RecordingStatusNotifier::default());
        register_agent_status_listener(Arc::clone(&session_store), center, notifier.clone());

        for (message_id, reason, at) in [
            ("fatal-1", "first fatal", 1.0),
            ("fatal-2", "latest fatal", 2.0),
        ] {
            session_store
                .append_error_episode_and_materialize(
                    app_data_dir.path(),
                    &session.id,
                    ErrorEpisodeInput {
                        message_id: message_id.to_string(),
                        reason: reason.to_string(),
                        at,
                    },
                )
                .unwrap();
        }

        let changes = notifier.changes.lock().unwrap();
        assert_eq!(changes.len(), 2);
        assert!(changes.iter().all(|change| {
            change
                .session
                .as_ref()
                .is_some_and(|status| status.chat_session_id == session.id)
        }));
    }

    #[test]
    fn stale_error_notification_cannot_overwrite_later_closed_commit() {
        let app_data_dir = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let session = create_session_internal(
            &session_store,
            app_data_dir.path(),
            "/repo",
            Some("codex".to_string()),
        )
        .unwrap();
        let center = Arc::new(AgentStatusCenter::new());
        center.update_session(SessionStatus {
            chat_session_id: session.id.clone(),
            worktree_id: session.worktree_path.clone(),
            worktree_path: session.worktree_path.clone(),
            pty_id: None,
            agent_state: AgentStatusCenter::derive_agent_state(
                TurnPhase::Idle,
                SessionState::Active,
            ),
            turn_phase: TurnPhaseRepr::Idle,
            session_state: SessionState::Active,
            pending_permission: false,
            pending_permission_request: None,
            last_activity_at: 0.0,
            workflow_node: None,
            workflow_execution_status: None,
            workflow_execution_id: None,
            node_execution_id: None,
            workflow_attempt: None,
            notice: None,
            workflow_node_progress: None,
        });
        let error_notification_entered = Arc::new(Barrier::new(2));
        let release_error_notification = Arc::new(Barrier::new(2));
        session_store.register_state_change_listener({
            let entered = Arc::clone(&error_notification_entered);
            let release = Arc::clone(&release_error_notification);
            Arc::new(move |_, _, state, _| {
                if state == &SessionState::Error {
                    entered.wait();
                    release.wait();
                }
            })
        });
        let notifier = Arc::new(RecordingStatusNotifier::default());
        register_agent_status_listener(Arc::clone(&session_store), Arc::clone(&center), notifier);

        let error_store = Arc::clone(&session_store);
        let error_data_dir = app_data_dir.path().to_path_buf();
        let error_session_id = session.id.clone();
        let error_thread = std::thread::spawn(move || {
            error_store.append_error_episode_and_materialize(
                &error_data_dir,
                &error_session_id,
                ErrorEpisodeInput {
                    message_id: "fatal-message".to_string(),
                    reason: "app server stopped".to_string(),
                    at: 2.0,
                },
            )
        });
        error_notification_entered.wait();

        session_store
            .set_session_state(app_data_dir.path(), &session.id, SessionState::Closed)
            .unwrap();
        let disk_meta = session_store
            .get_session_meta(app_data_dir.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(disk_meta.state, SessionState::Closed);
        assert_eq!(
            center
                .get_session(&session.id)
                .map(|status| status.session_state),
            Some(SessionState::Closed)
        );

        release_error_notification.wait();
        error_thread.join().unwrap().unwrap();
        assert_eq!(
            center
                .get_session(&session.id)
                .map(|status| status.session_state),
            Some(SessionState::Closed)
        );
    }
}
