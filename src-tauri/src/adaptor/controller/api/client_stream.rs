use crate::usecase::terminal_surface::application::TerminalSurfaceApplication;
use std::sync::Arc;

pub(crate) fn valid_subscription_id(id: &str, allow_empty: bool) -> bool {
    id.len() <= crate::common::SUBSCRIPTION_ID_MAX_BYTES && (allow_empty || !id.is_empty())
}

#[cfg(test)]
#[path = "client_stream_test.rs"]
mod client_stream_tests;
#[derive(Clone)]
pub(crate) struct TerminalApiDeps {
    pub application: Arc<TerminalSurfaceApplication>,
}
impl TerminalApiDeps {
    pub(crate) fn new(application: Arc<TerminalSurfaceApplication>) -> Self {
        Self { application }
    }
}
