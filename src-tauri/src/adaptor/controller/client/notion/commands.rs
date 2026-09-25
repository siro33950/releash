use crate::adaptor::controller::state::AppState;
use crate::adaptor::protocol::notion::{
    NotionLabelOptionView, NotionRepoConfigView, NotionTaskPageView, NotionTaskQueryInput,
    NotionValidationResultView, PropertyMappingView,
};
use crate::other::AppError;
use crate::usecase::notion::error::NotionUsecaseError;

fn map_join_error(error: tokio::task::JoinError) -> AppError {
    AppError::new(format!("task join error: {error}"))
}

fn map_usecase_error(error: NotionUsecaseError) -> AppError {
    AppError::from_failure(error)
}

pub(crate) async fn query_notion_tasks_shared(
    state: &AppState,
    repo_path: String,
    query: NotionTaskQueryInput,
) -> Result<NotionTaskPageView, AppError> {
    let notion_usecase = state.notion_usecase.clone();
    let query = query.into();
    crate::other::operation_context::spawn_blocking(move || {
        notion_usecase
            .query_tasks(&repo_path, &query)
            .map(Into::into)
    })
    .await
    .map_err(map_join_error)?
    .map_err(map_usecase_error)
}

pub(crate) async fn fetch_notion_label_options_shared(
    state: &AppState,
    repo_path: String,
) -> Result<Vec<NotionLabelOptionView>, AppError> {
    let notion_usecase = state.notion_usecase.clone();
    crate::other::operation_context::spawn_blocking(move || {
        notion_usecase
            .fetch_label_options(&repo_path)
            .map(|options| options.into_iter().map(Into::into).collect())
    })
    .await
    .map_err(map_join_error)?
    .map_err(map_usecase_error)
}

pub(crate) async fn save_notion_config_shared(
    state: &AppState,
    repo_path: String,
    api_token: String,
    database_id: String,
    property_mapping: PropertyMappingView,
) -> Result<(), AppError> {
    let notion_usecase = state.notion_usecase.clone();
    let config = NotionRepoConfigView {
        api_token,
        database_id,
        property_mapping,
    }
    .into();
    crate::adaptor::controller::client::worktree_mutation::spawn_blocking(move || {
        notion_usecase.save_config(repo_path, config)
    })
    .await
    .map_err(map_join_error)?
    .map_err(map_usecase_error)
}

pub(crate) fn get_notion_config_shared(
    state: &AppState,
    repo_path: String,
) -> Result<Option<NotionRepoConfigView>, AppError> {
    state
        .notion_usecase
        .get_config(&repo_path)
        .map(|config| config.map(Into::into))
        .map_err(map_usecase_error)
}

pub(crate) async fn delete_notion_config_shared(
    state: &AppState,
    repo_path: String,
) -> Result<(), AppError> {
    let notion_usecase = state.notion_usecase.clone();
    crate::adaptor::controller::client::worktree_mutation::spawn_blocking(move || {
        notion_usecase.delete_config(&repo_path)
    })
    .await
    .map_err(map_join_error)?
    .map_err(map_usecase_error)
}

pub(crate) async fn validate_notion_config_shared(
    state: &AppState,
    api_token: String,
    database_id: String,
) -> Result<NotionValidationResultView, AppError> {
    let notion_usecase = state.notion_usecase.clone();
    crate::other::operation_context::spawn_blocking(move || {
        let result = notion_usecase
            .validate_config(api_token, database_id)
            .map_err(AppError::from_failure)?;
        Ok(result.into())
    })
    .await
    .map_err(map_join_error)?
}
