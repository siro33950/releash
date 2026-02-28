use axum::extract::State;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::Response;

pub async fn auth_middleware(
    State(expected_token): State<String>,
    request: axum::http::Request<axum::body::Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    let auth_header = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok());

    match auth_header {
        Some(header) if header.starts_with("Bearer ") => {
            let token = &header[7..];
            if token == expected_token {
                Ok(next.run(request).await)
            } else {
                Err(StatusCode::UNAUTHORIZED)
            }
        }
        _ => Err(StatusCode::UNAUTHORIZED),
    }
}

#[cfg(test)]
mod tests {
    use rand::distr::Alphanumeric;
    use rand::RngExt;

    const MCP_TOKEN_LENGTH: usize = 48;

    fn generate_mcp_token() -> String {
        rand::rng()
            .sample_iter(&Alphanumeric)
            .take(MCP_TOKEN_LENGTH)
            .map(char::from)
            .collect()
    }

    #[test]
    fn token_has_correct_length() {
        let token = generate_mcp_token();
        assert_eq!(token.len(), MCP_TOKEN_LENGTH);
    }

    #[test]
    fn token_is_alphanumeric() {
        let token = generate_mcp_token();
        assert!(token.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn tokens_are_unique() {
        let t1 = generate_mcp_token();
        let t2 = generate_mcp_token();
        assert_ne!(t1, t2);
    }
}
