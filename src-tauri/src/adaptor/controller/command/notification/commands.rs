use std::sync::Arc;

use crate::adaptor::gateway::notification::{
    config_notify_to_domain, domain_notify_to_config, NotificationSettingsConfigGateway,
};
use crate::config::{AppConfig, NotifySection};

#[tauri::command]
pub fn get_notify_config(state: tauri::State<'_, Arc<AppConfig>>) -> Result<NotifySection, String> {
    let gateway = NotificationSettingsConfigGateway::new(state.inner().clone());
    let notify = config_notify_to_domain(gateway.get_notify_config()?);
    let notify = crate::usecase::notification::query_service::get_notify_config(notify);
    Ok(domain_notify_to_config(notify))
}

#[tauri::command]
pub async fn update_notify_config(
    state: tauri::State<'_, Arc<AppConfig>>,
    notify: NotifySection,
) -> Result<(), String> {
    let gateway = NotificationSettingsConfigGateway::new(state.inner().clone());
    tokio::task::spawn_blocking(move || gateway.update_notify_config(notify))
        .await
        .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn update_webhook_url(
    state: tauri::State<'_, Arc<AppConfig>>,
    url: String,
) -> Result<(), String> {
    let gateway = NotificationSettingsConfigGateway::new(state.inner().clone());
    tokio::task::spawn_blocking(move || gateway.update_webhook_url(url))
        .await
        .map_err(|e| format!("task join error: {e}"))?
}
