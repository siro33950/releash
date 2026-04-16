use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::RwLock;

use crate::comment_store::CommentStore;
use crate::config::AppConfig;
use crate::hook_listener::AgentStatesMap;
use crate::pty::PtyManager;
use crate::thread_store::ThreadStore;
use crate::ws_bridge::WsBroadcaster;

#[derive(Clone)]
pub struct McpSharedState {
    pub repo_paths: Arc<RwLock<Vec<String>>>,
    #[allow(dead_code)]
    pub pty_manager: Arc<PtyManager>,
    pub app_config: Arc<AppConfig>,
    #[allow(dead_code)] // Used in Phase I (mobile WebSocket broadcast)
    pub broadcaster: Arc<WsBroadcaster>,
    #[allow(dead_code)]
    pub agent_states: AgentStatesMap,
    #[allow(dead_code)] // Retained for backward compatibility during migration
    pub comment_store: Arc<CommentStore>,
    pub thread_store: Arc<ThreadStore>,
    pub app_handle: Option<tauri::AppHandle>,
    #[allow(dead_code)] // Retained: may be needed by future MCP tools
    pub app_data_dir: Option<PathBuf>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn make_state() -> McpSharedState {
        let app_config = Arc::new(crate::config::AppConfig::new(
            crate::config::ReleashConfig::default(),
            PathBuf::from("/tmp/test-config.toml"),
        ));
        let pty_manager = Arc::new(PtyManager::default());
        let broadcaster = Arc::new(WsBroadcaster::default());
        let agent_states: AgentStatesMap = Arc::new(parking_lot::Mutex::new(HashMap::new()));
        let comment_store = Arc::new(CommentStore::default());
        let thread_store = Arc::new(ThreadStore::default());

        McpSharedState {
            repo_paths: Arc::new(RwLock::new(vec!["/tmp/repo".to_string()])),
            pty_manager,
            app_config,
            broadcaster,
            agent_states,
            comment_store,
            thread_store,
            app_handle: None,
            app_data_dir: None,
        }
    }

    #[test]
    fn clone_shares_arc_references() {
        let state = make_state();
        let cloned = state.clone();

        assert!(Arc::ptr_eq(&state.repo_paths, &cloned.repo_paths));
        assert!(Arc::ptr_eq(&state.pty_manager, &cloned.pty_manager));
        assert!(Arc::ptr_eq(&state.app_config, &cloned.app_config));
        assert!(Arc::ptr_eq(&state.broadcaster, &cloned.broadcaster));
        assert!(Arc::ptr_eq(&state.agent_states, &cloned.agent_states));
        assert!(Arc::ptr_eq(&state.comment_store, &cloned.comment_store));
        assert!(Arc::ptr_eq(&state.thread_store, &cloned.thread_store));
    }

    #[test]
    fn shared_mutation_via_repo_paths() {
        let state = make_state();
        let cloned = state.clone();

        cloned.repo_paths.write().push("/tmp/repo2".to_string());

        let paths = state.repo_paths.read();
        assert_eq!(paths.len(), 2);
        assert_eq!(paths[1], "/tmp/repo2");
    }
}
