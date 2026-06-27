use std::sync::Arc;

use crate::domain::app_config::ConfigRepository;
use crate::domain::notification::NotifyConfig;
use crate::usecase::notification::usecase::NotificationSettingsGateway;

#[derive(Clone)]
pub struct NotificationSettingsConfigGateway {
    config: Arc<dyn ConfigRepository>,
}

impl NotificationSettingsConfigGateway {
    pub fn new(config: Arc<dyn ConfigRepository>) -> Self {
        Self { config }
    }

    pub fn get_notify_config(&self) -> Result<NotifyConfig, String> {
        Ok(self.config.load().map_err(|e| e.to_string())?.server.notify)
    }

    pub fn update_notify_config(&self, notify: NotifyConfig) -> Result<(), String> {
        let mut config = self.config.load().map_err(|e| e.to_string())?;
        config.server.notify = notify;
        self.config.save(config).map_err(|e| e.to_string())
    }

    pub fn update_webhook_url(&self, url: String) -> Result<(), String> {
        let mut config = self.config.load().map_err(|e| e.to_string())?;
        config.server.notify.webhook_url = url;
        self.config.save(config).map_err(|e| e.to_string())
    }
}

impl NotificationSettingsGateway for NotificationSettingsConfigGateway {
    fn load_notify_config(&self) -> Result<NotifyConfig, String> {
        self.get_notify_config()
    }
}
