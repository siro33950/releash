use std::sync::Arc;

use crate::domain::app_config::AppConfigRepository;

pub(crate) async fn update_external_editor_shared(
    state: &Arc<dyn AppConfigRepository>,
    publisher: Option<crate::usecase::state_subscription::StateSubscriptionUsecase>,
    editor: String,
    config_mutation: Arc<parking_lot::Mutex<()>>,
) -> Result<(), crate::adaptor::presenter::error::AppError> {
    let repository = state.clone();
    crate::common::operation_context::spawn_blocking(move || {
        crate::usecase::external_editor::open_usecase::update_external_editor(
            repository.as_ref(),
            publisher.as_ref(),
            editor,
            config_mutation.as_ref(),
        )
    })
    .await
    .map_err(|e| {
        crate::adaptor::presenter::error::AppError::from_failure(
            crate::domain::failure::TechnicalFailure::from(e),
        )
    })?
    .map_err(crate::adaptor::presenter::error::AppError::from_failure)
}
