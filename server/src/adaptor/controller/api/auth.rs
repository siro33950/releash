use axum::extract::{Request, State};
use axum::http::header::AUTHORIZATION;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use super::error::ApiError;
#[derive(Clone)]
pub struct ClientTokens {
    pub operator: crate::infrastructure::local_api::BearerToken,
    pub hook: crate::infrastructure::local_api::BearerToken,
}
include!(concat!(env!("OUT_DIR"), "/client_scopes.rs"));

pub async fn require_client(
    State(tokens): State<ClientTokens>,
    request: Request,
    next: Next,
) -> Response {
    let candidate = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    let scope = candidate.and_then(|candidate| {
        if tokens.operator.accepts(candidate) {
            Some(crate::adaptor::presenter::client::Scope::Operator)
        } else if tokens.hook.accepts(candidate) {
            Some(crate::adaptor::presenter::client::Scope::Hook)
        } else {
            None
        }
    });
    match scope {
        None => ApiError::unauthorized().into_response(),
        Some(scope)
            if method_scopes(request.uri().path().rsplit('/').next().unwrap_or_default())
                .is_some_and(|scopes| !scopes.contains(&scope)) =>
        {
            crate::adaptor::presenter::connect::scope_denied(request.headers())
        }
        Some(_) => next.run(request).await,
    }
}

#[cfg(test)]
#[path = "auth_test.rs"]
mod auth_tests;
