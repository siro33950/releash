use crate::adaptor::controller::state::AppState;
use crate::adaptor::presenter::error::AppError;
use crate::adaptor::presenter::notion::{
    NotionRepoConfigView, NotionValidationResultView, PropertyMappingView,
};
use crate::usecase::notion::error::NotionUsecaseError;

fn map_join_error(error: tokio::task::JoinError) -> AppError {
    AppError::from_failure(crate::domain::failure::TechnicalFailure::from(error))
}

fn map_usecase_error(error: NotionUsecaseError) -> AppError {
    AppError::from_failure(error)
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
    state
        .notion_usecase
        .validate_config(api_token, database_id)
        .await
        .map(Into::into)
        .map_err(AppError::from_failure)
}
