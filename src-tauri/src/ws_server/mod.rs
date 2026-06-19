mod auth;
pub(crate) mod commands;
pub(crate) mod handlers;
mod http;
mod rate_limit;
mod routing;
mod session;
pub(crate) mod validation;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::adaptor::gateway::pty_session::backend_impl::PtySessionRuntimeGateway;
use crate::config::AppConfig;
use crate::git_host::PrCache;
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

impl WsServerHandle {
    pub fn active_bind(&self) -> Option<String> {
        self.active_bind.lock().clone()
    }

    pub fn is_tls_enabled(&self) -> bool {
        *self.tls_enabled.lock()
    }

    pub fn connection_mode(&self) -> Option<String> {
        self.connection_mode.lock().clone()
    }

    pub fn is_running(&self) -> bool {
        *self.running.lock()
    }
}

pub(crate) struct WsServerState {
    active_connection: Arc<Mutex<bool>>,
    rate_limits: Arc<Mutex<HashMap<std::net::IpAddr, rate_limit::RateLimitEntry>>>,
    remote_dir: Option<PathBuf>,
    broadcaster: Arc<WsBroadcaster>,
    pty_session_runtime_gateway: Option<Arc<PtySessionRuntimeGateway>>,
    repo_paths: Arc<parking_lot::RwLock<Vec<String>>>,
    terminal_startup_command: Arc<parking_lot::RwLock<String>>,
    app_config: Arc<AppConfig>,
    app_handle: Option<tauri::AppHandle>,
    tls_enabled: bool,
    pr_cache: Arc<PrCache>,
    backend_registry: Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>,
    /// repository 責務の usecase（composition root の wiring で1回構築し State が Arc で
    /// 保持・再利用する）。WS ハンドラはこれを介して worktree / branch を読み書きする。
    repository_usecase: Arc<crate::usecase::repository_usecase::RepositoryUsecase>,
    /// テスト経路から SessionStore / data_dir を直接注入するためのバックドア。
    /// AppHandle を必要としない統合テスト（Spec issues-947 の AgentSessionStartRequest 正常系等）で
    /// `create_session_with_permission` を AppHandle 無しで走らせる。
    #[cfg(test)]
    test_session_deps: Option<(
        Arc<crate::usecase::agent_session::session::SessionStore>,
        PathBuf,
    )>,
    #[cfg(test)]
    test_review_deps: Option<(Arc<crate::review_comments::ReviewCommentStore>, PathBuf)>,
    #[cfg(test)]
    test_review_emit_log: Arc<parking_lot::Mutex<Vec<String>>>,
}

impl WsServerState {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        remote_dir: Option<PathBuf>,
        broadcaster: Arc<WsBroadcaster>,
        pty_session_runtime_gateway: Option<Arc<PtySessionRuntimeGateway>>,
        repo_paths: Arc<parking_lot::RwLock<Vec<String>>>,
        app_config: Arc<AppConfig>,
        app_handle: Option<tauri::AppHandle>,
        tls_enabled: bool,
        pr_cache: Arc<PrCache>,
        backend_registry: Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>,
        repository_usecase: Arc<crate::usecase::repository_usecase::RepositoryUsecase>,
    ) -> Self {
        Self {
            active_connection: Arc::new(Mutex::new(false)),
            rate_limits: Arc::new(Mutex::new(HashMap::new())),
            remote_dir,
            broadcaster,
            pty_session_runtime_gateway,
            repo_paths,
            terminal_startup_command: Arc::new(parking_lot::RwLock::new(String::new())),
            app_config,
            app_handle,
            tls_enabled,
            pr_cache,
            backend_registry,
            repository_usecase,
            #[cfg(test)]
            test_session_deps: None,
            #[cfg(test)]
            test_review_deps: None,
            #[cfg(test)]
            test_review_emit_log: Arc::new(parking_lot::Mutex::new(Vec::new())),
        }
    }

    #[cfg(test)]
    pub(crate) fn set_test_session_deps(
        &mut self,
        session_store: Arc<crate::usecase::agent_session::session::SessionStore>,
        data_dir: PathBuf,
    ) {
        self.test_session_deps = Some((session_store, data_dir));
    }

    #[cfg(test)]
    pub(crate) fn set_test_review_deps(
        &mut self,
        store: Arc<crate::review_comments::ReviewCommentStore>,
        data_dir: PathBuf,
    ) {
        self.test_review_deps = Some((store, data_dir));
    }

    #[cfg(test)]
    pub(crate) fn test_review_emit_log(&self) -> Vec<String> {
        self.test_review_emit_log.lock().clone()
    }

    pub(crate) fn get_backend_registry(
        &self,
    ) -> &Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry> {
        &self.backend_registry
    }

    /// WS handler の session start 経路向け。検証済み抽象 PermissionMode を初回保存で確定する。
    /// edit デフォルトで save → update_permission_mode の二段階保存を回避し、途中失敗時に
    /// 中間状態（edit のセッションだけ残る）が発生しないようにする（Spec issues-947）。
    pub(crate) fn create_session_with_permission(
        &self,
        worktree_path: &str,
        backend_id: Option<String>,
        permission_mode: crate::permission::PermissionMode,
        selected_model: Option<String>,
    ) -> Result<crate::usecase::agent_session::session::ChatSession, String> {
        #[cfg(test)]
        {
            if let Some((session_store, data_dir)) = &self.test_session_deps {
                return crate::usecase::agent_session::session::create_session_internal_with_attributes(
                    session_store,
                    data_dir,
                    worktree_path,
                    backend_id,
                    permission_mode,
                    crate::usecase::agent_session::session::SessionCreationAttributes {
                        selected_model,
                        ..Default::default()
                    },
                );
            }
        }
        use tauri::Manager;
        let app = self.app_handle.as_ref().ok_or("App handle not available")?;
        let session_store =
            app.state::<Arc<crate::usecase::agent_session::session::SessionStore>>();
        let data_dir = crate::app_data_dir::resolve_data_dir(app)?;
        match backend_id {
            Some(bid) => {
                crate::usecase::agent_session::session::create_session_with_model_and_plan_mode(
                    &session_store,
                    &self.backend_registry,
                    &data_dir,
                    worktree_path,
                    bid,
                    permission_mode,
                    selected_model,
                    false,
                )
            }
            None => {
                crate::usecase::agent_session::session::create_session_internal_with_permission(
                    &session_store,
                    &data_dir,
                    worktree_path,
                    None,
                    permission_mode,
                )
            }
        }
    }

    pub(crate) fn get_repo_paths(&self) -> Vec<String> {
        self.repo_paths.read().clone()
    }

    pub(crate) fn repository_usecase(
        &self,
    ) -> &Arc<crate::usecase::repository_usecase::RepositoryUsecase> {
        &self.repository_usecase
    }

    pub(crate) fn broadcaster(&self) -> &Arc<WsBroadcaster> {
        &self.broadcaster
    }

    pub(crate) fn pty_session_runtime_gateway(&self) -> Option<&Arc<PtySessionRuntimeGateway>> {
        self.pty_session_runtime_gateway.as_ref()
    }

    pub(crate) fn app_handle(&self) -> Option<&tauri::AppHandle> {
        self.app_handle.as_ref()
    }

    pub(crate) fn pr_cache(&self) -> &Arc<PrCache> {
        &self.pr_cache
    }

    pub(crate) fn get_terminal_startup_command(&self) -> String {
        self.terminal_startup_command.read().clone()
    }

    pub(crate) fn set_terminal_startup_command(&self, command: String) {
        *self.terminal_startup_command.write() = command;
    }

    pub(crate) fn current_token(&self) -> Result<String, String> {
        let config = self.app_config.get_config()?;
        Ok(config.server.token.clone())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::config::AppConfig;
    use crate::protocol::deserialize_message;
    use crate::ws_bridge::WsBroadcaster;

    use super::WsServerState;

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

    #[test]
    fn test_get_repo_paths_returns_initial() {
        let config = crate::config::ReleashConfig::default();
        let app_config = Arc::new(AppConfig::new(
            config,
            std::path::PathBuf::from("/tmp/test-releash.toml"),
        ));
        let state = WsServerState::new(
            None,
            Arc::new(WsBroadcaster::default()),
            None,
            Arc::new(parking_lot::RwLock::new(vec![
                "/repo/a".to_string(),
                "/repo/b".to_string(),
            ])),
            app_config,
            None,
            false,
            Arc::new(crate::git_host::PrCache::new()),
            Arc::new(crate::infrastructure::agent_session::runtime::AgentBackendRegistry::new()),
            Arc::new(crate::adaptor::controller::wiring::build_repository_usecase()),
        );
        assert_eq!(
            state.get_repo_paths(),
            vec!["/repo/a".to_string(), "/repo/b".to_string()]
        );
    }
}
