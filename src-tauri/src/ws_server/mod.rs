mod auth;
mod commands;
mod handlers;
mod http;
mod rate_limit;
mod routing;
mod session;
mod validation;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::config::AppConfig;
use crate::pty::PtyManager;
use crate::ws_bridge::WsBroadcaster;

pub use commands::{broadcast_comments, get_server_status, start_server, stop_server};

#[derive(Debug, Clone, serde::Serialize)]
pub struct StartServerResult {
    pub ip: String,
    pub mode: String,
}

pub struct WsServerHandle {
    running: parking_lot::Mutex<bool>,
    shutdown_tx: parking_lot::Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    active_bind: parking_lot::Mutex<Option<String>>,
    tls_enabled: parking_lot::Mutex<bool>,
    connection_mode: parking_lot::Mutex<Option<String>>,
}

impl Default for WsServerHandle {
    fn default() -> Self {
        Self {
            running: parking_lot::Mutex::new(false),
            shutdown_tx: parking_lot::Mutex::new(None),
            active_bind: parking_lot::Mutex::new(None),
            tls_enabled: parking_lot::Mutex::new(false),
            connection_mode: parking_lot::Mutex::new(None),
        }
    }
}

impl WsServerHandle {
    pub fn active_bind(&self) -> Option<String> {
        self.active_bind.lock().clone()
    }

    pub fn is_tls_enabled(&self) -> bool {
        *self.tls_enabled.lock()
    }
}

pub(crate) struct WsServerState {
    active_connection: Arc<Mutex<bool>>,
    rate_limits: Arc<Mutex<HashMap<std::net::IpAddr, rate_limit::RateLimitEntry>>>,
    remote_dir: Option<PathBuf>,
    broadcaster: Arc<WsBroadcaster>,
    pty_manager: Option<Arc<PtyManager>>,
    repo_path: Option<String>,
    app_config: Arc<AppConfig>,
    app_handle: Option<tauri::AppHandle>,
    tls_enabled: bool,
}

impl WsServerState {
    pub(crate) fn new(
        remote_dir: Option<PathBuf>,
        broadcaster: Arc<WsBroadcaster>,
        pty_manager: Option<Arc<PtyManager>>,
        repo_path: Option<String>,
        app_config: Arc<AppConfig>,
        app_handle: Option<tauri::AppHandle>,
        tls_enabled: bool,
    ) -> Self {
        Self {
            active_connection: Arc::new(Mutex::new(false)),
            rate_limits: Arc::new(Mutex::new(HashMap::new())),
            remote_dir,
            broadcaster,
            pty_manager,
            repo_path,
            app_config,
            app_handle,
            tls_enabled,
        }
    }

    pub(crate) fn current_token(&self) -> Result<String, String> {
        let config = self.app_config.get_config()?;
        Ok(config.server.token.clone())
    }
}

#[cfg(test)]
mod tests {
    use crate::protocol::deserialize_message;

    #[test]
    fn test_deserialize_invalid_json() {
        let result = deserialize_message("not valid json at all");
        assert!(result.is_err());
    }

    #[test]
    fn test_deserialize_empty_payload() {
        let result = deserialize_message("");
        assert!(result.is_err());
    }

    #[test]
    fn test_deserialize_missing_type_field() {
        let result = deserialize_message(r#"{"data": "hello"}"#);
        assert!(result.is_err());
    }
}
