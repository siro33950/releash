use std::sync::Arc;

use tauri::Manager;

pub(crate) fn collect_configured_secret_values<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> Vec<String> {
    let mut values = Vec::new();
    if let Some(config) = app.try_state::<Arc<crate::config::AppConfig>>() {
        if let Ok(cfg) = config.get_config() {
            values.extend(collect_configured_secret_values_from_config(&cfg));
        }
    }
    values.extend(
        crate::domain::workflow::services::secret_masker::collect_secret_values_from_env_vars(
            std::env::vars(),
        ),
    );
    crate::domain::workflow::services::secret_masker::normalize_secret_values(values)
}

pub(crate) fn collect_configured_secret_values_from_config(
    cfg: &crate::config::ReleashConfig,
) -> Vec<String> {
    let mut values = Vec::new();
    for v in [
        cfg.server.token.as_str(),
        cfg.server.mcp_token.as_str(),
        cfg.server.notify.webhook_url.as_str(),
    ] {
        if v.len() >= 8 {
            values.push(v.to_string());
        }
    }
    for notion in cfg.notion.values() {
        if notion.api_token.len() >= 8 {
            values.push(notion.api_token.clone());
        }
    }
    values
}
