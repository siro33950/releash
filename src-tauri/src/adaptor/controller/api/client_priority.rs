use std::sync::Arc;

use crate::common::concurrency::PriorityLimits;
use crate::usecase::failure::{Failure, FailureKey, FailureOutput, WorkFailure};

pub(crate) const TOTAL_SEATS: usize = 64;
pub(crate) const QUEUE_LENGTH: usize = 50;
pub(crate) const SHARES: &[(&str, u32)] =
    &[("interactive", 30), ("workflow", 40), ("default", 120)];

pub(crate) fn limits() -> PriorityLimits {
    PriorityLimits::new(TOTAL_SEATS, SHARES, QUEUE_LENGTH)
}

pub(crate) fn priority_level(path: &str) -> Option<&'static str> {
    match path.rsplit('/').next().unwrap_or_default() {
        "GetServerInfo" | "ReportTerminalProcessed" => None,
        "WriteTerminalSurface"
        | "WritePathsToTerminalSurface"
        | "ResizeTerminalSurface"
        | "StartStateSubscription"
        | "StopStateSubscription"
        | "WatchFiles"
        | "WatchGitDirectory"
        | "StopWatching" => Some("interactive"),
        "StartWorkflow"
        | "AbortWorkflow"
        | "ApproveWorkspaceNode"
        | "ApproveWorkflowNode"
        | "RetryWorkspaceNode"
        | "ResumeWorkspaceSessionNode"
        | "WorkflowSubmitOutput"
        | "WorkflowValidateOutput"
        | "WorkflowGetOutput" => Some("workflow"),
        _ => Some("default"),
    }
}

pub(crate) struct PriorityInterceptor {
    pub(crate) limits: Arc<PriorityLimits>,
    pub(crate) failures: Option<Arc<dyn FailureOutput>>,
}

#[connectrpc::async_trait]
impl connectrpc::Interceptor for PriorityInterceptor {
    async fn intercept_unary(
        &self,
        req: connectrpc::interceptor::UnaryRequest,
        next: connectrpc::Next<'_>,
    ) -> Result<connectrpc::interceptor::UnaryResponse, connectrpc::ConnectError> {
        let path = req.ctx.path().unwrap_or_default();
        let Some(level) = priority_level(path) else {
            return next.run(req).await;
        };
        let _seat = match self.limits.admit(level, req.ctx.deadline()).await {
            Ok(seat) => seat,
            Err(rejection) => {
                if let Some(failures) = &self.failures {
                    failures.observed(
                        &FailureKey::new("client_request_limit", "daemon"),
                        WorkFailure {
                            kind: Failure::Technical(
                                crate::domain::failure::TechnicalFailureNature::Transient,
                            ),
                            message: format!("{path}: {rejection}"),
                        },
                    );
                }
                return Err(crate::adaptor::presenter::connect::request_rejected(
                    &rejection,
                ));
            }
        };
        next.run(req).await
    }
}
