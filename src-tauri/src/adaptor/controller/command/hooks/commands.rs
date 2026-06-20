use std::sync::Arc;

use crate::adaptor::gateway::hooks::ClaudeHooksSettingsRepository;
use crate::domain::app_config::ConfigRepository;

#[tauri::command]
pub fn generate_hooks_config(
    state: tauri::State<'_, Arc<dyn ConfigRepository>>,
) -> Result<String, String> {
    let config = state.load().map_err(|e| e.to_string())?;
    crate::usecase::hooks::query_service::generate_hooks_config(
        config.server.hook_port,
        &config.server.token,
    )
}

#[tauri::command]
pub async fn apply_hooks_config(config_json: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let repository = ClaudeHooksSettingsRepository;
        let existing = repository.load_or_empty()?;
        let new_config: serde_json::Value =
            serde_json::from_str(&config_json).map_err(|e| format!("設定JSONパース失敗: {e}"))?;
        let merged = crate::usecase::hooks::usecase::apply_hooks_config(existing, new_config)?;
        repository.save(&merged)
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn get_hooks_status(
    state: tauri::State<'_, Arc<dyn ConfigRepository>>,
) -> Result<String, String> {
    let config = state.load().map_err(|e| e.to_string())?;
    let hook_port = config.server.hook_port;
    let token = config.server.token.clone();

    tokio::task::spawn_blocking(move || {
        let repository = ClaudeHooksSettingsRepository;
        let settings = repository.load_optional()?;
        Ok(
            crate::usecase::hooks::query_service::get_hooks_status(settings, hook_port, &token)
                .as_str()
                .to_string(),
        )
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}
