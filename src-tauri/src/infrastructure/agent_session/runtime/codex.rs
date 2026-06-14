use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tauri::{AppHandle, Manager};
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration};

use crate::app_data_dir::resolve_data_dir;
use crate::domain::agent_session::CODEX_FIXED_MODELS;
use crate::infrastructure::agent_session::runtime::bridge_common::{
    close_external_agent_process, finish_external_pending_message_turn_start,
    handle_external_bridge_message, prepare_external_pending_message_turn,
    register_external_agent_process, start_external_agent_turn_state, write_bridge_command,
    AgentProcessMap, ExternalBridgeMessageState, CODEX_BACKEND_ID,
};
use crate::infrastructure::agent_session::runtime::codex_app_server::{
    app_server_message_to_bridge_messages, build_app_server_permission_response_for_bridge_request,
    build_initialize_request, build_initialized_notification, build_thread_name_set_request,
    build_thread_resume_request, build_thread_settings_update_permission_request,
    build_thread_start_request, build_turn_interrupt_request,
    build_turn_start_request_with_permission, build_turn_steer_request, decode_jsonrpc_line,
    message_kind, spawn_app_server_process_parts, AppServerBridgeState, AppServerMessageKind,
    NOTIFY_TURN_COMPLETED,
};
use crate::infrastructure::agent_session::runtime::runtime_coordinator::wait_until_session_close_finished;
use crate::infrastructure::agent_session::runtime::{
    AgentBackend, AgentEditorContext, AgentMessage, ImageAttachment, PermissionResponse,
    SessionConfig, SessionHandle,
};
use crate::usecase::agent_session::session::SessionStore;

/// Codex app-server バックエンド。
/// Codex の実行・approval・streaming は `codex app-server` の JSON-RPC に委譲する。
/// モデル選択肢は `CODEX_FIXED_MODELS` で完全固定する。
pub struct CodexBackend {
    #[allow(dead_code)]
    runtime: Option<Arc<dyn CodexBackendRuntime>>,
    cli_path: Option<String>,
}

pub(crate) fn configured_cli_path<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Option<String> {
    app.try_state::<std::sync::Arc<crate::config::AppConfig>>()
        .and_then(|cfg_state| cfg_state.get_config().ok())
        .and_then(|cfg| cfg.agents.codex.cli_path)
        .filter(|path| !path.trim().is_empty())
}

#[allow(dead_code)]
impl CodexBackend {
    pub fn new() -> Self {
        Self {
            runtime: None,
            cli_path: None,
        }
    }

    pub fn with_agent_process_runtime(
        app: AppHandle,
        handles: Arc<Mutex<AgentProcessMap>>,
        session_store: Arc<SessionStore>,
    ) -> Self {
        let cli_path = configured_cli_path(&app);
        let resolved_cli_path = cli_path.clone().unwrap_or_else(|| "codex".to_string());
        Self {
            runtime: Some(Arc::new(AppServerCodexRuntime {
                app,
                handles,
                session_store,
                cli_path: resolved_cli_path,
                sessions: Arc::new(Mutex::new(HashMap::new())),
            })),
            cli_path,
        }
    }

    fn runtime(&self) -> Result<Arc<dyn CodexBackendRuntime>, String> {
        self.runtime.clone().ok_or_else(|| {
            "CodexBackend runtime is not attached; build the registry with app runtime".to_string()
        })
    }

    fn cli_path(&self) -> String {
        self.cli_path.clone().unwrap_or_else(|| "codex".to_string())
    }
}

#[allow(dead_code)]
#[async_trait]
trait CodexBackendRuntime: Send + Sync {
    async fn start_session(&self, config: SessionConfig) -> Result<SessionHandle, String>;
    async fn send_message(
        &self,
        session: &SessionHandle,
        message: AgentMessage,
    ) -> Result<(), String>;
    async fn steer_message(
        &self,
        session: &SessionHandle,
        message: AgentMessage,
    ) -> Result<(), String>;
    async fn active_turn_steering_ready(&self, session: &SessionHandle) -> bool;
    async fn interrupt(&self, session: &SessionHandle) -> Result<(), String>;
    async fn respond_permission(
        &self,
        session: &SessionHandle,
        response: PermissionResponse,
    ) -> Result<(), String>;
    async fn set_thread_name(&self, session: &SessionHandle, name: &str) -> Result<(), String>;
    async fn set_permission_mode(
        &self,
        session: &SessionHandle,
        cwd: &str,
        permission_mode: &str,
    ) -> Result<(), String>;
    async fn set_permission_profile(
        &self,
        session: &SessionHandle,
        cwd: &str,
        permission_mode: &str,
        permission_profile_id: Option<&str>,
    ) -> Result<(), String>;
    async fn close_session(&self, session: &SessionHandle) -> Result<(), String>;
}

struct AppServerSessionState {
    bridge_state: AppServerBridgeState,
    external_message_state: ExternalBridgeMessageState,
    next_id: u64,
}

impl AppServerSessionState {
    fn new() -> Self {
        Self {
            bridge_state: AppServerBridgeState::default(),
            external_message_state: ExternalBridgeMessageState::default(),
            next_id: 1,
        }
    }

    fn next_request_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
}

struct AppServerCodexRuntime {
    app: AppHandle,
    handles: Arc<Mutex<AgentProcessMap>>,
    session_store: Arc<SessionStore>,
    cli_path: String,
    sessions: Arc<Mutex<HashMap<String, Arc<Mutex<AppServerSessionState>>>>>,
}

struct AppServerTurnStart<'a> {
    chat_session_id: &'a str,
    state: &'a Arc<Mutex<AppServerSessionState>>,
    cwd: &'a str,
    permission_mode: &'a str,
    plan_mode: bool,
    permission_profile_id: Option<&'a str>,
    prompt: &'a str,
    images: &'a [ImageAttachment],
    streaming_message_id: &'a str,
    editor_context: Option<&'a AgentEditorContext>,
}

impl AppServerCodexRuntime {
    async fn session_state(
        &self,
        chat_session_id: &str,
    ) -> Option<Arc<Mutex<AppServerSessionState>>> {
        self.sessions.lock().await.get(chat_session_id).cloned()
    }

    async fn next_request_id(state: &Arc<Mutex<AppServerSessionState>>) -> u64 {
        state.lock().await.next_request_id()
    }

    async fn send_jsonrpc(
        &self,
        chat_session_id: &str,
        payload: serde_json::Value,
    ) -> Result<(), String> {
        write_bridge_command(&self.handles, chat_session_id, payload).await
    }

    async fn ensure_session(
        &self,
        config: &SessionConfig,
    ) -> Result<Arc<Mutex<AppServerSessionState>>, String> {
        let mut sessions = self.sessions.lock().await;
        if let Some(state) = sessions.get(&config.chat_session_id).cloned() {
            return Ok(state);
        }

        wait_until_session_close_finished(&config.chat_session_id).await;
        let parts = spawn_app_server_process_parts(&self.cli_path)?;
        let state = Arc::new(Mutex::new(AppServerSessionState::new()));
        let data_dir = resolve_data_dir(&self.app)?;
        let stored_session = self
            .session_store
            .get_session(&data_dir, &config.chat_session_id)?;
        let selected_model = stored_session.as_ref().and_then(|session| {
            session
                .selected_model
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
        });
        let saved_thread_id = stored_session.as_ref().and_then(|session| {
            session
                .agent_session_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
        });
        let permission_mode = config
            .permission_mode
            .as_deref()
            .or_else(|| {
                stored_session
                    .as_ref()
                    .map(|session| session.permission_mode.as_str())
            })
            .unwrap_or("edit")
            .to_string();
        let permission_profile_id = config
            .permission_profile_id
            .as_deref()
            .or_else(|| {
                stored_session
                    .as_ref()
                    .and_then(|session| session.permission_profile_id.as_deref())
            })
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string);

        register_external_agent_process(
            &self.app,
            &self.session_store,
            &self.handles,
            &config.chat_session_id,
            CODEX_BACKEND_ID.to_string(),
            parts.child,
            parts.stdin,
            #[cfg(unix)]
            parts.pgid,
            permission_mode.clone(),
            selected_model.clone(),
            None,
        )
        .await?;

        self.spawn_read_loop(
            config.chat_session_id.clone(),
            parts.stdout,
            Arc::clone(&state),
        );

        let initialize_id = Self::next_request_id(&state).await;
        if let Err(err) = self
            .send_jsonrpc(
                &config.chat_session_id,
                build_initialize_request(initialize_id, env!("CARGO_PKG_VERSION")),
            )
            .await
        {
            let _ = close_external_agent_process(&self.app, &self.handles, &config.chat_session_id)
                .await;
            return Err(err);
        }
        if let Err(err) = self
            .send_jsonrpc(&config.chat_session_id, build_initialized_notification())
            .await
        {
            let _ = close_external_agent_process(&self.app, &self.handles, &config.chat_session_id)
                .await;
            return Err(err);
        }
        let request_id = Self::next_request_id(&state).await;
        let thread_request = match if let Some(thread_id) = saved_thread_id.as_deref() {
            build_thread_resume_request(
                request_id,
                thread_id,
                &config.cwd,
                selected_model.as_deref(),
                Some(&permission_mode),
                config.plan_mode,
                permission_profile_id.as_deref(),
                config.system_prompt.as_deref(),
            )
        } else {
            build_thread_start_request(
                request_id,
                &config.cwd,
                selected_model.as_deref(),
                Some(&permission_mode),
                config.plan_mode,
                permission_profile_id.as_deref(),
                config.system_prompt.as_deref(),
            )
        } {
            Ok(request) => request,
            Err(err) => {
                let _ =
                    close_external_agent_process(&self.app, &self.handles, &config.chat_session_id)
                        .await;
                return Err(err);
            }
        };
        if let Err(err) = self
            .send_jsonrpc(&config.chat_session_id, thread_request)
            .await
        {
            let _ = close_external_agent_process(&self.app, &self.handles, &config.chat_session_id)
                .await;
            return Err(err);
        }

        sessions.insert(config.chat_session_id.clone(), Arc::clone(&state));

        Ok(state)
    }

    fn spawn_read_loop(
        &self,
        chat_session_id: String,
        mut stdout: tokio::io::Lines<tokio::io::BufReader<tokio::process::ChildStdout>>,
        state: Arc<Mutex<AppServerSessionState>>,
    ) {
        let app = self.app.clone();
        let session_store = Arc::clone(&self.session_store);
        let handles = Arc::clone(&self.handles);
        let sessions = Arc::clone(&self.sessions);
        tokio::spawn(async move {
            while let Ok(Some(line)) = stdout.next_line().await {
                if line.trim().is_empty() {
                    continue;
                }
                let message = match decode_jsonrpc_line(&line) {
                    Ok(message) => message,
                    Err(e) => {
                        log::warn!("invalid codex app-server message for {chat_session_id}: {e}");
                        continue;
                    }
                };
                let is_turn_completed = matches!(
                    message_kind(&message),
                    Some(AppServerMessageKind::Notification { ref method })
                        if method == NOTIFY_TURN_COMPLETED
                );
                let bridge_messages = {
                    let mut guard = state.lock().await;
                    app_server_message_to_bridge_messages(&message, &mut guard.bridge_state)
                };
                for bridge_message in bridge_messages {
                    let mut guard = state.lock().await;
                    handle_external_bridge_message(
                        &app,
                        &session_store,
                        &handles,
                        &chat_session_id,
                        bridge_message,
                        &mut guard.external_message_state,
                    )
                    .await;
                }
                if is_turn_completed {
                    if let Err(e) = start_next_app_server_pending_turn(
                        &app,
                        &session_store,
                        &handles,
                        &chat_session_id,
                        Arc::clone(&state),
                    )
                    .await
                    {
                        log::error!(
                            "failed to start pending codex app-server turn for {chat_session_id}: {e}"
                        );
                    }
                }
            }
            sessions.lock().await.remove(&chat_session_id);
        });
    }

    async fn wait_for_thread_id(
        state: &Arc<Mutex<AppServerSessionState>>,
    ) -> Result<String, String> {
        for _ in 0..200 {
            if let Some(thread_id) = state.lock().await.bridge_state.thread_id.clone() {
                return Ok(thread_id);
            }
            sleep(Duration::from_millis(25)).await;
        }
        Err("Timed out waiting for Codex app-server thread".to_string())
    }

    async fn send_turn(&self, turn: AppServerTurnStart<'_>) -> Result<(), String> {
        let thread_id = Self::wait_for_thread_id(turn.state).await?;
        start_external_agent_turn_state(
            &self.app,
            &self.session_store,
            &self.handles,
            turn.chat_session_id,
            turn.permission_mode,
            turn.streaming_message_id,
        )
        .await?;
        let id = Self::next_request_id(turn.state).await;
        self.send_jsonrpc(
            turn.chat_session_id,
            build_turn_start_request_with_permission(
                id,
                &thread_id,
                turn.cwd,
                turn.prompt,
                turn.images,
                Some(turn.streaming_message_id),
                turn.editor_context,
                Some(turn.permission_mode),
                turn.plan_mode,
                turn.permission_profile_id,
            )?,
        )
        .await
    }
}

async fn start_next_app_server_pending_turn(
    app: &AppHandle,
    session_store: &Arc<SessionStore>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    chat_session_id: &str,
    state: Arc<Mutex<AppServerSessionState>>,
) -> Result<(), String> {
    let Some(pending) =
        prepare_external_pending_message_turn(app, handles, session_store, chat_session_id).await?
    else {
        return Ok(());
    };

    let result = async {
        let thread_id = AppServerCodexRuntime::wait_for_thread_id(&state).await?;
        start_external_agent_turn_state(
            app,
            session_store,
            handles,
            chat_session_id,
            &pending.permission_mode,
            &pending.agent_message_id,
        )
        .await?;
        let id = AppServerCodexRuntime::next_request_id(&state).await;
        write_bridge_command(
            handles,
            chat_session_id,
            build_turn_start_request_with_permission(
                id,
                &thread_id,
                &pending.worktree_path,
                &pending.prompt,
                &pending.images,
                Some(&pending.agent_message_id),
                pending.editor_context.as_ref(),
                Some(&pending.permission_mode),
                pending.plan_mode,
                pending.permission_profile_id.as_deref(),
            )?,
        )
        .await
    }
    .await;

    finish_external_pending_message_turn_start(chat_session_id).await;
    result
}

#[async_trait]
impl CodexBackendRuntime for AppServerCodexRuntime {
    async fn start_session(&self, config: SessionConfig) -> Result<SessionHandle, String> {
        self.ensure_session(&config).await?;
        Ok(SessionHandle {
            chat_session_id: config.chat_session_id,
            backend_id: CODEX_BACKEND_ID.to_string(),
        })
    }

    async fn send_message(
        &self,
        session: &SessionHandle,
        message: AgentMessage,
    ) -> Result<(), String> {
        ensure_codex_session(session)?;
        let data_dir = resolve_data_dir(&self.app)?;
        let stored_session = self
            .session_store
            .get_session(&data_dir, &session.chat_session_id)?
            .ok_or_else(|| format!("Session not found: {}", session.chat_session_id))?;
        let config = SessionConfig {
            chat_session_id: session.chat_session_id.clone(),
            cwd: stored_session.worktree_path.clone(),
            permission_mode: Some(message.permission_mode.clone()),
            plan_mode: message.plan_mode,
            permission_profile_id: message.permission_profile_id.clone(),
            system_prompt: None,
        };
        let state = self.ensure_session(&config).await?;
        self.send_turn(AppServerTurnStart {
            chat_session_id: &session.chat_session_id,
            state: &state,
            cwd: &stored_session.worktree_path,
            permission_mode: &message.permission_mode,
            plan_mode: message.plan_mode,
            permission_profile_id: message.permission_profile_id.as_deref(),
            prompt: &message.content,
            images: &message.images,
            streaming_message_id: &message.streaming_message_id,
            editor_context: message.editor_context.as_ref(),
        })
        .await
    }

    async fn steer_message(
        &self,
        session: &SessionHandle,
        message: AgentMessage,
    ) -> Result<(), String> {
        ensure_codex_session(session)?;
        let data_dir = resolve_data_dir(&self.app)?;
        let stored_session = self
            .session_store
            .get_session(&data_dir, &session.chat_session_id)?
            .ok_or_else(|| format!("Session not found: {}", session.chat_session_id))?;
        let state = self
            .session_state(&session.chat_session_id)
            .await
            .ok_or_else(|| {
                format!(
                    "No active Codex app-server session: {}",
                    session.chat_session_id
                )
            })?;
        let (thread_id, turn_id) = {
            let guard = state.lock().await;
            (
                guard.bridge_state.thread_id.clone(),
                guard.bridge_state.turn_id.clone(),
            )
        };
        let thread_id = thread_id.ok_or_else(|| {
            format!(
                "Codex app-server thread is not ready: {}",
                session.chat_session_id
            )
        })?;
        let turn_id = turn_id.ok_or_else(|| {
            format!(
                "Codex app-server turn is not active: {}",
                session.chat_session_id
            )
        })?;
        let id = Self::next_request_id(&state).await;
        self.send_jsonrpc(
            &session.chat_session_id,
            build_turn_steer_request(
                id,
                &thread_id,
                &turn_id,
                &stored_session.worktree_path,
                &message.content,
                &message.images,
                Some(&message.streaming_message_id),
                message.editor_context.as_ref(),
            ),
        )
        .await
    }

    async fn active_turn_steering_ready(&self, _session: &SessionHandle) -> bool {
        // steering（turn/steer による実行中ターンへの注入）は無効化する。
        // steering を有効にすると、ターン実行中の送信が常に steer 経路へ入り、
        // キューに積まれない（=「キューに入らず投入されるが無視される」）。
        // Claude と同様、busy 時は必ずキュー経路へ流して drain で順次実行する。
        false
    }

    async fn interrupt(&self, session: &SessionHandle) -> Result<(), String> {
        ensure_codex_session(session)?;
        let state = self
            .session_state(&session.chat_session_id)
            .await
            .ok_or_else(|| {
                format!(
                    "No active Codex app-server session: {}",
                    session.chat_session_id
                )
            })?;
        let (thread_id, turn_id) = {
            let guard = state.lock().await;
            (
                guard.bridge_state.thread_id.clone(),
                guard.bridge_state.turn_id.clone(),
            )
        };
        let thread_id = thread_id.ok_or_else(|| {
            format!(
                "Codex app-server thread is not ready: {}",
                session.chat_session_id
            )
        })?;
        let turn_id = turn_id.ok_or_else(|| {
            format!(
                "Codex app-server turn is not active: {}",
                session.chat_session_id
            )
        })?;
        let id = Self::next_request_id(&state).await;
        self.send_jsonrpc(
            &session.chat_session_id,
            build_turn_interrupt_request(id, &thread_id, &turn_id),
        )
        .await
    }

    async fn respond_permission(
        &self,
        session: &SessionHandle,
        response: PermissionResponse,
    ) -> Result<(), String> {
        ensure_codex_session(session)?;
        let state = self
            .session_state(&session.chat_session_id)
            .await
            .ok_or_else(|| {
                format!(
                    "No active Codex app-server session: {}",
                    session.chat_session_id
                )
            })?;
        let payload = {
            let mut guard = state.lock().await;
            build_app_server_permission_response_for_bridge_request(
                &mut guard.bridge_state,
                &response.request_id,
                &response.behavior,
                response.updated_input.as_deref(),
            )?
        };
        self.send_jsonrpc(&session.chat_session_id, payload).await
    }

    async fn set_thread_name(&self, session: &SessionHandle, name: &str) -> Result<(), String> {
        ensure_codex_session(session)?;
        let state = self
            .session_state(&session.chat_session_id)
            .await
            .ok_or_else(|| {
                format!(
                    "No active Codex app-server session: {}",
                    session.chat_session_id
                )
            })?;
        let thread_id = state
            .lock()
            .await
            .bridge_state
            .thread_id
            .clone()
            .ok_or_else(|| {
                format!(
                    "Codex app-server thread is not ready: {}",
                    session.chat_session_id
                )
            })?;
        let id = Self::next_request_id(&state).await;
        self.send_jsonrpc(
            &session.chat_session_id,
            build_thread_name_set_request(id, &thread_id, name),
        )
        .await
    }

    async fn set_permission_mode(
        &self,
        session: &SessionHandle,
        cwd: &str,
        permission_mode: &str,
    ) -> Result<(), String> {
        ensure_codex_session(session)?;
        let state = self
            .session_state(&session.chat_session_id)
            .await
            .ok_or_else(|| {
                format!(
                    "No active Codex app-server session: {}",
                    session.chat_session_id
                )
            })?;
        let thread_id = Self::wait_for_thread_id(&state).await?;
        let id = Self::next_request_id(&state).await;
        self.send_jsonrpc(
            &session.chat_session_id,
            build_thread_settings_update_permission_request(
                id,
                &thread_id,
                cwd,
                permission_mode,
                None,
            )?,
        )
        .await
    }

    async fn set_permission_profile(
        &self,
        session: &SessionHandle,
        cwd: &str,
        permission_mode: &str,
        permission_profile_id: Option<&str>,
    ) -> Result<(), String> {
        ensure_codex_session(session)?;
        let state = self
            .session_state(&session.chat_session_id)
            .await
            .ok_or_else(|| {
                format!(
                    "No active Codex app-server session: {}",
                    session.chat_session_id
                )
            })?;
        let thread_id = Self::wait_for_thread_id(&state).await?;
        let id = Self::next_request_id(&state).await;
        self.send_jsonrpc(
            &session.chat_session_id,
            build_thread_settings_update_permission_request(
                id,
                &thread_id,
                cwd,
                permission_mode,
                permission_profile_id,
            )?,
        )
        .await
    }

    async fn close_session(&self, session: &SessionHandle) -> Result<(), String> {
        ensure_codex_session(session)?;
        self.sessions.lock().await.remove(&session.chat_session_id);
        close_external_agent_process(&self.app, &self.handles, &session.chat_session_id).await
    }
}

#[allow(dead_code)]
fn ensure_codex_session(session: &SessionHandle) -> Result<(), String> {
    if session.backend_id == CODEX_BACKEND_ID {
        return Ok(());
    }
    Err(format!(
        "Session {} belongs to backend {}, not {}",
        session.chat_session_id, session.backend_id, CODEX_BACKEND_ID
    ))
}

#[async_trait]
impl AgentBackend for CodexBackend {
    fn id(&self) -> &str {
        CODEX_BACKEND_ID
    }

    fn name(&self) -> &str {
        "Codex"
    }

    async fn start_session(&self, config: SessionConfig) -> Result<SessionHandle, String> {
        self.runtime()?.start_session(config).await
    }

    async fn send_message(
        &self,
        session: &SessionHandle,
        message: AgentMessage,
    ) -> Result<(), String> {
        self.runtime()?.send_message(session, message).await
    }

    async fn steer_message(
        &self,
        session: &SessionHandle,
        message: AgentMessage,
    ) -> Result<(), String> {
        self.runtime()?.steer_message(session, message).await
    }

    async fn active_turn_steering_ready(&self, session: &SessionHandle) -> bool {
        match self.runtime() {
            Ok(runtime) => runtime.active_turn_steering_ready(session).await,
            Err(_) => false,
        }
    }

    async fn interrupt(&self, session: &SessionHandle) -> Result<(), String> {
        self.runtime()?.interrupt(session).await
    }

    async fn respond_permission(
        &self,
        session: &SessionHandle,
        response: PermissionResponse,
    ) -> Result<(), String> {
        self.runtime()?.respond_permission(session, response).await
    }

    async fn set_thread_name(&self, session: &SessionHandle, name: &str) -> Result<(), String> {
        self.runtime()?.set_thread_name(session, name).await
    }

    async fn set_permission_mode(
        &self,
        session: &SessionHandle,
        cwd: &str,
        permission_mode: &str,
    ) -> Result<(), String> {
        self.runtime()?
            .set_permission_mode(session, cwd, permission_mode)
            .await
    }

    async fn set_permission_profile(
        &self,
        session: &SessionHandle,
        cwd: &str,
        permission_mode: &str,
        permission_profile_id: Option<&str>,
    ) -> Result<(), String> {
        self.runtime()?
            .set_permission_profile(session, cwd, permission_mode, permission_profile_id)
            .await
    }

    fn fixed_models(&self) -> Option<Vec<String>> {
        Some(CODEX_FIXED_MODELS.iter().map(|s| s.to_string()).collect())
    }

    async fn close_session(&self, session: &SessionHandle) -> Result<(), String> {
        self.runtime()?.close_session(session).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_models_returns_codex_fixed_list_in_order() {
        let backend = CodexBackend::new();
        let expected: Vec<String> = CODEX_FIXED_MODELS.iter().map(|s| s.to_string()).collect();
        assert_eq!(backend.fixed_models(), Some(expected));
    }

    #[tokio::test]
    async fn runtime_methods_require_attached_runtime() {
        let backend = CodexBackend::new();
        let session = SessionHandle {
            chat_session_id: "session-1".to_string(),
            backend_id: CODEX_BACKEND_ID.to_string(),
        };
        let message = AgentMessage {
            content: "hello".to_string(),
            streaming_message_id: "message-1".to_string(),
            images: vec![],
            permission_mode: "edit".to_string(),
            plan_mode: false,
            permission_profile_id: None,
            editor_context: None,
        };

        assert!(backend.send_message(&session, message).await.is_err());
        assert!(backend.interrupt(&session).await.is_err());
        assert!(backend.set_thread_name(&session, "Title").await.is_err());
        assert!(backend
            .set_permission_mode(&session, "/repo", "ask")
            .await
            .is_err());
        assert!(backend.close_session(&session).await.is_err());
    }
}
