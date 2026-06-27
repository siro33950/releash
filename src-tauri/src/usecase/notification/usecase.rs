use crate::domain::notification::services::should_notify;
use crate::domain::notification::{
    AgentNotificationState, NotificationError, NotificationEvent, NotifyConfig,
    WebhookSenderGateway,
};

pub trait NotificationSettingsGateway: Send + Sync {
    fn load_notify_config(&self) -> Result<NotifyConfig, String>;
}

pub trait NotificationInactivityGateway: Send + Sync {
    fn is_inactive(&self, timeout_minutes: u32) -> bool;
}

#[derive(Debug)]
pub enum AgentSessionNotificationError {
    Config(String),
    Send(NotificationError),
}

impl std::fmt::Display for AgentSessionNotificationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Config(message) => write!(f, "failed to load notification config: {message}"),
            Self::Send(error) => write!(f, "failed to send agent notification: {error}"),
        }
    }
}

impl std::error::Error for AgentSessionNotificationError {}

#[derive(Debug, Clone, PartialEq)]
pub struct AgentNotificationStateChange {
    pub session_id: String,
    pub worktree_path: String,
    pub state: AgentNotificationState,
    pub timestamp: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AgentNotificationSnapshot {
    pub notify: NotifyConfig,
    pub inactive: bool,
    pub event: NotificationEvent,
}

pub struct AgentSessionNotificationUsecase {
    settings: std::sync::Arc<dyn NotificationSettingsGateway>,
    inactivity: std::sync::Arc<dyn NotificationInactivityGateway>,
    sender: std::sync::Arc<dyn WebhookSenderGateway>,
}

impl AgentSessionNotificationUsecase {
    pub fn new(
        settings: std::sync::Arc<dyn NotificationSettingsGateway>,
        inactivity: std::sync::Arc<dyn NotificationInactivityGateway>,
        sender: std::sync::Arc<dyn WebhookSenderGateway>,
    ) -> Self {
        Self {
            settings,
            inactivity,
            sender,
        }
    }

    pub fn snapshot_state_change(
        &self,
        change: AgentNotificationStateChange,
    ) -> Result<AgentNotificationSnapshot, AgentSessionNotificationError> {
        let notify = self
            .settings
            .load_notify_config()
            .map_err(AgentSessionNotificationError::Config)?;
        let inactive = self.inactivity.is_inactive(notify.inactive_timeout_minutes);
        let event = notification_event_from_state_change(change);
        Ok(AgentNotificationSnapshot {
            notify,
            inactive,
            event,
        })
    }

    pub async fn send_snapshot(
        &self,
        snapshot: AgentNotificationSnapshot,
    ) -> Result<(), AgentSessionNotificationError> {
        on_agent_status_changed(
            snapshot.notify,
            snapshot.inactive,
            snapshot.event,
            self.sender.as_ref(),
        )
        .await
        .map_err(AgentSessionNotificationError::Send)
    }
}

pub async fn on_agent_status_changed(
    notify: NotifyConfig,
    inactive: bool,
    event: NotificationEvent,
    sender: &dyn WebhookSenderGateway,
) -> Result<(), NotificationError> {
    let url = notify.webhook_url.clone();
    if url.is_empty() || !should_notify(&notify, &event.state, inactive) {
        return Ok(());
    }
    sender.send(&url, event).await
}

fn notification_event_from_state_change(change: AgentNotificationStateChange) -> NotificationEvent {
    NotificationEvent {
        worktree_path: change.worktree_path,
        state: change.state,
        exit_code: None,
        timestamp: change.timestamp,
        session_id: Some(change.session_id),
        pty_id: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::notification::DesktopNotifyMode;
    use async_trait::async_trait;
    use parking_lot::Mutex;
    use std::sync::Arc;

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
        calls: Mutex<Vec<u32>>,
    }

    impl NotificationInactivityGateway for RecordingInactivityGateway {
        fn is_inactive(&self, timeout_minutes: u32) -> bool {
            self.calls.lock().push(timeout_minutes);
            self.inactive
        }
    }

    #[derive(Default)]
    struct RecordingSender {
        urls: Mutex<Vec<String>>,
        events: Mutex<Vec<NotificationEvent>>,
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
        calls: Mutex<u32>,
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
            inactive_timeout_minutes: 7,
        }
    }

    #[tokio::test]
    async fn state_change_snapshot_uses_configured_inactivity_timeout_and_sends_event() {
        let inactivity = Arc::new(RecordingInactivityGateway {
            inactive: true,
            calls: Mutex::new(Vec::new()),
        });
        let sender = Arc::new(RecordingSender::default());
        let usecase = AgentSessionNotificationUsecase::new(
            Arc::new(StaticSettingsGateway {
                notify: notify_config(),
            }),
            inactivity.clone(),
            sender.clone(),
        );

        let snapshot = usecase
            .snapshot_state_change(AgentNotificationStateChange {
                session_id: "session-1".to_string(),
                worktree_path: "/repo".to_string(),
                state: AgentNotificationState::Waiting,
                timestamp: 123.0,
            })
            .unwrap();
        usecase.send_snapshot(snapshot).await.unwrap();

        assert_eq!(*inactivity.calls.lock(), vec![7]);
        let events = sender.events.lock();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].session_id.as_deref(), Some("session-1"));
        assert_eq!(events[0].worktree_path, "/repo");
        assert_eq!(events[0].state, AgentNotificationState::Waiting);
        assert_eq!(events[0].timestamp, 123.0);
        assert_eq!(sender.urls.lock().as_slice(), ["https://example.test/hook"]);
    }

    #[tokio::test]
    async fn send_snapshot_uses_captured_notify_and_inactive_values() {
        let inactivity = Arc::new(RecordingInactivityGateway {
            inactive: true,
            calls: Mutex::new(Vec::new()),
        });
        let sender = Arc::new(RecordingSender::default());
        let usecase = AgentSessionNotificationUsecase::new(
            Arc::new(StaticSettingsGateway {
                notify: notify_config(),
            }),
            inactivity,
            sender.clone(),
        );
        let mut snapshot = usecase
            .snapshot_state_change(AgentNotificationStateChange {
                session_id: "session-1".to_string(),
                worktree_path: "/repo".to_string(),
                state: AgentNotificationState::Done,
                timestamp: 456.0,
            })
            .unwrap();

        snapshot.notify.webhook_url = "https://example.test/captured".to_string();
        usecase.send_snapshot(snapshot).await.unwrap();

        let events = sender.events.lock();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].state, AgentNotificationState::Done);
        assert_eq!(events[0].timestamp, 456.0);
        assert_eq!(
            sender.urls.lock().as_slice(),
            ["https://example.test/captured"]
        );
    }

    #[test]
    fn snapshot_state_change_returns_config_error_without_sending() {
        let sender = Arc::new(RecordingSender::default());
        let usecase = AgentSessionNotificationUsecase::new(
            Arc::new(FailingSettingsGateway),
            Arc::new(RecordingInactivityGateway {
                inactive: true,
                calls: Mutex::new(Vec::new()),
            }),
            sender.clone(),
        );

        let result = usecase.snapshot_state_change(AgentNotificationStateChange {
            session_id: "session-1".to_string(),
            worktree_path: "/repo".to_string(),
            state: AgentNotificationState::Running,
            timestamp: 123.0,
        });

        assert!(matches!(
            result,
            Err(AgentSessionNotificationError::Config(message)) if message == "config boom"
        ));
        assert!(sender.events.lock().is_empty());
    }

    #[tokio::test]
    async fn send_snapshot_propagates_sender_error_without_panic() {
        let sender = Arc::new(FailingSender::default());
        let usecase = AgentSessionNotificationUsecase::new(
            Arc::new(StaticSettingsGateway {
                notify: notify_config(),
            }),
            Arc::new(RecordingInactivityGateway {
                inactive: true,
                calls: Mutex::new(Vec::new()),
            }),
            sender.clone(),
        );
        let snapshot = usecase
            .snapshot_state_change(AgentNotificationStateChange {
                session_id: "session-1".to_string(),
                worktree_path: "/repo".to_string(),
                state: AgentNotificationState::Done,
                timestamp: 456.0,
            })
            .unwrap();

        let result = usecase.send_snapshot(snapshot).await;

        assert!(matches!(
            result,
            Err(AgentSessionNotificationError::Send(NotificationError::Message(message)))
                if message == "send boom"
        ));
        assert_eq!(*sender.calls.lock(), 1);
    }
}
