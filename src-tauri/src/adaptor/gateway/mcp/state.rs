use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::RwLock;

use crate::adaptor::gateway::pty_session::backend_impl::PtySessionRuntimeGateway;
use crate::domain::app_config::ConfigRepository;
use crate::usecase::code_usecase::CodeUsecase;
use crate::usecase::repository_usecase::RepositoryUsecase;
use crate::ws_bridge::WsBroadcaster;

#[derive(Clone)]
pub struct McpSharedState {
    pub repo_paths: Arc<RwLock<Vec<String>>>,
    #[allow(dead_code)]
    pub pty_session_runtime_gateway: Arc<PtySessionRuntimeGateway>,
    pub app_config: Arc<dyn ConfigRepository>,
    #[allow(dead_code)] // Used in Phase I (mobile WebSocket broadcast)
    pub broadcaster: Arc<WsBroadcaster>,
    #[allow(dead_code)] // Retained: may be needed by future MCP tools
    pub app_data_dir: Option<PathBuf>,
    /// repository 責務の usecase（worktree 読み取り・作成等の唯一の入口）。
    pub repository_usecase: Arc<RepositoryUsecase>,
    /// code 責務の usecase（read_file のファイル内容参照の唯一の入口）。
    pub code_usecase: Arc<CodeUsecase>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_state() -> McpSharedState {
        let app_config: Arc<dyn ConfigRepository> =
            Arc::new(crate::adaptor::gateway::app_config::AppConfig::new(
                crate::adaptor::gateway::app_config::ReleashConfig::default(),
                PathBuf::from("/tmp/test-config.toml"),
            ));
        let pty_session_runtime_gateway = Arc::new(PtySessionRuntimeGateway::default());
        let broadcaster = Arc::new(WsBroadcaster::default());

        McpSharedState {
            repo_paths: Arc::new(RwLock::new(vec!["/tmp/repo".to_string()])),
            pty_session_runtime_gateway,
            app_config,
            broadcaster,
            app_data_dir: None,
            repository_usecase: Arc::new(
                crate::adaptor::controller::wiring::build_repository_usecase(),
            ),
            code_usecase: Arc::new(crate::adaptor::controller::wiring::build_code_usecase()),
        }
    }

    #[test]
    fn clone_shares_arc_references() {
        let state = make_state();
        let cloned = state.clone();

        assert!(Arc::ptr_eq(&state.repo_paths, &cloned.repo_paths));
        assert!(Arc::ptr_eq(
            &state.pty_session_runtime_gateway,
            &cloned.pty_session_runtime_gateway
        ));
        assert!(Arc::ptr_eq(&state.app_config, &cloned.app_config));
        assert!(Arc::ptr_eq(&state.broadcaster, &cloned.broadcaster));
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
