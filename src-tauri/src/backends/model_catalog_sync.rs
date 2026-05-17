use std::sync::Arc;

use tauri::{Emitter, Manager};
use tokio::sync::Mutex;

use crate::backends::bridge_common::AgentProcessMap;
use crate::backends::ModelInfo;

/// `supported_models` メッセージを受信した際の共通処理。
///
/// 1. 生 ID を抽出（all-or-nothing 検証）
/// 2. registry 経由で config.toml に書き込み（検証失敗時は config を変更しない）
/// 3. 最新の available_models（config 由来）を返す
///
/// emit や proc キャッシュ同期はこの関数の責務ではない。呼び出し側で行う。
/// `None` を返した場合は呼び出し側で proc キャッシュ・emit 共に上書きせず、
/// 「現在の状態を維持」として扱う（空配列で上書きしない）。
pub(crate) fn apply_supported_models_to_config(
    registry: Option<&crate::backends::AgentBackendRegistry>,
    backend_id: &str,
    msg: &serde_json::Value,
) -> Option<Vec<ModelInfo>> {
    let registry = registry?;
    match extract_model_ids_from_supported_models(msg) {
        Ok(raw_ids) if raw_ids.is_empty() => {
            log::warn!("{}", supported_models_empty_payload_log(backend_id));
            fallback_models_from_config(registry, backend_id)
        }
        Ok(raw_ids) => match registry.write_models_to_config(backend_id, raw_ids) {
            Ok(stored) => Some(
                stored
                    .into_iter()
                    .map(|value| ModelInfo { value })
                    .collect(),
            ),
            Err(e) => {
                log::warn!("{}", supported_models_write_failed_log(backend_id, &e));
                fallback_models_from_config(registry, backend_id)
            }
        },
        Err(e) => {
            log::warn!("{}", supported_models_invalid_payload_log(backend_id, &e));
            fallback_models_from_config(registry, backend_id)
        }
    }
}

pub(crate) fn supported_models_empty_payload_log(backend_id: &str) -> String {
    format!(
        "supported_models 受信内容のモデル配列が空のため '{backend_id}' のモデル一覧は更新しません"
    )
}

pub(crate) fn supported_models_write_failed_log(backend_id: &str, error: &str) -> String {
    format!("supported_models から '{backend_id}' のモデル一覧を反映できませんでした: {error}")
}

pub(crate) fn supported_models_invalid_payload_log(backend_id: &str, error: &str) -> String {
    format!(
        "supported_models 受信内容が不正のため '{backend_id}' のモデル一覧は更新しません: {error}"
    )
}

pub(crate) fn supported_models_fallback_read_failed_log(backend_id: &str, error: &str) -> String {
    format!(
        "supported_models フォールバック読み出し失敗 (backend='{backend_id}'): {error} - proc キャッシュと emit は維持"
    )
}

pub(crate) fn startup_model_refresh_failed_log(backend_id: &str, error: &str) -> String {
    format!("backend '{backend_id}' model refresh failed: {error}")
}

fn fallback_models_from_config(
    registry: &crate::backends::AgentBackendRegistry,
    backend_id: &str,
) -> Option<Vec<ModelInfo>> {
    match registry.available_models(backend_id) {
        Ok(models) => Some(models),
        Err(e) => {
            log::warn!(
                "{}",
                supported_models_fallback_read_failed_log(backend_id, &e)
            );
            None
        }
    }
}

/// `supported_models` メッセージ受信時の完全処理。
pub(crate) async fn handle_supported_models_message(
    app: Option<&tauri::AppHandle>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    registry: Option<&crate::backends::AgentBackendRegistry>,
    chat_session_id: &str,
    msg: &serde_json::Value,
) {
    let (selected_model, backend_id) = {
        let map = handles.lock().await;
        match map.get(chat_session_id) {
            Some(proc) => (proc.selected_model.clone(), proc.backend_id.clone()),
            None => {
                log::warn!(
                    "supported_models 受信時に session '{chat_session_id}' の active process が見つからないため何もしません"
                );
                return;
            }
        }
    };

    let Some(models) = apply_supported_models_to_config(registry, &backend_id, msg) else {
        return;
    };

    {
        let mut map = handles.lock().await;
        if let Some(proc) = map.get_mut(chat_session_id) {
            proc.available_models = models.clone();
        }
    }

    if let Some(app) = app {
        let _ = app.emit(
            "agent-models-updated",
            build_agent_models_updated_payload(chat_session_id, &models, selected_model.as_deref()),
        );
        let _ = app.emit(
            "agent-backend-models-updated",
            build_agent_backend_models_updated_payload(&backend_id, &models),
        );
        notify_backend_models_updated(app, &backend_id, &models);
    }
}

pub(crate) fn build_agent_models_updated_payload(
    chat_session_id: &str,
    available_models: &[ModelInfo],
    selected_model: Option<&str>,
) -> serde_json::Value {
    serde_json::json!({
        "chat_session_id": chat_session_id,
        "available_models": available_models,
        "selected_model": selected_model,
    })
}

pub(crate) fn extract_model_ids_from_supported_models(
    value: &serde_json::Value,
) -> Result<Vec<String>, String> {
    let arr = value
        .get("models")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "supported_models に 'models' 配列が含まれていません".to_string())?;

    let mut result = Vec::with_capacity(arr.len());
    for (idx, m) in arr.iter().enumerate() {
        let id = m
            .get("value")
            .and_then(|v| v.as_str())
            .or_else(|| m.get("id").and_then(|v| v.as_str()))
            .ok_or_else(|| {
                format!("supported_models models[{idx}] に有効な value/id がありません")
            })?;
        result.push(id.to_string());
    }
    Ok(result)
}

pub async fn refresh_models_for_backend_and_propagate(
    app: tauri::AppHandle,
    handles: Arc<Mutex<AgentProcessMap>>,
    registry: Arc<crate::backends::AgentBackendRegistry>,
    backend_id: String,
) {
    match registry.refresh_models_to_config_for(&backend_id).await {
        Ok(Some(models)) => {
            log::info!(
                "backend '{backend_id}' models refreshed ({} entries)",
                models.len()
            );
            propagate_refreshed_models_to_active_sessions(&app, &handles, &backend_id, &models)
                .await;
        }
        Ok(None) => {
            log::info!("backend '{backend_id}' does not support startup CLI model fetch; skipped");
        }
        Err(e) => {
            log::warn!("{}", startup_model_refresh_failed_log(&backend_id, &e));
        }
    }
}

pub async fn propagate_refreshed_models_to_active_sessions(
    app: &tauri::AppHandle,
    handles: &Arc<Mutex<AgentProcessMap>>,
    backend_id: &str,
    models: &[String],
) {
    let model_infos: Vec<ModelInfo> = models
        .iter()
        .map(|v| ModelInfo { value: v.clone() })
        .collect();

    sync_active_processes_for_backend(handles, backend_id, &model_infos).await;

    let _ = app.emit(
        "agent-backend-models-updated",
        build_agent_backend_models_updated_payload(backend_id, &model_infos),
    );
    notify_backend_models_updated(app, backend_id, &model_infos);
}

fn notify_backend_models_updated(
    app: &tauri::AppHandle,
    backend_id: &str,
    available_models: &[ModelInfo],
) {
    if let Some(notifier) =
        app.try_state::<crate::usecase::backend_models::BackendModelsUpdateNotifierState>()
    {
        let values: Vec<String> = available_models
            .iter()
            .map(|model| model.value.clone())
            .collect();
        notifier.broadcast_backend_models_updated(backend_id, &values);
    }
}

pub(crate) fn build_agent_backend_models_updated_payload(
    backend_id: &str,
    available_models: &[ModelInfo],
) -> serde_json::Value {
    serde_json::json!({
        "backend_id": backend_id,
        "available_models": available_models,
    })
}

pub(crate) async fn sync_active_processes_for_backend(
    handles: &Arc<Mutex<AgentProcessMap>>,
    backend_id: &str,
    model_infos: &[ModelInfo],
) {
    let mut map = handles.lock().await;
    for proc in map.values_mut() {
        if proc.backend_id == backend_id {
            proc.available_models = model_infos.to_vec();
        }
    }
}
