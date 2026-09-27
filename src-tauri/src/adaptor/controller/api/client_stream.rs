use crate::usecase::terminal_surface::application::TerminalSurfaceApplication;
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
