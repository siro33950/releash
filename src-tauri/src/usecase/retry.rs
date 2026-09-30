use crate::common::retry::{attempts, AttemptProgress, RetryBackoff, RetryLimiter};
use crate::usecase::failure::{next_attempt, FailureKey, FailureRecordingUsecase, RetryFailure};
use std::future::Future;
use std::sync::Arc;

pub struct Retrying {
    pub(crate) limiter: Arc<RetryLimiter>,
    pub(crate) failures: Arc<FailureRecordingUsecase>,
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
        key: FailureKey,
        policy: RetryBackoff,
        operation: F,
    ) -> Result<T, E>
    where
        E: RetryFailure,
        F: FnMut(AttemptProgress) -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        let result = attempts(
            policy,
            &self.limiter,
            |error: &E| {
                let failure = error.work_failure();
                let progress = next_attempt(failure.kind);
                self.failures.observed(&key, failure);
                progress
            },
            operation,
        )
        .await;
        if result.is_ok() {
            self.failures.resolved(&key);
        }
        result
    }

    pub(crate) async fn stage<T, E, F, Fut>(
        &self,
        key: FailureKey,
        policy: RetryBackoff,
        operation: F,
    ) -> Result<T, E>
    where
        E: RetryFailure,
        F: FnMut(AttemptProgress) -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        let result = attempts(
            policy,
            &self.limiter,
            |error: &E| {
                let failure = error.work_failure();
                if next_attempt(failure.kind) != Some(AttemptProgress::Continue) {
                    return None;
                }
                self.failures.observed(&key, failure);
                Some(AttemptProgress::Continue)
            },
            operation,
        )
        .await;
        if result.is_ok() {
            self.failures.resolved(&key);
        }
        result
    }
}

#[cfg(test)]
pub(crate) use crate::test_support::retry::{shared, test_retrying};

#[cfg(test)]
#[path = "retry_test.rs"]
mod retry_tests;
