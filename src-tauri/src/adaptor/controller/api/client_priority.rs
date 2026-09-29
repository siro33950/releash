use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
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
        let path = req
            .ctx
            .spec()
            .expect("generated ClientService methods have a spec")
            .procedure;
        let deadline = req.ctx.deadline();
        self.gate
            .run(
                path,
                deadline,
                || next.run(req),
                |rejection| crate::adaptor::presenter::connect::request_rejected(&rejection),
            )
            .await
    }
}

const IDLE: u8 = 0;
const OBSERVING: u8 = 1;
const OBSERVING_ADMITTED: u8 = 2;
const PENDING: u8 = 3;
const RESOLVING: u8 = 4;

pub(crate) struct PriorityFailureReporter {
    failures: Option<Arc<FailureRecordingUsecase>>,
    pending: AtomicBool,
    state: AtomicU8,
}

impl PriorityFailureReporter {
    pub(crate) fn new(failures: Option<Arc<FailureRecordingUsecase>>) -> Self {
        Self {
            failures,
            pending: AtomicBool::new(false),
            state: AtomicU8::new(IDLE),
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
        loop {
            let state = self.state.load(Ordering::SeqCst);
            if (state == IDLE || state == PENDING)
                && self
                    .state
                    .compare_exchange(state, OBSERVING, Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok()
            {
                break;
            }
            std::thread::yield_now();
        }
        failures.observed(
            &priority_failure_key(),
            WorkFailure {
                kind: Failure::Technical(TechnicalFailureNature::Transient),
                message: format!("{path}: {rejection}"),
            },
        );
        self.pending.store(true, Ordering::SeqCst);
        if self
            .state
            .compare_exchange(OBSERVING, PENDING, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            self.state.store(RESOLVING, Ordering::SeqCst);
            if self.pending.swap(false, Ordering::SeqCst) {
                failures.resolved(&priority_failure_key());
            }
            self.state.store(IDLE, Ordering::SeqCst);
        }
    }

    fn admitted(&self) {
        let Some(failures) = &self.failures else {
            return;
        };
        loop {
            match self.state.load(Ordering::SeqCst) {
                IDLE | OBSERVING_ADMITTED | RESOLVING => return,
                OBSERVING => {
                    if self
                        .state
                        .compare_exchange(
                            OBSERVING,
                            OBSERVING_ADMITTED,
                            Ordering::SeqCst,
                            Ordering::SeqCst,
                        )
                        .is_ok()
                    {
                        return;
                    }
                }
                PENDING => {
                    if self
                        .state
                        .compare_exchange(PENDING, RESOLVING, Ordering::SeqCst, Ordering::SeqCst)
                        .is_ok()
                    {
                        if self.pending.swap(false, Ordering::SeqCst) {
                            failures.resolved(&priority_failure_key());
                        }
                        self.state.store(IDLE, Ordering::SeqCst);
                        return;
                    }
                }
                _ => unreachable!(),
            }
        }
    }
}

#[cfg(test)]
#[path = "client_priority_test.rs"]
mod client_priority_tests;
