use std::collections::HashMap;
use std::sync::Arc;

use http_body_util::Full;
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;

use crate::config::AppConfig;
use crate::protocol::{AgentHookPayload, AgentStateSync, WsMessage};
use crate::ws_bridge::WsBroadcaster;

pub type AgentStatesMap = Arc<parking_lot::Mutex<HashMap<String, AgentStateSync>>>;

#[tauri::command]
pub fn get_agent_states(
    states: tauri::State<'_, AgentStatesMap>,
) -> HashMap<String, AgentStateSync> {
    states.lock().clone()
}

pub struct HookListenerState {
    pub app_config: Arc<AppConfig>,
    pub app_handle: tauri::AppHandle,
    pub broadcaster: Arc<WsBroadcaster>,
    pub agent_states: Arc<parking_lot::Mutex<HashMap<String, AgentStateSync>>>,
}

pub async fn start_hook_listener(state: HookListenerState) -> Result<(), String> {
    let port = state
        .app_config
        .get_config()
        .map(|c| c.server.hook_port)
        .unwrap_or(19700);

    let addr = format!("127.0.0.1:{port}");
    let listener = TcpListener::bind(&addr)
        .await
        .map_err(|e| format!("Hook listener bind failed on {addr}: {e}"))?;

    log::info!("Hook listener started on {addr}");

    let state = Arc::new(state);

    loop {
        let Ok((stream, _peer_addr)) = listener.accept().await else {
            continue;
        };

        let state = Arc::clone(&state);
        tokio::spawn(async move {
            let service = service_fn(move |req| {
                let state = Arc::clone(&state);
                async move { Ok::<_, std::convert::Infallible>(handle_request(req, &state).await) }
            });

            if let Err(e) = http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service)
                .await
            {
                log::warn!("Hook listener connection error: {e}");
            }
        });
    }
}

async fn handle_request(
    req: Request<hyper::body::Incoming>,
    state: &HookListenerState,
) -> Response<Full<Bytes>> {
    let method = req.method().clone();
    let path = req.uri().path().to_string();

    if method == hyper::Method::POST && path == "/hooks/agent" {
        handle_agent_hook(req, state).await
    } else {
        Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Full::new(Bytes::from("Not Found")))
            .unwrap()
    }
}

fn extract_bearer_token(req: &Request<hyper::body::Incoming>) -> Option<String> {
    req.headers()
        .get(hyper::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|s| s.to_string())
}

fn error_response(status: StatusCode, msg: &str) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .body(Full::new(Bytes::from(msg.to_string())))
        .unwrap()
}

async fn handle_agent_hook(
    req: Request<hyper::body::Incoming>,
    state: &HookListenerState,
) -> Response<Full<Bytes>> {
    let token = match extract_bearer_token(&req) {
        Some(t) => t,
        None => return error_response(StatusCode::UNAUTHORIZED, "Missing Authorization header"),
    };

    let expected = match state.app_config.get_config() {
        Ok(c) => c.server.token.clone(),
        Err(_) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Config error"),
    };

    if token != expected {
        return error_response(StatusCode::UNAUTHORIZED, "Invalid token");
    }

    let body = {
        use http_body_util::BodyExt;
        match req.into_body().collect().await {
            Ok(b) => b.to_bytes().to_vec(),
            Err(_) => return error_response(StatusCode::BAD_REQUEST, "Failed to read body"),
        }
    };

    let payload: AgentHookPayload = match serde_json::from_slice(&body) {
        Ok(p) => p,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, &format!("Invalid JSON: {e}")),
    };

    let sync = AgentStateSync::from_payload(&payload);

    {
        let mut states = state.agent_states.lock();
        states.insert(sync.worktree_path.clone(), sync.clone());
    }

    {
        use tauri::Emitter;
        let _ = state.app_handle.emit("agent-state-changed", &sync);
    }

    state
        .broadcaster
        .try_send(WsMessage::AgentStateSync(sync.clone()));

    if let Ok(cfg) = state.app_config.get_config() {
        let url = cfg.server.notify.webhook_url.clone();
        if !url.is_empty() {
            let sync_clone = sync;
            tokio::spawn(async move {
                crate::webhook::send_webhook(&url, &sync_clone).await;
            });
        }
    }

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Full::new(Bytes::from(r#"{"ok":true}"#)))
        .unwrap()
}

#[cfg(test)]
mod tests {
    fn parse_bearer(header_value: Option<&str>) -> Option<String> {
        header_value
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(|s| s.to_string())
    }

    #[test]
    fn extract_bearer_token_valid() {
        assert_eq!(
            parse_bearer(Some("Bearer my-secret-token")),
            Some("my-secret-token".to_string())
        );
    }

    #[test]
    fn extract_bearer_token_missing() {
        assert_eq!(parse_bearer(None), None);
    }

    #[test]
    fn extract_bearer_token_wrong_scheme() {
        assert_eq!(parse_bearer(Some("Basic abc123")), None);
    }
}
