use std::sync::Arc;

use parking_lot::RwLock;

use crate::config::AppConfig;
use crate::hook_listener::AgentStatesMap;
use crate::pty::PtyManager;
use crate::ws_bridge::WsBroadcaster;

#[derive(Clone)]
pub struct McpSharedState {
    pub repo_paths: Arc<RwLock<Vec<String>>>,
    #[allow(dead_code)]
    pub pty_manager: Arc<PtyManager>,
    pub app_config: Arc<AppConfig>,
    #[allow(dead_code)]
    pub broadcaster: Arc<WsBroadcaster>,
    #[allow(dead_code)]
    pub agent_states: AgentStatesMap,
}
