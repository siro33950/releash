mod auth;
pub(crate) mod commands;
mod http;
mod rate_limit;
mod routing;
mod session;

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::domain::app_config::ConfigRepository;
use crate::usecase::agent_session::session::AgentStreamResyncReadModel;
use crate::usecase::pty_session::query_service::PtySessionReplayReader;
use crate::ws_bridge::WsBroadcaster;

#[derive(Debug, Clone, serde::Serialize)]
pub struct StartServerResult {
    pub ip: String,
    pub mode: String,
}

pub struct WsServerHandle {
    pub(crate) running: parking_lot::Mutex<bool>,
    pub(crate) shutdown_tx: parking_lot::Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    pub(crate) active_bind: parking_lot::Mutex<Option<String>>,
    pub(crate) tls_enabled: parking_lot::Mutex<bool>,
    pub(crate) connection_mode: parking_lot::Mutex<Option<String>>,
    pub(crate) server_state: parking_lot::Mutex<Option<Arc<WsServerState>>>,
}

impl Default for WsServerHandle {
    fn default() -> Self {
        Self {
            running: parking_lot::Mutex::new(false),
            shutdown_tx: parking_lot::Mutex::new(None),
            active_bind: parking_lot::Mutex::new(None),
            tls_enabled: parking_lot::Mutex::new(false),
            connection_mode: parking_lot::Mutex::new(None),
            server_state: parking_lot::Mutex::new(None),
        }
    }
}

pub(crate) struct WsServerState {
    active_connection: Arc<Mutex<bool>>,
    rate_limits: Arc<Mutex<HashMap<std::net::IpAddr, rate_limit::RateLimitEntry>>>,
    broadcaster: Arc<WsBroadcaster>,
    pty_replay_reader: Arc<dyn PtySessionReplayReader>,
    app_config: Arc<dyn ConfigRepository>,
    stream_resync_read_model: Arc<dyn AgentStreamResyncReadModel>,
    tls_enabled: bool,
}

impl WsServerState {
    pub(crate) fn new(
        broadcaster: Arc<WsBroadcaster>,
        pty_replay_reader: Arc<dyn PtySessionReplayReader>,
        app_config: Arc<dyn ConfigRepository>,
        stream_resync_read_model: Arc<dyn AgentStreamResyncReadModel>,
        tls_enabled: bool,
    ) -> Self {
        Self {
            active_connection: Arc::new(Mutex::new(false)),
            rate_limits: Arc::new(Mutex::new(HashMap::new())),
            broadcaster,
            pty_replay_reader,
            app_config,
            stream_resync_read_model,
            tls_enabled,
        }
    }

    pub(crate) fn current_token(&self) -> Result<String, String> {
        let config = self.app_config.load().map_err(|e| e.to_string())?;
        Ok(config.server.token.clone())
    }
}

#[cfg(test)]
mod tests {
    use crate::adaptor::protocol::deserialize_message;

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
