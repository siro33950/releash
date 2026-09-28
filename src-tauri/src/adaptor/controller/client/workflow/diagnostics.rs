use crate::adaptor::presenter::error::AppError;
use std::collections::HashMap;

use crate::adaptor::controller::state::AppState;

pub(crate) async fn render_facet_preview_shared(
    state: &AppState,
    content: String,
    sample_values: HashMap<String, String>,
) -> Result<String, AppError> {
    Ok(state
        .workflow_usecase
        .render_facet_preview(&content, &sample_values))
}
