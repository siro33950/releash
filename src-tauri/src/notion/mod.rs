pub mod client;
pub mod types;

use std::sync::Arc;

use types::{
    LabelProperty, NotionConfigStatus, NotionLabelOption, NotionRepoConfig, NotionTaskPage,
    NotionTaskQuery, NotionValidationResult, PropertyMapping,
};

use crate::domain::app_config::value_objects as app_config_vo;
use crate::domain::app_config::NotionConfigRepository;

#[tauri::command]
pub async fn query_notion_tasks(
    app_config: tauri::State<'_, Arc<dyn NotionConfigRepository>>,
    repo_path: String,
    query: NotionTaskQuery,
) -> Result<NotionTaskPage, String> {
    let notion_config = app_config
        .inner()
        .get(&repo_path)
        .map_err(|e| e.to_string())?
        .map(domain_to_wire)
        .ok_or_else(|| "Notion設定が見つかりません".to_string())?;

    tokio::task::spawn_blocking(move || {
        client::query_tasks(&notion_config, &query).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn fetch_notion_label_options(
    app_config: tauri::State<'_, Arc<dyn NotionConfigRepository>>,
    repo_path: String,
) -> Result<Vec<NotionLabelOption>, String> {
    let notion_config = app_config
        .inner()
        .get(&repo_path)
        .map_err(|e| e.to_string())?
        .map(domain_to_wire)
        .ok_or_else(|| "Notion設定が見つかりません".to_string())?;

    tokio::task::spawn_blocking(move || {
        client::fetch_label_options(&notion_config).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn save_notion_config(
    app_config: tauri::State<'_, Arc<dyn NotionConfigRepository>>,
    repo_path: String,
    api_token: String,
    database_id: String,
    property_mapping: PropertyMapping,
) -> Result<(), String> {
    let app_config = app_config.inner().clone();
    let config = wire_to_domain(NotionRepoConfig {
        api_token,
        database_id,
        property_mapping,
    });
    tokio::task::spawn_blocking(move || {
        app_config
            .upsert(repo_path, config)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub fn get_notion_config(
    app_config: tauri::State<'_, Arc<dyn NotionConfigRepository>>,
    repo_path: String,
) -> Result<Option<NotionRepoConfig>, String> {
    app_config
        .inner()
        .get(&repo_path)
        .map(|config| config.map(domain_to_wire))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_notion_config(
    app_config: tauri::State<'_, Arc<dyn NotionConfigRepository>>,
    repo_path: String,
) -> Result<(), String> {
    let app_config = app_config.inner().clone();
    tokio::task::spawn_blocking(move || app_config.remove(&repo_path).map_err(|e| e.to_string()))
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

fn domain_to_wire(config: app_config_vo::NotionRepoConfig) -> NotionRepoConfig {
    NotionRepoConfig {
        api_token: config.api_token,
        database_id: config.database_id,
        property_mapping: domain_mapping_to_wire(config.property_mapping),
    }
}

fn wire_to_domain(config: NotionRepoConfig) -> app_config_vo::NotionRepoConfig {
    app_config_vo::NotionRepoConfig {
        api_token: config.api_token,
        database_id: config.database_id,
        property_mapping: wire_mapping_to_domain(config.property_mapping),
    }
}

fn domain_mapping_to_wire(mapping: app_config_vo::NotionPropertyMapping) -> PropertyMapping {
    PropertyMapping {
        title: mapping.title,
        labels: mapping
            .labels
            .into_iter()
            .map(|label| LabelProperty {
                name: label.name,
                property_type: label.property_type,
            })
            .collect(),
        branch_name: mapping.branch_name,
        branch_prefix: mapping.branch_prefix,
    }
}

fn wire_mapping_to_domain(mapping: PropertyMapping) -> app_config_vo::NotionPropertyMapping {
    app_config_vo::NotionPropertyMapping {
        title: mapping.title,
        labels: mapping
            .labels
            .into_iter()
            .map(|label| app_config_vo::NotionLabelProperty {
                name: label.name,
                property_type: label.property_type,
            })
            .collect(),
        branch_name: mapping.branch_name,
        branch_prefix: mapping.branch_prefix,
    }
}
