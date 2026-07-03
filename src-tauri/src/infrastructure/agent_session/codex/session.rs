use std::collections::HashMap;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use futures_util::stream::{self, Stream};
use serde_json::{json, Value};
use tokio::sync::{mpsc, Mutex};

use crate::domain::agent_session::entities::{AttachmentPayload, PermissionResponse};
use crate::domain::agent_session::gateway::{
    AgentBackendError, AgentRuntimeEvent, AgentSessionRuntime, SessionSpec, TurnInput,
};
use crate::domain::agent_session::value_objects::{EditorContext, ModelId, PermissionMode};

use super::app_server::{CodexAppServerHandle, CodexAppServerProcess};
use super::convert::{convert_jsonrpc_message, CodexConvertState};
use super::permission::{codex_permission_response, codex_permission_settings};
use super::wire::{
    initialize_request, initialized_notification, message_kind, request, AppServerMessageKind,
    METHOD_THREAD_RESUME, METHOD_THREAD_SETTINGS_UPDATE, METHOD_THREAD_START,
    METHOD_TURN_INTERRUPT, METHOD_TURN_START,
};

const AGENT_PROCESS_EXITED_UNEXPECTEDLY: &str = "Agent process exited unexpectedly";

pub(crate) struct CodexSessionRuntime {
    handle: CodexAppServerHandle,
    state: Arc<Mutex<CodexRuntimeState>>,
    events: StdMutex<Option<mpsc::UnboundedReceiver<AgentRuntimeEvent>>>,
    next_id: AtomicU64,
    closed: Arc<AtomicBool>,
}

#[derive(Debug)]
struct CodexRuntimeState {
    thread_id: Option<String>,
    turn_id: Option<String>,
    startup_error: Option<String>,
    cwd: String,
    model: ModelId,
    permission_mode: PermissionMode,
    plan_mode: bool,
    permission_profile_id: Option<String>,
    pending_methods: HashMap<u64, String>,
    pending_client_methods: HashMap<u64, String>,
}

impl CodexSessionRuntime {
    pub(crate) async fn open(
        cli_path: String,
        spec: SessionSpec,
    ) -> Result<Self, AgentBackendError> {
        let timeout = startup_timeout_for_spec(&spec);
        let max_retries = startup_max_retries_for_spec(&spec);
        let mut attempts = 0;
        loop {
            let runtime = Self::open_once(cli_path.clone(), spec.clone()).await?;
            match wait_for_thread_id(&runtime.state, timeout).await {
                Ok(_) => return Ok(runtime),
                Err(AgentBackendError::StartupTimeout { .. }) if attempts < max_retries => {
                    attempts += 1;
                    runtime.close().await;
                }
                Err(AgentBackendError::StartupTimeout { .. }) => {
                    runtime.close().await;
                    return Err(AgentBackendError::StartupTimeout {
                        retry_count: attempts,
                        max_retries,
                    });
                }
                Err(error) => {
                    runtime.close().await;
                    return Err(error);
                }
            }
        }
    }

    async fn open_once(cli_path: String, spec: SessionSpec) -> Result<Self, AgentBackendError> {
        let process = CodexAppServerProcess::spawn(
            &cli_path,
            &spec.session_id,
            Some(&spec.cwd),
            spec.base_branch.as_deref(),
        )
        .await
        .map_err(AgentBackendError::Other)?;
        let handle = process.handle();
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let state = Arc::new(Mutex::new(CodexRuntimeState {
            thread_id: None,
            turn_id: None,
            startup_error: None,
            cwd: spec.cwd.clone(),
            model: spec.model.clone(),
            permission_mode: spec.permission_mode,
            plan_mode: spec.plan_mode,
            permission_profile_id: spec.permission_profile_id.clone(),
            pending_methods: HashMap::new(),
            pending_client_methods: HashMap::new(),
        }));
        let closed = Arc::new(AtomicBool::new(false));
        let read_state = Arc::clone(&state);
        let read_closed = Arc::clone(&closed);
        let requested_resume_id = spec.resume.clone();
        let startup_request_id = 2;
        tokio::spawn(async move {
            read_loop(
                process,
                read_state,
                events_tx,
                read_closed,
                requested_resume_id,
                startup_request_id,
            )
            .await;
        });

        if let Err(error) = handle
            .write_json(&initialize_request(1, env!("CARGO_PKG_VERSION")))
            .await
        {
            closed.store(true, Ordering::Relaxed);
            handle.shutdown().await;
            return Err(AgentBackendError::Other(error));
        }
        if let Err(error) = handle.write_json(&initialized_notification()).await {
            closed.store(true, Ordering::Relaxed);
            handle.shutdown().await;
            return Err(AgentBackendError::Other(error));
        }
        let thread_request = if let Some(thread_id) = spec.resume.as_deref() {
            match build_thread_resume_request(startup_request_id, thread_id, &spec) {
                Ok(request) => request,
                Err(error) => {
                    closed.store(true, Ordering::Relaxed);
                    handle.shutdown().await;
                    return Err(error);
                }
            }
        } else {
            match build_thread_start_request(startup_request_id, &spec) {
                Ok(request) => request,
                Err(error) => {
                    closed.store(true, Ordering::Relaxed);
                    handle.shutdown().await;
                    return Err(error);
                }
            }
        };
        if let Err(error) = handle.write_json(&thread_request).await {
            closed.store(true, Ordering::Relaxed);
            handle.shutdown().await;
            return Err(AgentBackendError::Other(error));
        }

        Ok(Self {
            handle,
            state,
            events: StdMutex::new(Some(events_rx)),
            next_id: AtomicU64::new(100),
            closed,
        })
    }

    fn next_request_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }
}

#[async_trait::async_trait]
impl AgentSessionRuntime for CodexSessionRuntime {
    fn take_events(&mut self) -> Pin<Box<dyn Stream<Item = AgentRuntimeEvent> + Send>> {
        let Some(mut receiver) = self.events.lock().ok().and_then(|mut events| events.take())
        else {
            return Box::pin(stream::empty());
        };
        Box::pin(stream::poll_fn(move |cx| receiver.poll_recv(cx)))
    }

    async fn start_turn(&self, input: TurnInput) -> Result<(), AgentBackendError> {
        let thread_id = wait_for_thread_id(&self.state, Duration::from_secs(15)).await?;
        let request_id = self.next_request_id();
        let request = {
            let mut state = self.state.lock().await;
            state.permission_mode = input.permission_mode;
            state.plan_mode = input.plan_mode;
            state.permission_profile_id = input.permission_profile_id.clone();
            let request = build_turn_start_request(request_id, &thread_id, &state, input)?;
            state
                .pending_client_methods
                .insert(request_id, METHOD_TURN_START.to_string());
            request
        };
        if let Err(error) = self
            .handle
            .write_json(&request)
            .await
            .map_err(AgentBackendError::Other)
        {
            self.state
                .lock()
                .await
                .pending_client_methods
                .remove(&request_id);
            Err(error)
        } else {
            Ok(())
        }
    }

    async fn interrupt(&self) -> Result<(), AgentBackendError> {
        let (thread_id, turn_id) = {
            let state = self.state.lock().await;
            (state.thread_id.clone(), state.turn_id.clone())
        };
        let Some(thread_id) = thread_id else {
            return Ok(());
        };
        let Some(turn_id) = turn_id else {
            return Ok(());
        };
        let value = request(
            self.next_request_id(),
            METHOD_TURN_INTERRUPT,
            json!({
                "threadId": thread_id,
                "turnId": turn_id,
            }),
        );
        self.handle
            .write_json(&value)
            .await
            .map_err(AgentBackendError::Other)
    }

    async fn respond_permission(
        &self,
        response: PermissionResponse,
    ) -> Result<(), AgentBackendError> {
        let jsonrpc_id = response.request_id.parse::<u64>().map_err(|_| {
            AgentBackendError::Invalid(format!(
                "invalid Codex permission request id: {}",
                response.request_id
            ))
        })?;
        let source_method = {
            let mut state = self.state.lock().await;
            take_pending_method(&mut state, jsonrpc_id)?
        };
        let value = codex_permission_response(jsonrpc_id, &source_method, response);
        self.handle
            .write_json(&value)
            .await
            .map_err(AgentBackendError::Other)
    }

    async fn set_permission_mode(
        &self,
        mode: PermissionMode,
        plan_mode: bool,
    ) -> Result<(), AgentBackendError> {
        let (thread_id, cwd, permission_profile_id) = {
            let mut state = self.state.lock().await;
            state.permission_mode = mode;
            state.plan_mode = plan_mode;
            (
                state.thread_id.clone(),
                state.cwd.clone(),
                state.permission_profile_id.clone(),
            )
        };
        let Some(thread_id) = thread_id else {
            return Ok(());
        };
        let settings =
            codex_permission_settings(mode, plan_mode, permission_profile_id.as_deref(), &cwd);
        let mut params = json!({ "threadId": thread_id });
        params["permissions"] = settings.permissions.unwrap_or(Value::Null);
        if let Some(approval_policy) = settings.approval_policy {
            params["approvalPolicy"] = Value::String(approval_policy.to_string());
        }
        if let Some(sandbox_policy) = settings.sandbox_policy {
            params["sandboxPolicy"] = sandbox_policy;
        }
        self.handle
            .write_json(&request(
                self.next_request_id(),
                METHOD_THREAD_SETTINGS_UPDATE,
                params,
            ))
            .await
            .map_err(AgentBackendError::Other)
    }

    async fn set_model(&self, model: &ModelId) -> Result<(), AgentBackendError> {
        self.state.lock().await.model = model.clone();
        Ok(())
    }

    async fn set_session_title(&self, title: &str) -> Result<(), AgentBackendError> {
        let thread_id = { self.state.lock().await.thread_id.clone() };
        let Some(thread_id) = thread_id else {
            return Ok(());
        };
        self.handle
            .write_json(&request(
                self.next_request_id(),
                super::wire::METHOD_THREAD_NAME_SET,
                json!({
                    "threadId": thread_id,
                    "name": title,
                }),
            ))
            .await
            .map_err(AgentBackendError::Other)
    }

    async fn close(&self) {
        self.closed.store(true, Ordering::Relaxed);
        self.handle.shutdown().await;
    }
}

fn take_pending_method(
    state: &mut CodexRuntimeState,
    jsonrpc_id: u64,
) -> Result<String, AgentBackendError> {
    state.pending_methods.remove(&jsonrpc_id).ok_or_else(|| {
        AgentBackendError::Invalid(format!("unknown Codex permission request id: {jsonrpc_id}"))
    })
}

async fn read_loop(
    mut process: CodexAppServerProcess,
    state: Arc<Mutex<CodexRuntimeState>>,
    events_tx: mpsc::UnboundedSender<AgentRuntimeEvent>,
    closed: Arc<AtomicBool>,
    requested_resume_id: Option<String>,
    startup_request_id: u64,
) {
    let mut convert_state = CodexConvertState {
        requested_resume_id,
        startup_request_id: Some(startup_request_id),
        ..CodexConvertState::default()
    };
    loop {
        match process.next_json().await {
            Ok(Some(message)) => {
                if closed.load(Ordering::Relaxed) {
                    break;
                }
                match message_kind(&message) {
                    Some(AppServerMessageKind::Request { id, method }) => {
                        state.lock().await.pending_methods.insert(id, method);
                    }
                    Some(AppServerMessageKind::Response { id }) => {
                        if let Some(method) = state.lock().await.pending_client_methods.remove(&id)
                        {
                            convert_state.client_response_methods.insert(id, method);
                        }
                    }
                    _ => {}
                }
                let events = convert_jsonrpc_message(&message, &mut convert_state);
                {
                    state.lock().await.turn_id = convert_state.turn_id.clone();
                }
                for event in events {
                    {
                        let mut state = state.lock().await;
                        if let AgentRuntimeEvent::SessionEstablished {
                            backend_session_id, ..
                        } = &event
                        {
                            state.thread_id = Some(backend_session_id.clone());
                        }
                        if let AgentRuntimeEvent::BackendSessionCleared = &event {
                            state.thread_id = None;
                        }
                        if let AgentRuntimeEvent::Fatal { message } = &event {
                            state.startup_error = Some(message.clone());
                        }
                        if matches!(event, AgentRuntimeEvent::TurnCompleted(_)) {
                            state.pending_methods.clear();
                        }
                    }
                    let _ = events_tx.send(event);
                }
            }
            Ok(None) => {
                if !closed.load(Ordering::Relaxed) {
                    log::warn!("Codex app-server exited unexpectedly");
                    if state.lock().await.turn_id.is_some() {
                        let _ = events_tx.send(AgentRuntimeEvent::TurnCompleted(
                            crate::domain::agent_session::entities::TurnResult::Interrupted {
                                reason:
                                    crate::domain::agent_session::entities::InterruptReason::Crash,
                                error: Some(AGENT_PROCESS_EXITED_UNEXPECTEDLY.to_string()),
                            },
                        ));
                    }
                    let _ = events_tx.send(AgentRuntimeEvent::Fatal {
                        message: AGENT_PROCESS_EXITED_UNEXPECTEDLY.to_string(),
                    });
                }
                break;
            }
            Err(error) => {
                if !closed.load(Ordering::Relaxed) {
                    if state.lock().await.turn_id.is_some() {
                        let _ = events_tx.send(AgentRuntimeEvent::TurnCompleted(
                            crate::domain::agent_session::entities::TurnResult::Interrupted {
                                reason:
                                    crate::domain::agent_session::entities::InterruptReason::Crash,
                                error: Some(error.clone()),
                            },
                        ));
                    }
                    let _ = events_tx.send(AgentRuntimeEvent::Fatal { message: error });
                }
                break;
            }
        }
    }
    process.shutdown().await;
}

fn startup_timeout_for_spec(spec: &SessionSpec) -> Duration {
    spec.startup_timeout
        .unwrap_or(Duration::from_secs(30))
        .min(Duration::from_secs(300))
}

fn startup_max_retries_for_spec(spec: &SessionSpec) -> u32 {
    spec.startup_max_retries.unwrap_or(0).min(10)
}

async fn wait_for_thread_id(
    state: &Arc<Mutex<CodexRuntimeState>>,
    timeout: Duration,
) -> Result<String, AgentBackendError> {
    let started = std::time::Instant::now();
    loop {
        {
            let state = state.lock().await;
            if let Some(thread_id) = state.thread_id.clone() {
                return Ok(thread_id);
            }
            if let Some(error) = state.startup_error.clone() {
                return Err(AgentBackendError::Other(error));
            }
        }
        if started.elapsed() > timeout {
            return Err(AgentBackendError::StartupTimeout {
                retry_count: 0,
                max_retries: 0,
            });
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

fn build_thread_start_request(id: u64, spec: &SessionSpec) -> Result<Value, AgentBackendError> {
    let mut params = json!({
        "cwd": &spec.cwd,
        "runtimeWorkspaceRoots": [&spec.cwd],
        "threadSource": "user",
        "model": spec.model.as_str(),
    });
    apply_permission_settings(
        &mut params,
        spec.permission_mode,
        spec.plan_mode,
        spec.permission_profile_id.as_deref(),
        &spec.cwd,
    );
    if let Some(system_prompt) = spec
        .system_prompt
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        params["developerInstructions"] = Value::String(system_prompt.to_string());
    }
    Ok(request(id, METHOD_THREAD_START, params))
}

fn build_thread_resume_request(
    id: u64,
    thread_id: &str,
    spec: &SessionSpec,
) -> Result<Value, AgentBackendError> {
    let mut params = json!({
        "threadId": thread_id,
        "cwd": &spec.cwd,
        "runtimeWorkspaceRoots": [&spec.cwd],
        "model": spec.model.as_str(),
    });
    apply_permission_settings(
        &mut params,
        spec.permission_mode,
        spec.plan_mode,
        spec.permission_profile_id.as_deref(),
        &spec.cwd,
    );
    if let Some(system_prompt) = spec
        .system_prompt
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        params["developerInstructions"] = Value::String(system_prompt.to_string());
    }
    Ok(request(id, METHOD_THREAD_RESUME, params))
}

fn build_turn_start_request(
    id: u64,
    thread_id: &str,
    state: &CodexRuntimeState,
    input: TurnInput,
) -> Result<Value, AgentBackendError> {
    let mut params = json!({
        "threadId": thread_id,
        "cwd": &state.cwd,
        "input": codex_user_input(&input.prompt, &input.images),
        "runtimeWorkspaceRoots": [&state.cwd],
        "model": state.model.as_str(),
    });
    apply_permission_settings(
        &mut params,
        input.permission_mode,
        input.plan_mode,
        input.permission_profile_id.as_deref(),
        &state.cwd,
    );
    if let Some(system_prompt) = input
        .system_prompt
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        params["developerInstructions"] = Value::String(system_prompt.to_string());
    }
    if let Some(additional_context) = editor_context_value(input.editor_context.as_ref()) {
        params["additionalContext"] = additional_context;
    }
    Ok(request(id, METHOD_TURN_START, params))
}

fn apply_permission_settings(
    params: &mut Value,
    mode: PermissionMode,
    plan_mode: bool,
    permission_profile_id: Option<&str>,
    cwd: &str,
) {
    let settings = codex_permission_settings(mode, plan_mode, permission_profile_id, cwd);
    if let Some(permissions) = settings.permissions {
        params["permissions"] = permissions;
        return;
    }
    if plan_mode {
        params["collaborationMode"] = Value::String("plan".to_string());
    }
    if let Some(approval_policy) = settings.approval_policy {
        params["approvalPolicy"] = Value::String(approval_policy.to_string());
    }
    if let Some(sandbox_policy) = settings.sandbox_policy {
        params["sandboxPolicy"] = sandbox_policy;
    }
}

fn codex_user_input(prompt: &str, images: &[AttachmentPayload]) -> Vec<Value> {
    let mut input = Vec::new();
    if !prompt.is_empty() || images.is_empty() {
        input.push(json!({ "type": "text", "text": prompt }));
    }
    input.extend(images.iter().map(|image| {
        json!({
            "type": "image",
            "url": format!("data:{};base64,{}", image.media_type, image.data),
        })
    }));
    input
}

fn editor_context_value(context: Option<&EditorContext>) -> Option<Value> {
    let context = context?;
    if context.active_editor_path.is_none()
        && context.open_editor_paths.is_empty()
        && context.selection.is_none()
    {
        return None;
    }
    Some(json!({
        "activeEditorPath": &context.active_editor_path,
        "openEditorPaths": &context.open_editor_paths,
        "selection": context.selection.as_ref().map(|selection| json!({
            "filePath": &selection.file_path,
            "startLine": selection.start_line,
            "endLine": selection.end_line,
        })),
    }))
}

#[cfg(test)]
mod tests {
    use crate::domain::agent_session::value_objects::ModelId;
    use crate::infrastructure::agent_session::codex::wire;

    use super::*;

    #[cfg(unix)]
    fn write_fake_codex_cli(dir: &std::path::Path) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let path = dir.join("fake-codex");
        std::fs::write(
            &path,
            r#"#!/bin/sh
exec sleep 30
"#,
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).unwrap();
        path
    }

    #[cfg(unix)]
    fn assert_agent_process_registry_empty(data_dir: &std::path::Path) {
        let pid_dir = data_dir.join("agent-processes");
        let entries = match std::fs::read_dir(&pid_dir) {
            Ok(entries) => entries
                .map(|entry| entry.unwrap().path())
                .collect::<Vec<_>>(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => panic!("failed to read pid registry {}: {error}", pid_dir.display()),
        };
        assert!(
            entries.is_empty(),
            "startup cleanup should remove pid files: {entries:?}"
        );
    }

    fn spec(plan_mode: bool) -> SessionSpec {
        SessionSpec {
            session_id: "s1".to_string(),
            cwd: "/repo".to_string(),
            permission_mode: PermissionMode::Edit,
            plan_mode,
            permission_profile_id: None,
            model: ModelId::parse("gpt-5.5").unwrap(),
            system_prompt: Some("system".to_string()),
            resume: None,
            base_branch: Some("main".to_string()),
            startup_timeout: None,
            startup_max_retries: None,
            stale_timeout: None,
        }
    }

    #[test]
    fn test_thread_start_planは検証済み_string_collaboration_modeを使う() {
        let value = build_thread_start_request(7, &spec(true)).unwrap();

        assert_eq!(value["method"], METHOD_THREAD_START);
        assert_eq!(value["params"]["collaborationMode"], "plan");
        assert_eq!(value["params"]["approvalPolicy"], "on-request");
        assert_eq!(value["params"]["sandboxPolicy"]["type"], "readOnly");
        assert_eq!(value["params"]["developerInstructions"], "system");
    }

    #[test]
    fn test_turn_start_imagesは_data_urlを使う() {
        let state = CodexRuntimeState {
            thread_id: Some("thread".to_string()),
            turn_id: None,
            startup_error: None,
            cwd: "/repo".to_string(),
            model: ModelId::parse("gpt-5.5").unwrap(),
            permission_mode: PermissionMode::Ask,
            plan_mode: false,
            permission_profile_id: None,
            pending_methods: HashMap::new(),
            pending_client_methods: HashMap::new(),
        };
        let value = build_turn_start_request(
            8,
            "thread",
            &state,
            TurnInput {
                prompt: "hello".to_string(),
                images: vec![AttachmentPayload {
                    data: "abc".to_string(),
                    media_type: "image/png".to_string(),
                }],
                system_prompt: None,
                permission_mode: PermissionMode::Ask,
                plan_mode: false,
                permission_profile_id: None,
                editor_context: None,
            },
        )
        .unwrap();

        assert_eq!(value["method"], METHOD_TURN_START);
        assert_eq!(value["params"]["input"][1]["type"], "image");
        assert_eq!(
            value["params"]["input"][1]["url"],
            "data:image/png;base64,abc"
        );
    }

    #[test]
    fn test_startup_timeout既定とclampは旧規則を維持する() {
        let mut default_spec = spec(false);
        default_spec.startup_timeout = None;
        default_spec.startup_max_retries = None;
        assert_eq!(
            startup_timeout_for_spec(&default_spec),
            Duration::from_secs(30)
        );
        assert_eq!(startup_max_retries_for_spec(&default_spec), 0);

        let mut clamped = spec(false);
        clamped.startup_timeout = Some(Duration::from_secs(999));
        clamped.startup_max_retries = Some(99);
        assert_eq!(startup_timeout_for_spec(&clamped), Duration::from_secs(300));
        assert_eq!(startup_max_retries_for_spec(&clamped), 10);
    }

    #[test]
    fn test_take_pending_method_未知idは既定acceptへfallbackしない() {
        let mut state = CodexRuntimeState {
            thread_id: Some("thread".to_string()),
            turn_id: None,
            startup_error: None,
            cwd: "/repo".to_string(),
            model: ModelId::parse("gpt-5.5").unwrap(),
            permission_mode: PermissionMode::Ask,
            plan_mode: false,
            permission_profile_id: None,
            pending_methods: HashMap::new(),
            pending_client_methods: HashMap::new(),
        };

        assert!(matches!(
            take_pending_method(&mut state, 404),
            Err(AgentBackendError::Invalid(message)) if message.contains("unknown Codex permission request id")
        ));
        state
            .pending_methods
            .insert(7, wire::REQUEST_COMMAND_APPROVAL.to_string());
        assert_eq!(
            take_pending_method(&mut state, 7).unwrap(),
            wire::REQUEST_COMMAND_APPROVAL
        );
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn test_open_once_initialize_write失敗時にpid登録を削除する() {
        let _env_lock = crate::test_support::TEST_ENV_LOCK.lock().unwrap();
        let data_dir = tempfile::tempdir().unwrap();
        let _env_guard =
            crate::test_support::EnvVarGuard::set_path("RELEASH_DATA_DIR", data_dir.path());
        let _fail_write_guard = crate::test_support::EnvVarGuard::set_value(
            "RELEASH_TEST_FAIL_CODEX_APP_SERVER_STDIN_WRITE",
            "1",
        );
        let cli_path = write_fake_codex_cli(data_dir.path());
        let mut spec = spec(false);
        spec.session_id = "codex-startup-cleanup".to_string();
        spec.cwd = data_dir.path().to_string_lossy().to_string();

        let result =
            CodexSessionRuntime::open_once(cli_path.to_string_lossy().to_string(), spec).await;

        assert!(matches!(
            result,
            Err(AgentBackendError::Other(message))
                if message.contains("failed to write codex app-server stdin")
                    || message.contains("codex app-server stdin is closed")
                    || message.contains("injected codex app-server stdin write failure")
        ));
        assert_agent_process_registry_empty(data_dir.path());
    }
}
