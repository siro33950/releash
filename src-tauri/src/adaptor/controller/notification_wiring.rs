use std::sync::Arc;

use crate::domain::notification::AgentNotificationState;
use crate::usecase::agent_session::session::{SessionState, SessionStore};
use crate::usecase::notification::usecase::{
    AgentNotificationSnapshot, AgentNotificationStateChange, AgentSessionNotificationError,
    AgentSessionNotificationUsecase,
};

pub(crate) fn register_agent_notification_listener(
    session_store: Arc<SessionStore>,
    notification_usecase: Arc<AgentSessionNotificationUsecase>,
) {
    register_agent_notification_listener_with_clock(
        session_store,
        notification_usecase,
        Arc::new(SystemNotificationClock),
    );
}

trait NotificationClock: Send + Sync {
    fn now(&self) -> f64;
}

struct SystemNotificationClock;

impl NotificationClock for SystemNotificationClock {
    fn now(&self) -> f64 {
        crate::other::utils::unix_timestamp_seconds()
    }
}

fn register_agent_notification_listener_with_clock(
    session_store: Arc<SessionStore>,
    notification_usecase: Arc<AgentSessionNotificationUsecase>,
    clock: Arc<dyn NotificationClock>,
) {
    session_store.register_state_change_listener(Arc::new(
        move |session_id, worktree_path, new_state, _state_revision| {
            let notification_usecase = notification_usecase.clone();
            let snapshot = match notification_snapshot_for_session_state_change(
                notification_usecase.as_ref(),
                clock.as_ref(),
                session_id,
                worktree_path,
                new_state,
            ) {
                Ok(Some(snapshot)) => snapshot,
                Ok(None) => return,
                Err(error) => {
                    log::warn!("{error}");
                    return;
                }
            };
            tauri::async_runtime::spawn(send_notification_snapshot(notification_usecase, snapshot));
        },
    ));
}

fn notification_snapshot_for_session_state_change(
    notification_usecase: &AgentSessionNotificationUsecase,
    clock: &dyn NotificationClock,
    session_id: &str,
    worktree_path: &str,
    new_state: &SessionState,
) -> Result<Option<AgentNotificationSnapshot>, AgentSessionNotificationError> {
    let Some(state) = notification_state_from_session_state(new_state) else {
        return Ok(None);
    };
    notification_usecase
        .snapshot_state_change(AgentNotificationStateChange {
            session_id: session_id.to_string(),
            worktree_path: worktree_path.to_string(),
            state,
            timestamp: clock.now(),
        })
        .map(Some)
}

async fn send_notification_snapshot(
    notification_usecase: Arc<AgentSessionNotificationUsecase>,
    snapshot: AgentNotificationSnapshot,
) {
    if let Err(error) = notification_usecase.send_snapshot(snapshot).await {
        log::warn!("{error}");
    }
}

fn notification_state_from_session_state(state: &SessionState) -> Option<AgentNotificationState> {
    match state {
        SessionState::Active => Some(AgentNotificationState::Running),
        SessionState::Idle => Some(AgentNotificationState::Waiting),
        SessionState::Done => Some(AgentNotificationState::Done),
        SessionState::Closed | SessionState::Archived => None,
        SessionState::Error => Some(AgentNotificationState::Error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::notification::{
        DesktopNotifyMode, NotificationError, NotificationEvent, NotifyConfig, WebhookSenderGateway,
    };
    use crate::usecase::notification::usecase::{
        NotificationInactivityGateway, NotificationSettingsGateway,
    };
    use async_trait::async_trait;
    use parking_lot::Mutex as ParkingMutex;

    struct FixedNotificationClock {
        now: f64,
    }

    impl NotificationClock for FixedNotificationClock {
        fn now(&self) -> f64 {
            self.now
        }
    }

    struct StaticSettingsGateway {
        notify: NotifyConfig,
    }

    impl NotificationSettingsGateway for StaticSettingsGateway {
        fn load_notify_config(&self) -> Result<NotifyConfig, String> {
            Ok(self.notify.clone())
        }
    }

    struct FailingSettingsGateway;

    impl NotificationSettingsGateway for FailingSettingsGateway {
        fn load_notify_config(&self) -> Result<NotifyConfig, String> {
            Err("config boom".to_string())
        }
    }

    struct RecordingInactivityGateway {
        inactive: bool,
        calls: ParkingMutex<Vec<u32>>,
    }

    impl NotificationInactivityGateway for RecordingInactivityGateway {
        fn is_inactive(&self, timeout_minutes: u32) -> bool {
            self.calls.lock().push(timeout_minutes);
            self.inactive
        }
    }

    #[derive(Default)]
    struct RecordingSender {
        urls: ParkingMutex<Vec<String>>,
        events: ParkingMutex<Vec<NotificationEvent>>,
    }

    #[async_trait]
    impl WebhookSenderGateway for RecordingSender {
        async fn send(&self, url: &str, event: NotificationEvent) -> Result<(), NotificationError> {
            self.urls.lock().push(url.to_string());
            self.events.lock().push(event);
            Ok(())
        }
    }

    #[derive(Default)]
    struct FailingSender {
        calls: ParkingMutex<u32>,
    }

    #[async_trait]
    impl WebhookSenderGateway for FailingSender {
        async fn send(
            &self,
            _url: &str,
            _event: NotificationEvent,
        ) -> Result<(), NotificationError> {
            *self.calls.lock() += 1;
            Err(NotificationError::Message("send boom".to_string()))
        }
    }

    fn notify_config() -> NotifyConfig {
        NotifyConfig {
            webhook_url: "https://example.test/hook".to_string(),
            on_running: true,
            on_done: true,
            on_error: true,
            on_waiting: true,
            desktop_mode: DesktopNotifyMode::WhenInactive,
            inactive_timeout_minutes: 9,
        }
    }

    fn usecase_with_sender(
        sender: Arc<dyn WebhookSenderGateway>,
        inactivity: Arc<RecordingInactivityGateway>,
    ) -> AgentSessionNotificationUsecase {
        AgentSessionNotificationUsecase::new(
            Arc::new(StaticSettingsGateway {
                notify: notify_config(),
            }),
            inactivity,
            sender,
        )
    }

    #[test]
    fn maps_session_state_to_notification_state_at_controller_boundary() {
        assert_eq!(
            notification_state_from_session_state(&SessionState::Active),
            Some(AgentNotificationState::Running)
        );
        assert_eq!(
            notification_state_from_session_state(&SessionState::Idle),
            Some(AgentNotificationState::Waiting)
        );
        assert_eq!(
            notification_state_from_session_state(&SessionState::Done),
            Some(AgentNotificationState::Done)
        );
        assert_eq!(
            notification_state_from_session_state(&SessionState::Closed),
            None
        );
        assert_eq!(
            notification_state_from_session_state(&SessionState::Archived),
            None
        );
        assert_eq!(
            notification_state_from_session_state(&SessionState::Error),
            Some(AgentNotificationState::Error)
        );
    }

    #[test]
    fn notification_snapshot_uses_listener_time_config_and_inactivity() {
        let inactivity = Arc::new(RecordingInactivityGateway {
            inactive: true,
            calls: ParkingMutex::new(Vec::new()),
        });
        let usecase = usecase_with_sender(Arc::new(RecordingSender::default()), inactivity.clone());
        let clock = FixedNotificationClock { now: 42.5 };

        let snapshot = notification_snapshot_for_session_state_change(
            &usecase,
            &clock,
            "session-1",
            "/repo",
            &SessionState::Idle,
        )
        .unwrap()
        .unwrap();

        assert_eq!(*inactivity.calls.lock(), vec![9]);
        assert!(snapshot.inactive);
        assert_eq!(snapshot.notify.webhook_url, "https://example.test/hook");
        assert_eq!(snapshot.event.session_id.as_deref(), Some("session-1"));
        assert_eq!(snapshot.event.worktree_path, "/repo");
        assert_eq!(snapshot.event.state, AgentNotificationState::Waiting);
        assert_eq!(snapshot.event.timestamp, 42.5);
    }

    #[test]
    fn closed_and_archived_do_not_create_notification_snapshots() {
        let inactivity = Arc::new(RecordingInactivityGateway {
            inactive: true,
            calls: ParkingMutex::new(Vec::new()),
        });
        let sender = Arc::new(RecordingSender::default());
        let usecase = usecase_with_sender(sender.clone(), inactivity.clone());
        let clock = FixedNotificationClock { now: 42.5 };

        for state in [SessionState::Closed, SessionState::Archived] {
            assert!(notification_snapshot_for_session_state_change(
                &usecase,
                &clock,
                "session-1",
                "/repo",
                &state,
            )
            .unwrap()
            .is_none());
        }

        assert!(inactivity.calls.lock().is_empty());
        assert!(sender.events.lock().is_empty());
    }

    #[tokio::test]
    async fn done_closed_archived_lifecycle_sends_done_once() {
        let inactivity = Arc::new(RecordingInactivityGateway {
            inactive: true,
            calls: ParkingMutex::new(Vec::new()),
        });
        let sender = Arc::new(RecordingSender::default());
        let usecase = Arc::new(usecase_with_sender(sender.clone(), inactivity));
        let clock = FixedNotificationClock { now: 42.5 };

        for state in [
            SessionState::Done,
            SessionState::Closed,
            SessionState::Archived,
        ] {
            if let Some(snapshot) = notification_snapshot_for_session_state_change(
                usecase.as_ref(),
                &clock,
                "session-1",
                "/repo",
                &state,
            )
            .unwrap()
            {
                send_notification_snapshot(usecase.clone(), snapshot).await;
            }
        }

        let events = sender.events.lock();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].state, AgentNotificationState::Done);
    }

    #[test]
    fn snapshot_config_error_does_not_call_sender() {
        let sender = Arc::new(RecordingSender::default());
        let usecase = AgentSessionNotificationUsecase::new(
            Arc::new(FailingSettingsGateway),
            Arc::new(RecordingInactivityGateway {
                inactive: true,
                calls: ParkingMutex::new(Vec::new()),
            }),
            sender.clone(),
        );
        let clock = FixedNotificationClock { now: 42.5 };

        let result = notification_snapshot_for_session_state_change(
            &usecase,
            &clock,
            "session-1",
            "/repo",
            &SessionState::Active,
        );

        assert!(matches!(
            result,
            Err(AgentSessionNotificationError::Config(message)) if message == "config boom"
        ));
        assert!(sender.events.lock().is_empty());
    }

    #[tokio::test]
    async fn listener_send_error_is_logged_without_panic() {
        let inactivity = Arc::new(RecordingInactivityGateway {
            inactive: true,
            calls: ParkingMutex::new(Vec::new()),
        });
        let sender = Arc::new(FailingSender::default());
        let usecase = Arc::new(usecase_with_sender(sender.clone(), inactivity));
        let clock = FixedNotificationClock { now: 42.5 };
        let snapshot = notification_snapshot_for_session_state_change(
            usecase.as_ref(),
            &clock,
            "session-1",
            "/repo",
            &SessionState::Done,
        )
        .unwrap()
        .unwrap();

        send_notification_snapshot(usecase, snapshot).await;

        assert_eq!(*sender.calls.lock(), 1);
    }
}
