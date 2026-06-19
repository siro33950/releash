use std::sync::Arc;

use crate::adaptor::gateway::app_config::{notify_to_domain, notify_to_model, NotifySection};
use crate::adaptor::gateway::notification::NotificationSettingsConfigGateway;
use crate::domain::app_config::ConfigRepository;

#[tauri::command]
pub fn get_notify_config(
    state: tauri::State<'_, Arc<dyn ConfigRepository>>,
) -> Result<NotifySection, String> {
    let gateway = NotificationSettingsConfigGateway::new(state.inner().clone());
    let notify = gateway.get_notify_config()?;
    let notify = crate::usecase::notification::query_service::get_notify_config(notify);
    Ok(notify_to_model(notify))
}

#[tauri::command]
pub async fn update_notify_config(
    state: tauri::State<'_, Arc<dyn ConfigRepository>>,
    notify: NotifySection,
) -> Result<(), String> {
    let gateway = NotificationSettingsConfigGateway::new(state.inner().clone());
    let notify = notify_to_domain(&notify);
    tokio::task::spawn_blocking(move || gateway.update_notify_config(notify))
        .await
        .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn update_webhook_url(
    state: tauri::State<'_, Arc<dyn ConfigRepository>>,
    url: String,
) -> Result<(), String> {
    let gateway = NotificationSettingsConfigGateway::new(state.inner().clone());
    tokio::task::spawn_blocking(move || gateway.update_webhook_url(url))
        .await
        .map_err(|e| format!("task join error: {e}"))?
}
