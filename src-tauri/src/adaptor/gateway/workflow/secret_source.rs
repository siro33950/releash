use std::sync::Arc;

use tauri::Manager;

use crate::domain::app_config::ConfigSecretRepository;

pub(crate) fn collect_configured_secret_values<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> Vec<String> {
    let mut values = Vec::new();
    if let Some(config) = app.try_state::<Arc<dyn ConfigSecretRepository>>() {
        values.extend(config.configured_secret_values().unwrap_or_default());
    }
    values.extend(
        crate::domain::workflow::services::secret_masker::collect_secret_values_from_env_vars(
            std::env::vars(),
        ),
    );
    crate::domain::workflow::services::secret_masker::normalize_secret_values(values)
}
