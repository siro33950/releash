use axum::extract::State;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::Response;

use crate::domain::mcp::services::is_authorized_bearer;

pub async fn auth_middleware(
    State(expected_token): State<String>,
    request: axum::http::Request<axum::body::Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    let auth_header = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok());

    if is_authorized_bearer(auth_header, &expected_token) {
        Ok(next.run(request).await)
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

#[cfg(test)]
mod tests {
    use crate::domain::mcp::services::is_authorized_bearer;

    #[test]
    fn test_bearer認証_正しいauthorizationヘッダーだけ許可する() {
        assert!(is_authorized_bearer(Some("Bearer token"), "token"));
        assert!(!is_authorized_bearer(Some("Bearer other"), "token"));
        assert!(!is_authorized_bearer(Some("Basic token"), "token"));
        assert!(!is_authorized_bearer(None, "token"));
    }
}
