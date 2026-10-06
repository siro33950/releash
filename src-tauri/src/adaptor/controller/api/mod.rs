pub(crate) mod auth;
pub(crate) mod client;
mod client_admission;
pub(crate) mod client_priority;
pub(crate) mod client_stream;
pub(crate) use client::{ClientApiDeps, StateSubscriptionDeps};
pub(crate) mod error;
pub(crate) mod protocol;
pub(crate) mod provider_lifecycle;

pub(crate) mod workflow;

use std::sync::Arc;

use axum::middleware;
use axum::response::IntoResponse;
use axum::Router;

use crate::usecase::workflow::{WorkflowReadUsecase, WorkflowRuntimeUsecase};

#[derive(Clone)]
struct LocalApiState {
    workflow: Arc<WorkflowReadUsecase>,
    runtime: Arc<WorkflowRuntimeUsecase>,
}

pub fn build_router(
    workflow: Arc<WorkflowReadUsecase>,
    runtime: Arc<WorkflowRuntimeUsecase>,
    token: Arc<str>,
    terminal_token: impl Into<crate::infrastructure::local_api::ClientBearerToken>,
    client: Option<ClientApiDeps>,
    provider_lifecycle: Option<
        Arc<dyn crate::usecase::provider_lifecycle::ProviderLifecycleIngressPort>,
    >,
    (priority, default_timeout): (
        Arc<crate::common::priority::PriorityGate>,
        std::time::Duration,
    ),
) -> Router {
    let priority = Arc::new(priority.with_classifier(local_priority_level));
    let ingress = LocalIngress {
        priority,
        default_timeout,
    };
    let state = LocalApiState { workflow, runtime };
    let application_router = workflow::router()
        .fallback(|| async {
            error::ApiError::not_found("local API endpoint was not found").into_response()
        })
        .with_state(state.clone());
    let terminal_router = client::router(client, default_timeout).layer(
        middleware::from_fn_with_state(terminal_token.into(), auth::require_client),
    );
    authenticated(
        application_router
            .merge(provider_lifecycle::router(provider_lifecycle))
            .layer(middleware::from_fn_with_state(ingress, local_ingress)),
        token,
    )
    .merge(terminal_router)
}

pub fn local_priority_level(path: &str) -> Option<&'static str> {
    Some(
        if path == "/v1/provider-lifecycle/signals"
            || path.ends_with("/submit")
            || path.ends_with("/artifacts:validate")
            || path.contains("/artifacts/")
        {
            "workflow"
        } else {
            "default"
        },
    )
}

#[derive(Clone)]
pub struct LocalIngress {
    priority: Arc<crate::common::priority::PriorityGate>,
    default_timeout: std::time::Duration,
}

pub async fn local_ingress(
    axum::extract::State(ingress): axum::extract::State<LocalIngress>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let deadline = std::time::Instant::now() + ingress.default_timeout;
    let operation = ingress.priority.run(
        request,
        |request| request.uri().path(),
        Some(deadline),
        |request| async move { Ok(next.run(request).await) },
        error::ApiError::from,
    );
    match crate::common::operation_context::ingress(Some(deadline), async {
        crate::common::operation_context::wait(
            &crate::common::operation_context::current(),
            operation,
        )
        .await
    })
    .await
    {
        Ok(Ok(Ok(response))) => response,
        Ok(Ok(Err(error))) => error.into_response(),
        Ok(Err(stopped)) | Err(stopped) => {
            error::ApiError::from(crate::domain::failure::TechnicalFailure::from(stopped))
                .into_response()
        }
    }
}

pub(crate) fn authenticated(router: Router, token: Arc<str>) -> Router {
    router.layer(middleware::from_fn_with_state(token, auth::require_bearer))
}

#[cfg(test)]
#[path = "mod_test.rs"]
mod mod_tests;

#[cfg(feature = "test-support")]
impl LocalIngress {
    pub fn test_new(
        priority: Arc<crate::common::priority::PriorityGate>,
        default_timeout: std::time::Duration,
    ) -> Self {
        Self {
            priority,
            default_timeout,
        }
    }
}
