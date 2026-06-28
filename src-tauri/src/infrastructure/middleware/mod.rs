mod auth;
mod http_upgrade;
mod rate_limit;
pub(crate) mod server_control;
mod session;

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::adaptor::gateway::shared::ws_broadcaster::WsBroadcaster;
use crate::domain::app_config::ConfigRepository;
use crate::usecase::agent_session::session::AgentStreamResyncReadModel;
use crate::usecase::agent_session::status::AgentStatusCenter;
use crate::usecase::pty_session::query_service::PtySessionReplayReader;

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
    agent_status_center: Arc<AgentStatusCenter>,
    tls_enabled: bool,
}

impl WsServerState {
    pub(crate) fn new(
        broadcaster: Arc<WsBroadcaster>,
        pty_replay_reader: Arc<dyn PtySessionReplayReader>,
        app_config: Arc<dyn ConfigRepository>,
        stream_resync_read_model: Arc<dyn AgentStreamResyncReadModel>,
        agent_status_center: Arc<AgentStatusCenter>,
        tls_enabled: bool,
    ) -> Self {
        Self {
            active_connection: Arc::new(Mutex::new(false)),
            rate_limits: Arc::new(Mutex::new(HashMap::new())),
            broadcaster,
            pty_replay_reader,
            app_config,
            stream_resync_read_model,
            agent_status_center,
            tls_enabled,
        }
    }

    pub(crate) fn current_token(&self) -> Result<String, String> {
        let config = self.app_config.load().map_err(|e| e.to_string())?;
        Ok(config.server.token.clone())
    }
}
