use std::sync::Arc;

use crate::adaptor::gateway::external_editor::{
    EditorSettingsConfigGateway, MacInstalledEditorGateway,
};
use crate::domain::app_config::ConfigRepository;

use crate::usecase::external_editor::dto::EditorInfoDto;

pub(crate) fn detect_editors_shared() -> Vec<EditorInfoDto> {
    crate::usecase::external_editor::detect_usecase::detect_editors(&MacInstalledEditorGateway)
        .into_iter()
        .map(Into::into)
        .collect()
}

pub(crate) fn get_external_editor_shared(
    state: &Arc<dyn ConfigRepository>,
) -> Result<String, crate::domain::external_editor::EditorError> {
    crate::usecase::external_editor::open_usecase::get_external_editor(
        &EditorSettingsConfigGateway::new(state.clone()),
    )
}

pub(crate) async fn update_external_editor_shared(
    state: &Arc<dyn ConfigRepository>,
    editor: String,
) -> Result<(), crate::other::AppError> {
    let settings = EditorSettingsConfigGateway::new(state.clone());
    tokio::task::spawn_blocking(move || {
        crate::usecase::external_editor::open_usecase::update_external_editor(&settings, editor)
    })
    .await
    .map_err(|e| crate::other::AppError::new(format!("task join error: {e}")))?
    .map_err(crate::other::AppError::from_failure)
}
