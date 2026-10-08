pub(crate) mod auth;
pub(crate) mod client;
mod client_admission;
pub(crate) mod client_priority;
pub(crate) mod client_stream;
pub(crate) use client::{ClientApiDeps, StateSubscriptionDeps};
pub(crate) mod error;
mod provider_signal;

use axum::middleware;
use axum::Router;

pub fn build_router(
    tokens: auth::ClientTokens,
    client: Option<ClientApiDeps>,
    default_timeout: std::time::Duration,
) -> Router {
    client::router(client, default_timeout)
        .layer(middleware::from_fn_with_state(tokens, auth::require_client))
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) mod test_helpers;
