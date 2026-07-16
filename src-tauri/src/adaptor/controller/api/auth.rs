use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::header::AUTHORIZATION;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use subtle::ConstantTimeEq;

use super::error::ApiError;

pub(super) async fn require_bearer(
    State(expected_token): State<Arc<str>>,
    request: Request,
    next: Next,
) -> Response {
    let authorized = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .is_some_and(|token| {
            bool::from(token.as_bytes().ct_eq(expected_token.as_ref().as_bytes()))
        });
    if !authorized {
        return ApiError::unauthorized().into_response();
    }
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::middleware;
    use axum::routing::get;
    use axum::Router;
    use tower::ServiceExt;

    use super::*;

    fn protected_router() -> Router {
        Router::new()
            .route("/", get(|| async { "ok" }))
            .layer(middleware::from_fn_with_state(
                Arc::<str>::from("secret"),
                require_bearer,
            ))
    }

    #[tokio::test]
    async fn bearer_middleware_rejects_missing_and_wrong_tokens() {
        for authorization in [None, Some("Bearer wrong")] {
            let mut request = Request::builder().uri("/");
            if let Some(value) = authorization {
                request = request.header("authorization", value);
            }
            let response = protected_router()
                .oneshot(request.body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
    }

    #[tokio::test]
    async fn bearer_middleware_accepts_the_discovery_token() {
        let response = protected_router()
            .oneshot(
                Request::builder()
                    .uri("/")
                    .header("authorization", "Bearer secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}
