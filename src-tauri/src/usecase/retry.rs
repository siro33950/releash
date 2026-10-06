use crate::common::retry::{attempts, AttemptProgress, RetryBackoff, RetryLimiter};
use crate::usecase::failure::{next_attempt, FailureKey, FailureRecordingUsecase, RetryFailure};
use std::future::Future;
use std::sync::Arc;

pub struct Retrying {
    pub limiter: Arc<RetryLimiter>,
    pub failures: Arc<FailureRecordingUsecase>,
}

impl Retrying {
    pub(crate) fn new(
        limiter: Arc<RetryLimiter>,
        failures: Arc<FailureRecordingUsecase>,
    ) -> Arc<Self> {
        Arc::new(Self { limiter, failures })
    }

    pub(crate) async fn restart<T, E, F, Fut>(
        &self,
        key: impl Into<Option<FailureKey>>,
        policy: RetryBackoff,
        operation: F,
    ) -> Result<T, E>
    where
        E: RetryFailure,
        F: FnMut(AttemptProgress) -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        let key = key.into();
        let result = attempts(
            policy,
            &self.limiter,
            |error: &E| {
                let failure = error.work_failure();
                let progress = next_attempt(failure.kind);
                if let Some(key) = &key {
                    self.failures.observed(key, failure);
                } else {
                    log::warn!("Operation failed: {}", failure.message);
                }
                progress
            },
            operation,
        )
        .await;
        if result.is_ok() {
            if let Some(key) = &key {
                self.failures.resolved(key);
            }
        }
        result
    }

    pub(crate) async fn stage<T, E, F, Fut>(
        &self,
        key: impl Into<Option<FailureKey>>,
        policy: RetryBackoff,
        operation: F,
    ) -> Result<T, E>
    where
        E: RetryFailure,
        F: FnMut(AttemptProgress) -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        let key = key.into();
        let result = attempts(
            policy,
            &self.limiter,
            |error: &E| {
                let failure = error.work_failure();
                if next_attempt(failure.kind) != Some(AttemptProgress::Continue) {
                    if key.is_none() {
                        log::warn!("Operation failed: {}", failure.message);
                    }
                    return None;
                }
                if let Some(key) = &key {
                    self.failures.observed(key, failure);
                } else {
                    log::warn!("Operation failed: {}", failure.message);
                }
                Some(AttemptProgress::Continue)
            },
            operation,
        )
        .await;
        if result.is_ok() {
            if let Some(key) = &key {
                self.failures.resolved(key);
            }
        }
        result
    }
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) use crate::test_support::retry::shared;
#[cfg(test)]
pub(crate) use crate::test_support::retry::test_retrying;

#[cfg(test)]
#[path = "retry_test.rs"]
mod retry_tests;
