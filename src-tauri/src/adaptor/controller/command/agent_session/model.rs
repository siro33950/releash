use std::sync::Arc;

use tokio::sync::Mutex;

use crate::infrastructure::agent_session::runtime::codex::configured_cli_path;
use crate::infrastructure::agent_session::runtime::codex_app_server::{
    build_model_list_request, CodexAppServerProcess,
};
use crate::infrastructure::agent_session::runtime::{
    AgentBackendRegistry, AgentProcessMap, SessionHandle, CODEX_BACKEND_ID,
};
use crate::usecase::agent_session::session::{ModelInfo, SessionStore};

fn parse_codex_model_catalog_page(value: &serde_json::Value) -> (Vec<ModelInfo>, Option<String>) {
    let models = value
        .get("data")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let model = item
                        .get("model")
                        .and_then(serde_json::Value::as_str)
                        .map(str::trim)
                        .filter(|model| !model.is_empty())
                        .or_else(|| {
                            item.get("id")
                                .and_then(serde_json::Value::as_str)
                                .map(str::trim)
                                .filter(|model| !model.is_empty())
                        })?;
                    Some(ModelInfo {
                        value: model.to_string(),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let next_cursor = value
        .get("nextCursor")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|cursor| !cursor.is_empty())
        .map(ToString::to_string);
    (models, next_cursor)
}

fn dedupe_model_catalog(models: Vec<ModelInfo>) -> Vec<ModelInfo> {
    let mut seen = std::collections::HashSet::new();
    models
        .into_iter()
        .filter(|model| seen.insert(model.value.clone()))
        .collect()
}

#[tauri::command]
pub async fn read_codex_model_catalog(app: tauri::AppHandle) -> Result<Vec<ModelInfo>, String> {
    let cli_path = configured_cli_path(&app).unwrap_or_else(|| "codex".to_string());
    let mut process = CodexAppServerProcess::spawn(&cli_path)?;
    let result = async {
        process.initialize(env!("CARGO_PKG_VERSION")).await?;

        let mut cursor: Option<String> = None;
        let mut models = Vec::new();
        for _ in 0..20 {
            let id = process.next_request_id();
            process
                .send(&build_model_list_request(id, cursor.as_deref()))
                .await?;
            let page = process.read_response_result(id).await?;
            let (page_models, next_cursor) = parse_codex_model_catalog_page(&page);
            models.extend(page_models);
            cursor = next_cursor;
            if cursor.is_none() {
                return Ok(dedupe_model_catalog(models));
            }
        }
        Err("Codex model catalog pagination did not finish within 20 pages".to_string())
    }
    .await;
    process.shutdown().await;
    result
}

#[tauri::command]
pub async fn set_agent_permission_mode(
    app: tauri::AppHandle,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    registry: tauri::State<'_, Arc<AgentBackendRegistry>>,
    chat_session_id: String,
    permission_mode: String,
) -> Result<(), String> {
    let pm = crate::permission::PermissionMode::parse(&permission_mode)
        .map_err(|error| error.to_string())?;
    let data_dir = crate::app_data_dir::resolve_data_dir(&app)?;
    let session = session_store
        .get_session(&data_dir, &chat_session_id)?
        .ok_or_else(|| format!("Session not found: {chat_session_id}"))?;

    if session.backend_id.as_deref() == Some(CODEX_BACKEND_ID) {
        session_store.update_permission_mode(&data_dir, &chat_session_id, pm.as_str())?;
        session_store.update_permission_profile_id(&data_dir, &chat_session_id, None)?;
        if let Some(backend) = registry.get(CODEX_BACKEND_ID) {
            if let Err(error) = backend
                .set_permission_mode(
                    &SessionHandle {
                        chat_session_id: chat_session_id.clone(),
                        backend_id: CODEX_BACKEND_ID.to_string(),
                    },
                    &session.worktree_path,
                    pm.as_str(),
                )
                .await
            {
                log::debug!(
                    "skipped Codex runtime permission mode sync for {chat_session_id}: {error}"
                );
            }
        }
        return Ok(());
    }

    crate::infrastructure::agent_session::runtime::set_agent_permission_mode(
        app,
        session_store,
        handles,
        chat_session_id,
        pm.as_str().to_string(),
    )
    .await
}

#[tauri::command]
pub async fn set_agent_model(
    app: tauri::AppHandle,
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    registry: tauri::State<'_, Arc<AgentBackendRegistry>>,
    chat_session_id: String,
    model_id: String,
) -> Result<(), String> {
    crate::infrastructure::agent_session::runtime::set_agent_model(
        app,
        handles,
        session_store,
        registry,
        chat_session_id,
        model_id,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn models(values: &[&str]) -> Vec<ModelInfo> {
        values
            .iter()
            .map(|value| ModelInfo {
                value: (*value).to_string(),
            })
            .collect()
    }

    fn model_values(models: &[ModelInfo]) -> Vec<String> {
        models.iter().map(|model| model.value.clone()).collect()
    }

    #[test]
    fn parses_codex_model_catalog_page_from_model_field() {
        let page = serde_json::json!({
            "data": [
                { "id": "model-row-1", "model": "gpt-5.4-codex", "displayName": "GPT 5.4 Codex" },
                { "id": "fallback-id", "model": "", "displayName": "Fallback" },
                { "id": "model-row-2", "model": "gpt-5.4-codex-mini", "displayName": "Mini" }
            ],
            "nextCursor": "cursor-2"
        });

        let (models, cursor) = parse_codex_model_catalog_page(&page);

        assert_eq!(
            models,
            vec![
                ModelInfo {
                    value: "gpt-5.4-codex".to_string()
                },
                ModelInfo {
                    value: "fallback-id".to_string()
                },
                ModelInfo {
                    value: "gpt-5.4-codex-mini".to_string()
                },
            ]
        );
        assert_eq!(cursor.as_deref(), Some("cursor-2"));
    }

    #[test]
    fn dedupes_codex_model_catalog_preserving_order() {
        let models = dedupe_model_catalog(models(&["gpt-5.4", "gpt-5.4-mini", "gpt-5.4"]));

        assert_eq!(model_values(&models), vec!["gpt-5.4", "gpt-5.4-mini"]);
    }
}
