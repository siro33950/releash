use std::sync::Arc;

use crate::common::concurrency::Rejection;
use crate::common::priority::{PriorityEvents, PriorityGate};

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

pub(crate) struct PriorityFailureReporter;

impl PriorityEvents for PriorityFailureReporter {
    fn rejected(&self, path: &str, rejection: &Rejection) {
        log::warn!("{path}: {rejection}");
    }

    fn admitted(&self) {}
}

#[cfg(test)]
#[path = "client_priority_test.rs"]
mod client_priority_tests;
