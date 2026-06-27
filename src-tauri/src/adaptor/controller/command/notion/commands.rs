use tauri::State;

use crate::adaptor::controller::state::AppState;
use crate::adaptor::protocol::notion::{
    NotionLabelOptionView, NotionRepoConfigView, NotionTaskPageView, NotionTaskQueryInput,
    NotionValidationResultView, PropertyMappingView,
};

fn map_join_error(error: tokio::task::JoinError) -> String {
    format!("task join error: {error}")
}

#[tauri::command]
pub(crate) async fn query_notion_tasks(
    state: State<'_, AppState>,
    repo_path: String,
    query: NotionTaskQueryInput,
) -> Result<NotionTaskPageView, String> {
    let notion_usecase = state.notion_usecase.clone();
    let query = query.into();
    tokio::task::spawn_blocking(move || {
        notion_usecase
            .query_tasks(&repo_path, &query)
            .map(Into::into)
    })
    .await
    .map_err(map_join_error)?
}

#[tauri::command]
pub(crate) async fn fetch_notion_label_options(
    state: State<'_, AppState>,
    repo_path: String,
) -> Result<Vec<NotionLabelOptionView>, String> {
    let notion_usecase = state.notion_usecase.clone();
    tokio::task::spawn_blocking(move || {
        notion_usecase
            .fetch_label_options(&repo_path)
            .map(|options| options.into_iter().map(Into::into).collect())
    })
    .await
    .map_err(map_join_error)?
}

#[tauri::command]
pub(crate) async fn save_notion_config(
    state: State<'_, AppState>,
    repo_path: String,
    api_token: String,
    database_id: String,
    property_mapping: PropertyMappingView,
) -> Result<(), String> {
    let notion_usecase = state.notion_usecase.clone();
    let config = NotionRepoConfigView {
        api_token,
        database_id,
        property_mapping,
    }
    .into();
    tokio::task::spawn_blocking(move || notion_usecase.save_config(repo_path, config))
        .await
        .map_err(map_join_error)?
}

#[tauri::command]
pub(crate) fn get_notion_config(
    state: State<'_, AppState>,
    repo_path: String,
) -> Result<Option<NotionRepoConfigView>, String> {
    state
        .notion_usecase
        .get_config(&repo_path)
        .map(|config| config.map(Into::into))
}

#[tauri::command]
pub(crate) async fn delete_notion_config(
    state: State<'_, AppState>,
    repo_path: String,
) -> Result<(), String> {
    let notion_usecase = state.notion_usecase.clone();
    tokio::task::spawn_blocking(move || notion_usecase.delete_config(&repo_path))
        .await
        .map_err(map_join_error)?
}

#[tauri::command]
pub(crate) async fn validate_notion_config(
    state: State<'_, AppState>,
    api_token: String,
    database_id: String,
) -> Result<NotionValidationResultView, String> {
    let notion_usecase = state.notion_usecase.clone();
    tokio::task::spawn_blocking(move || {
        let result = notion_usecase.validate_config(api_token, database_id);
        Ok(result.into())
    })
    .await
    .map_err(map_join_error)?
}
