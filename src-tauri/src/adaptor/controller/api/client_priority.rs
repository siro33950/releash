use parking_lot::Mutex;
use std::sync::Arc;

use crate::common::concurrency::Rejection;
use crate::common::priority::{PriorityEvents, PriorityGate};
use crate::domain::failure::TechnicalFailureNature;
use crate::usecase::failure::{Failure, FailureKey, FailureRecordingUsecase, WorkFailure};

#[derive(Clone)]
pub(crate) struct PriorityInterceptor {
    pub(crate) gate: Arc<PriorityGate>,
}

#[connectrpc::async_trait]
impl connectrpc::Interceptor for PriorityInterceptor {
    async fn intercept_unary(
        &self,
        req: connectrpc::interceptor::UnaryRequest,
        next: connectrpc::Next<'_>,
    ) -> Result<connectrpc::interceptor::UnaryResponse, connectrpc::ConnectError> {
        let deadline = req.ctx.deadline();
        self.gate
            .run(
                req,
                |req| {
                    req.ctx
                        .path()
                        .expect("dispatch sets path before interceptors run")
                },
                deadline,
                |req| next.run(req),
                |rejection| crate::adaptor::presenter::connect::request_rejected(&rejection),
            )
            .await
    }
}

pub(crate) struct PriorityFailureReporter {
    failures: Option<Arc<FailureRecordingUsecase>>,
    pending: Mutex<bool>,
}

impl PriorityFailureReporter {
    pub(crate) fn new(failures: Option<Arc<FailureRecordingUsecase>>) -> Self {
        Self {
            failures,
            pending: Mutex::new(false),
        }
    }
}

fn priority_failure_key() -> FailureKey {
    FailureKey::new("client_request_limit", "daemon")
}

impl PriorityEvents for PriorityFailureReporter {
    fn rejected(&self, path: &str, rejection: &Rejection) {
        let Some(failures) = &self.failures else {
            return;
        };
        let mut pending = self.pending.lock();
        failures.observed(
            &priority_failure_key(),
            WorkFailure {
                kind: Failure::Technical(TechnicalFailureNature::Transient),
                message: format!("{path}: {rejection}"),
            },
        );
        *pending = true;
    }

    fn admitted(&self) {
        let Some(failures) = &self.failures else {
            return;
        };
        let mut pending = self.pending.lock();
        if *pending {
            failures.resolved(&priority_failure_key());
            *pending = false;
        }
    }
}

#[cfg(test)]
#[path = "client_priority_test.rs"]
mod client_priority_tests;
