use std::sync::Arc;

use crate::adaptor::gateway::external_editor::EditorSettingsConfigGateway;
use crate::domain::app_config::ConfigRepository;

pub(crate) async fn update_external_editor_shared(
    state: &Arc<dyn ConfigRepository>,
    publisher: Option<crate::usecase::state_subscription::StateSubscriptionUsecase>,
    editor: String,
) -> Result<(), crate::adaptor::presenter::error::AppError> {
    let settings = EditorSettingsConfigGateway::new(state.clone());
    crate::common::operation_context::spawn_blocking(move || {
        crate::usecase::external_editor::open_usecase::update_external_editor(
            &settings,
            publisher.as_ref(),
            editor,
        )
    })
    .await
    .map_err(|e| crate::adaptor::presenter::error::AppError::new(format!("task join error: {e}")))?
    .map_err(crate::adaptor::presenter::error::AppError::from_failure)
}
