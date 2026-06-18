use crate::adaptor::controller::state::AppState;
use crate::domain::workflow::{FacetKind, FacetSummary};

fn parse_domain_facet_kind(kind: &str) -> Result<FacetKind, String> {
    match kind {
        "policy" => Ok(FacetKind::Policy),
        "knowledge" => Ok(FacetKind::Knowledge),
        "instruction" => Ok(FacetKind::Instruction),
        "contract" => Ok(FacetKind::Contract),
        _ => Err(format!("Unknown facet kind: {kind}")),
    }
}

#[tauri::command]
pub async fn list_facets(
    state: tauri::State<'_, AppState>,
    kind: String,
) -> Result<Vec<String>, String> {
    let kind = parse_domain_facet_kind(&kind)?;
    let query = state.workflow_usecase.clone();
    tokio::task::spawn_blocking(move || query.list_facets(kind).map_err(|e| e.to_string()))
        .await
        .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn get_facet(
    state: tauri::State<'_, AppState>,
    kind: String,
    key: String,
) -> Result<String, String> {
    let kind = parse_domain_facet_kind(&kind)?;
    let query = state.workflow_usecase.clone();
    tokio::task::spawn_blocking(move || query.get_facet(kind, &key).map_err(|e| e.to_string()))
        .await
        .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn save_facet(
    state: tauri::State<'_, AppState>,
    kind: String,
    key: String,
    content: String,
    is_new: Option<bool>,
) -> Result<(), String> {
    let kind = parse_domain_facet_kind(&kind)?;
    let is_new = is_new.unwrap_or(false);
    let usecase = state.workflow_usecase.clone();
    tokio::task::spawn_blocking(move || {
        usecase
            .save_facet(kind, &key, &content, is_new)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn delete_facet(
    state: tauri::State<'_, AppState>,
    kind: String,
    key: String,
) -> Result<(), String> {
    let kind = parse_domain_facet_kind(&kind)?;
    let usecase = state.workflow_usecase.clone();
    tokio::task::spawn_blocking(move || usecase.delete_facet(kind, &key).map_err(|e| e.to_string()))
        .await
        .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn list_facet_summaries(
    state: tauri::State<'_, AppState>,
    kind: String,
) -> Result<Vec<FacetSummary>, String> {
    let kind = parse_domain_facet_kind(&kind)?;
    let query = state.workflow_usecase.clone();
    tokio::task::spawn_blocking(move || query.list_facet_summaries(kind).map_err(|e| e.to_string()))
        .await
        .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn duplicate_facet(
    state: tauri::State<'_, AppState>,
    kind: String,
    source_key: String,
    new_key: String,
) -> Result<(), String> {
    let kind = parse_domain_facet_kind(&kind)?;
    let usecase = state.workflow_usecase.clone();
    tokio::task::spawn_blocking(move || {
        usecase
            .duplicate_facet(kind, &source_key, &new_key)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub fn open_facet_in_editor(
    state: tauri::State<'_, AppState>,
    kind: String,
    key: String,
) -> Result<(), String> {
    let kind = parse_domain_facet_kind(&kind)?;
    state
        .workflow_usecase
        .open_facet_in_editor(kind, &key)
        .map_err(|e| e.to_string())
}
