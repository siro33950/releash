use std::sync::Arc;

use crate::config::{AppConfig, DesktopNotifyMode as ConfigDesktopNotifyMode, NotifySection};
use crate::domain::notification::{DesktopNotifyMode, NotifyConfig};

#[derive(Clone)]
pub struct NotificationSettingsConfigGateway {
    config: Arc<AppConfig>,
}

impl NotificationSettingsConfigGateway {
    pub fn new(config: Arc<AppConfig>) -> Self {
        Self { config }
    }

    pub fn get_notify_config(&self) -> Result<NotifySection, String> {
        Ok(self.config.get_config()?.server.notify)
    }

    pub fn update_notify_config(&self, notify: NotifySection) -> Result<(), String> {
        self.config.with_config_mut(|config| {
            config.server.notify = notify;
            Ok(())
        })
    }

    pub fn update_webhook_url(&self, url: String) -> Result<(), String> {
        self.config.with_config_mut(|config| {
            config.server.notify.webhook_url = url;
            Ok(())
        })
    }
}

pub fn config_notify_to_domain(notify: NotifySection) -> NotifyConfig {
    NotifyConfig {
        webhook_url: notify.webhook_url,
        on_running: notify.on_running,
        on_done: notify.on_done,
        on_error: notify.on_error,
        on_waiting: notify.on_waiting,
        desktop_mode: match notify.desktop_mode {
            ConfigDesktopNotifyMode::Always => DesktopNotifyMode::Always,
            ConfigDesktopNotifyMode::WhenInactive => DesktopNotifyMode::WhenInactive,
        },
        inactive_timeout_minutes: notify.inactive_timeout_minutes,
    }
}
