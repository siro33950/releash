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
        return Err(crate::adaptor::protocol::connect::classified_error(
            crate::other::AppError::new("Identifier exceeds 128 bytes")
                .with_failure_kind(crate::domain::failure::FailureKind::InvalidInput),
        ));
    }
    Ok(())
}
