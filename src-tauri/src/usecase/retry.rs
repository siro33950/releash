use crate::common::retry::{attempts, AttemptProgress, RetryBackoff, RetryLimiter};
#[cfg(test)]
use crate::usecase::failure::FailureQueryService;
use crate::usecase::failure::{next_attempt, FailureKey, FailureRecordingUsecase, RetryFailure};
use std::future::Future;
use std::sync::Arc;

pub struct Retrying {
    pub(crate) limiter: Arc<RetryLimiter>,
    pub(crate) failures: Arc<FailureRecordingUsecase>,
    #[cfg(test)]
    pub(crate) test_query: Option<Arc<dyn FailureQueryService>>,
}

impl Retrying {
    pub(crate) fn new(
        limiter: Arc<RetryLimiter>,
        failures: Arc<FailureRecordingUsecase>,
    ) -> Arc<Self> {
        Arc::new(Self {
            limiter,
            failures,
            #[cfg(test)]
            test_query: None,
        })
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

    #[cfg(test)]
    pub(crate) async fn page_targets(
        &self,
        targets: &[String],
        offset: usize,
    ) -> crate::usecase::failure::FailurePage {
        self.test_query
            .as_ref()
            .expect("test failure store")
            .page(targets, offset)
            .await
    }

    #[cfg(test)]
    pub(crate) async fn page(
        &self,
        target: &str,
        offset: usize,
    ) -> crate::usecase::failure::FailurePage {
        self.page_targets(&[target.to_string()], offset).await
    }
}

#[cfg(test)]
pub(crate) use crate::test_support::retry::{shared, test_retrying};

#[cfg(test)]
#[path = "retry_test.rs"]
mod retry_tests;
