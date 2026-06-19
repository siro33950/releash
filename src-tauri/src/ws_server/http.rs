use std::net::SocketAddr;
use std::sync::Arc;

use http_body_util::Full;
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;

use crate::config::ReleashConfig;

use super::session::handle_ws_session;
use super::WsServerState;

pub(super) fn apply_security_headers(
    builder: hyper::http::response::Builder,
    tls_enabled: bool,
) -> hyper::http::response::Builder {
    let builder = builder
        .header("X-Content-Type-Options", "nosniff")
        .header("X-Frame-Options", "DENY")
        .header("Referrer-Policy", "strict-origin-when-cross-origin");
    if tls_enabled {
        builder.header("Strict-Transport-Security", "max-age=31536000")
    } else {
        builder
    }
}

pub(super) fn content_type_for(path: &str) -> &'static str {
    if path.ends_with(".html") {
        "text/html; charset=utf-8"
    } else if path.ends_with(".js") {
        "application/javascript; charset=utf-8"
    } else if path.ends_with(".css") {
        "text/css; charset=utf-8"
    } else if path.ends_with(".json") {
        "application/json"
    } else if path.ends_with(".svg") {
        "image/svg+xml"
    } else if path.ends_with(".png") {
        "image/png"
    } else if path.ends_with(".ico") {
        "image/x-icon"
    } else if path.ends_with(".woff2") {
        "font/woff2"
    } else {
        "application/octet-stream"
    }
}

pub(super) async fn start_ws_server(
    cfg: &ReleashConfig,
    server_state: Arc<WsServerState>,
    shutdown_rx: tokio::sync::oneshot::Receiver<()>,
) -> Result<(), String> {
    if (cfg.server.bind == "0.0.0.0" || cfg.server.bind == "any") && !cfg.server.tls.enabled {
        return Err(
            "セキュリティ上の理由により、bind=0.0.0.0/any かつ TLS無効での起動は拒否されます"
                .to_string(),
        );
    }

    let bind_addr = if cfg.server.bind == "any" {
        "0.0.0.0".to_string()
    } else {
        cfg.server.bind.clone()
    };
    let addr = format!("{}:{}", bind_addr, cfg.server.port);

    let tls_acceptor = if cfg.server.tls.enabled {
        Some(
            crate::adaptor::gateway::remote_access::certificate_impl::load_tls_config(
                &cfg.server.tls.cert,
                &cfg.server.tls.key,
            )?,
        )
    } else {
        None
    };

    let listener = TcpListener::bind(&addr)
        .await
        .map_err(|e| format!("サーバー起動失敗: {e}"))?;

    log::info!("WebSocket server listening on {}", addr);

    tokio::spawn(async move {
        tokio::select! {
            _ = async {
                loop {
                    let Ok((stream, peer_addr)) = listener.accept().await else {
                        continue;
                    };

                    let server_state = Arc::clone(&server_state);
                    let tls_acceptor = tls_acceptor.clone();

                    tokio::spawn(async move {
                        let result = if let Some(tls) = &tls_acceptor {
                            match tls.accept(stream).await {
                                Ok(tls_stream) => {
                                    serve_hyper_connection(TokioIo::new(tls_stream), peer_addr, server_state).await
                                }
                                Err(e) => Err(format!("TLS handshake failed: {e}")),
                            }
                        } else {
                            serve_hyper_connection(TokioIo::new(stream), peer_addr, server_state).await
                        };
                        if let Err(e) = result {
                            log::warn!("Connection error from {}: {}", peer_addr, e);
                        }
                    });
                }
            } => {},
            _ = shutdown_rx => {
                log::info!("WebSocket server shutting down");
            }
        }
    });

    Ok(())
}

async fn serve_hyper_connection<I>(
    io: I,
    peer_addr: SocketAddr,
    state: Arc<WsServerState>,
) -> Result<(), String>
where
    I: hyper::rt::Read + hyper::rt::Write + Unpin + Send + 'static,
{
    let service = service_fn(move |req| {
        let state = Arc::clone(&state);
        async move { Ok::<_, std::convert::Infallible>(handle_http(req, peer_addr, state).await) }
    });

    http1::Builder::new()
        .serve_connection(io, service)
        .with_upgrades()
        .await
        .map_err(|e| format!("HTTP connection error: {e}"))
}

fn is_ws_upgrade(req: &Request<hyper::body::Incoming>) -> bool {
    req.headers()
        .get(hyper::header::UPGRADE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false)
}

async fn handle_http(
    req: Request<hyper::body::Incoming>,
    peer_addr: SocketAddr,
    state: Arc<WsServerState>,
) -> Response<Full<Bytes>> {
    let path = req.uri().path().to_string();
    let tls = state.tls_enabled;
    if is_ws_upgrade(&req) {
        match handle_ws_upgrade(req, peer_addr, state) {
            Ok(resp) => resp,
            Err(e) => error_response(StatusCode::BAD_REQUEST, &e, tls),
        }
    } else {
        serve_remote(&path, &state)
    }
}

fn handle_ws_upgrade(
    mut req: Request<hyper::body::Incoming>,
    peer_addr: SocketAddr,
    state: Arc<WsServerState>,
) -> Result<Response<Full<Bytes>>, String> {
    let key = req
        .headers()
        .get("sec-websocket-key")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .ok_or("Missing Sec-WebSocket-Key")?;

    let accept = tokio_tungstenite::tungstenite::handshake::derive_accept_key(key.as_bytes());
    let on_upgrade = hyper::upgrade::on(&mut req);

    tokio::spawn(async move {
        match on_upgrade.await {
            Ok(upgraded) => {
                let ws = tokio_tungstenite::WebSocketStream::from_raw_socket(
                    TokioIo::new(upgraded),
                    tokio_tungstenite::tungstenite::protocol::Role::Server,
                    None,
                )
                .await;
                if let Err(e) = handle_ws_session(ws, peer_addr, &state).await {
                    log::warn!("WebSocket error from {}: {}", peer_addr, e);
                }
            }
            Err(e) => {
                log::warn!("WebSocket upgrade failed for {}: {}", peer_addr, e);
            }
        }
    });

    Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header(hyper::header::UPGRADE, "websocket")
        .header(hyper::header::CONNECTION, "Upgrade")
        .header("Sec-WebSocket-Accept", accept)
        .body(Full::default())
        .map_err(|e| e.to_string())
}

fn serve_remote(path: &str, state: &WsServerState) -> Response<Full<Bytes>> {
    let remote_dir = match &state.remote_dir {
        Some(d) => d,
        None => {
            return error_response(
                StatusCode::NOT_FOUND,
                "Remote UI is not available",
                state.tls_enabled,
            )
        }
    };

    let file_path = match path {
        "/" | "" => "remote.html",
        p => p.trim_start_matches('/'),
    };

    let tls = state.tls_enabled;
    let full_path = remote_dir.join(file_path);
    if let (Ok(canonical), Ok(remote_canonical)) =
        (full_path.canonicalize(), remote_dir.canonicalize())
    {
        if !canonical.starts_with(&remote_canonical) {
            return error_response(StatusCode::FORBIDDEN, "Access denied", tls);
        }
        match std::fs::read(&canonical) {
            Ok(content) => {
                let ct = content_type_for(canonical.to_str().unwrap_or(""));
                apply_security_headers(Response::builder(), tls)
                    .status(StatusCode::OK)
                    .header("Content-Type", ct)
                    .header("Cache-Control", "no-cache")
                    .body(Full::new(Bytes::from(content)))
                    .unwrap()
            }
            Err(_) => serve_remote_fallback(remote_dir, tls),
        }
    } else {
        serve_remote_fallback(remote_dir, tls)
    }
}

fn serve_remote_fallback(remote_dir: &std::path::Path, tls_enabled: bool) -> Response<Full<Bytes>> {
    match std::fs::read(remote_dir.join("remote.html")) {
        Ok(content) => apply_security_headers(Response::builder(), tls_enabled)
            .status(StatusCode::OK)
            .header("Content-Type", "text/html; charset=utf-8")
            .body(Full::new(Bytes::from(content)))
            .unwrap(),
        Err(_) => error_response(StatusCode::NOT_FOUND, "Not Found", tls_enabled),
    }
}

pub(super) fn error_response(
    status: StatusCode,
    msg: &str,
    tls_enabled: bool,
) -> Response<Full<Bytes>> {
    apply_security_headers(Response::builder(), tls_enabled)
        .status(status)
        .body(Full::new(Bytes::from(msg.to_string())))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_content_type_html() {
        assert_eq!(content_type_for("index.html"), "text/html; charset=utf-8");
    }

    #[test]
    fn test_content_type_js() {
        assert_eq!(
            content_type_for("app.js"),
            "application/javascript; charset=utf-8"
        );
    }

    #[test]
    fn test_content_type_unknown() {
        assert_eq!(content_type_for("data.bin"), "application/octet-stream");
    }

    #[test]
    fn test_security_block_any_without_tls() {
        let bind = "0.0.0.0";
        let tls_enabled = false;
        let should_block = (bind == "0.0.0.0" || bind == "any") && !tls_enabled;
        assert!(should_block);
    }

    #[test]
    fn test_security_allow_localhost_without_tls() {
        let bind = "127.0.0.1";
        let tls_enabled = false;
        let should_block = (bind == "0.0.0.0" || bind == "any") && !tls_enabled;
        assert!(!should_block);
    }

    #[test]
    fn test_security_allow_any_with_tls() {
        let bind = "0.0.0.0";
        let tls_enabled = true;
        let should_block = (bind == "0.0.0.0" || bind == "any") && !tls_enabled;
        assert!(!should_block);
    }
}
