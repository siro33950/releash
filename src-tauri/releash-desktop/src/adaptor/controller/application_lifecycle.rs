use crate::domain::daemon_supervision::StopIntent;

use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct ApplicationQuitIngress {
    handler: Arc<dyn Fn(StopIntent) + Send + Sync>,
}

impl ApplicationQuitIngress {
    pub(crate) fn new(handler: impl Fn(StopIntent) + Send + Sync + 'static) -> Self {
        Self {
            handler: Arc::new(handler),
        }
    }

    pub(crate) fn request(&self, intent: StopIntent) {
        (self.handler)(intent);
    }
}
