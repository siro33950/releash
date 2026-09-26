use crate::usecase::terminal_surface::application::TerminalSurfaceApplication;
use connectrpc::ConnectError;
use std::sync::Arc;
#[derive(Clone)]
pub(crate) struct TerminalApiDeps {
    pub application: Arc<TerminalSurfaceApplication>,
}
impl TerminalApiDeps {
    pub(crate) fn new(application: Arc<TerminalSurfaceApplication>) -> Self {
        Self { application }
    }
}
pub(super) fn validate_identifier(id: &str) -> Result<(), ConnectError> {
    if id.len() > 128 {
        return Err(crate::adaptor::presenter::connect::classified_error(
            crate::adaptor::presenter::error::AppError::invalid_request(
                "Identifier exceeds 128 bytes",
            ),
        ));
    }
    Ok(())
}
