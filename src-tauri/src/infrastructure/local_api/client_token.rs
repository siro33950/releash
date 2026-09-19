use std::sync::Arc;
use subtle::ConstantTimeEq;

#[derive(Clone)]
pub(crate) struct ClientBearerToken(Arc<parking_lot::RwLock<Option<Arc<str>>>>);

impl From<Arc<str>> for ClientBearerToken {
    fn from(token: Arc<str>) -> Self {
        Self(Arc::new(parking_lot::RwLock::new(Some(token))))
    }
}

impl ClientBearerToken {
    pub(crate) fn accepts(&self, candidate: &str) -> bool {
        self.0
            .read()
            .as_ref()
            .is_some_and(|token| bool::from(candidate.as_bytes().ct_eq(token.as_bytes())))
    }
    #[cfg(any(test, all(debug_assertions, feature = "desktop")))]
    pub(crate) fn token(&self) -> Arc<str> {
        self.0
            .read()
            .as_ref()
            .expect("token requested before server shutdown")
            .clone()
    }
    pub(crate) fn revoke(&self) {
        self.0.write().take();
    }
}
