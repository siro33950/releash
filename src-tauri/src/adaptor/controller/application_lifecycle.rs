#[cfg(feature = "desktop")]
use crate::domain::daemon_supervision::StopIntent;
#[cfg(feature = "desktop")]
use std::sync::Arc;

#[cfg(feature = "desktop")]
#[derive(Clone)]
pub(crate) struct ApplicationQuitIngress {
    handler: Arc<dyn Fn(StopIntent) + Send + Sync>,
}

#[cfg(feature = "desktop")]
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
