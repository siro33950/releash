pub mod client;
pub mod types;

use std::sync::Arc;

use types::{
    NotionConfigStatus, NotionLabelOption, NotionRepoConfig, NotionTaskPage, NotionTaskQuery,
    NotionValidationResult, PropertyMapping,
};

use crate::config::AppConfig;

#[tauri::command]
pub async fn query_notion_tasks(
    app_config: tauri::State<'_, Arc<AppConfig>>,
    repo_path: String,
    query: NotionTaskQuery,
) -> Result<NotionTaskPage, String> {
    let config = app_config.get_config()?;
    let notion_config = config
        .notion
        .get(&repo_path)
        .cloned()
        .ok_or_else(|| "Notion設定が見つかりません".to_string())?;

    tokio::task::spawn_blocking(move || {
        client::query_tasks(&notion_config, &query).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn fetch_notion_label_options(
    app_config: tauri::State<'_, Arc<AppConfig>>,
    repo_path: String,
) -> Result<Vec<NotionLabelOption>, String> {
    let config = app_config.get_config()?;
    let notion_config = config
        .notion
        .get(&repo_path)
        .cloned()
        .ok_or_else(|| "Notion設定が見つかりません".to_string())?;

    tokio::task::spawn_blocking(move || {
        client::fetch_label_options(&notion_config).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn save_notion_config(
    app_config: tauri::State<'_, Arc<AppConfig>>,
    repo_path: String,
    api_token: String,
    database_id: String,
    property_mapping: PropertyMapping,
) -> Result<(), String> {
    let app_config = app_config.inner().clone();
    tokio::task::spawn_blocking(move || {
        let mut config = app_config
            .get_config()
            .map_err(|e| format!("設定取得失敗: {e}"))?;
        config.notion.insert(
            repo_path,
            NotionRepoConfig {
                api_token,
                database_id,
                property_mapping,
            },
        );
        crate::config::write_config(&app_config.config_path(), &config)?;
        app_config.set_config(config)?;
        Ok(())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub fn get_notion_config(
    app_config: tauri::State<'_, Arc<AppConfig>>,
    repo_path: String,
) -> Result<Option<NotionRepoConfig>, String> {
    let config = app_config.get_config()?;
    Ok(config.notion.get(&repo_path).cloned())
}

#[tauri::command]
pub async fn delete_notion_config(
    app_config: tauri::State<'_, Arc<AppConfig>>,
    repo_path: String,
) -> Result<(), String> {
    let app_config = app_config.inner().clone();
    tokio::task::spawn_blocking(move || {
        let mut config = app_config
            .get_config()
            .map_err(|e| format!("設定取得失敗: {e}"))?;
        config.notion.remove(&repo_path);
        crate::config::write_config(&app_config.config_path(), &config)?;
        app_config.set_config(config)?;
        Ok(())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn validate_notion_config(
    api_token: String,
    database_id: String,
) -> Result<NotionValidationResult, String> {
    tokio::task::spawn_blocking(move || {
        if api_token.is_empty() || database_id.is_empty() {
            return Ok(NotionValidationResult {
                status: NotionConfigStatus::NotConfigured,
                properties: vec![],
            });
        }
        let config = NotionRepoConfig {
            api_token,
            database_id,
            property_mapping: PropertyMapping::default(),
        };
        Ok(client::validate_config(&config))
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}
