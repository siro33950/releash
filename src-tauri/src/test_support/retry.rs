use crate::adaptor::gateway::failure_records::FailureRecordStore;
use crate::common::retry::RetryLimiter;
use crate::usecase::failure::FailureRecordingUsecase;
use crate::usecase::retry::Retrying;
use std::sync::{Arc, OnceLock};

impl Retrying {
    pub(crate) fn records(
        &self,
        target: &str,
    ) -> Vec<crate::adaptor::gateway::failure_records::FailureRecordObservation> {
        self.test_query
            .as_ref()
            .expect("test failure store")
            .records(target)
    }
}

pub(crate) fn test_retrying() -> Arc<Retrying> {
    let store = Arc::new(FailureRecordStore::default());
    Arc::new(Retrying {
        limiter: Arc::new(RetryLimiter::deterministic()),
        failures: Arc::new(FailureRecordingUsecase::new(store.clone(), None)),
        test_query: Some(store),
    })
}

pub(crate) fn shared() -> &'static Arc<Retrying> {
    static SHARED: OnceLock<Arc<Retrying>> = OnceLock::new();
    SHARED.get_or_init(test_retrying)
}
