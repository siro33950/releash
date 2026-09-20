#[cfg(feature = "desktop")]
use crate::domain::application_lifecycle::ApplicationQuitIntent;
#[cfg(feature = "desktop")]
use std::sync::Arc;

#[cfg(feature = "desktop")]
#[derive(Clone)]
pub(crate) struct ApplicationQuitIngress {
    handler: Arc<dyn Fn(ApplicationQuitIntent) + Send + Sync>,
}

#[cfg(feature = "desktop")]
impl ApplicationQuitIngress {
    pub(crate) fn new(handler: impl Fn(ApplicationQuitIntent) + Send + Sync + 'static) -> Self {
        Self {
            handler: Arc::new(handler),
        }
    }

    pub(crate) fn request(&self, intent: ApplicationQuitIntent) {
        (self.handler)(intent);
    }
}
