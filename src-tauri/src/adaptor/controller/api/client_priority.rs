use std::sync::{Arc, Mutex};

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
        let path = req.ctx.path().unwrap_or_default().to_owned();
        let deadline = req.ctx.deadline();
        self.gate
            .run(
                &path,
                deadline,
                || next.run(req),
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

impl PriorityEvents for PriorityFailureReporter {
    fn rejected(&self, path: &str, rejection: &Rejection) {
        let mut pending = self.pending.lock().unwrap();
        if let Some(failures) = &self.failures {
            failures.observed(
                &FailureKey::new("client_request_limit", "daemon"),
                WorkFailure {
                    kind: Failure::Technical(TechnicalFailureNature::Transient),
                    message: format!("{path}: {rejection}"),
                },
            );
            *pending = true;
        }
    }

    fn admitted(&self) {
        let mut pending = self.pending.lock().unwrap();
        if *pending {
            if let Some(failures) = &self.failures {
                failures.resolved(&FailureKey::new("client_request_limit", "daemon"));
            }
            *pending = false;
        }
    }
}

#[cfg(test)]
#[path = "client_priority_test.rs"]
mod client_priority_tests;
