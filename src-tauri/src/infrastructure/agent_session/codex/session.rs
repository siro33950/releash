use std::collections::HashMap;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use futures_util::stream::{self, Stream};
use serde_json::{json, Value};
use tokio::io::AsyncBufRead;
use tokio::sync::{mpsc, Mutex};

use crate::domain::agent_session::entities::{AttachmentPayload, MessagePart, PermissionResponse};
use crate::domain::agent_session::gateway::{
    AgentBackendError, AgentRuntimeEvent, AgentSessionRuntime, SessionSpec, TurnInput,
};
use crate::domain::agent_session::value_objects::{EditorContext, ModelId, PermissionMode};
use crate::infrastructure::agent_session::stdout_line_reader::{
    StdoutDiagnostics, StdoutItem, StdoutLineReader,
};

use super::app_server::{CodexAppServerHandle, CodexAppServerProcess};
use super::convert::{convert_jsonrpc_message, CodexConvertState};
use super::permission::{codex_permission_response, codex_permission_settings};
use super::wire::{
    initialize_request, initialized_notification, message_kind, request, AppServerMessageKind,
    PendingClientRequests, METHOD_INITIALIZE, METHOD_THREAD_RESUME, METHOD_THREAD_START,
    METHOD_TURN_INTERRUPT, METHOD_TURN_START,
};

const AGENT_PROCESS_EXITED_UNEXPECTEDLY: &str = "Agent process exited unexpectedly";

pub(crate) struct CodexSessionRuntime {
    handle: CodexAppServerHandle,
    state: Arc<Mutex<CodexRuntimeState>>,
    events: StdMutex<Option<mpsc::UnboundedReceiver<AgentRuntimeEvent>>>,
    read_task: StdMutex<Option<tokio::task::JoinHandle<()>>>,
    next_id: Arc<AtomicU64>,
    closed: Arc<AtomicBool>,
}

struct ReservedInterruptSender<W = CodexAppServerHandle> {
    handle: W,
    next_id: Arc<AtomicU64>,
}

#[async_trait::async_trait]
trait JsonWriteSink: Send + Sync {
    async fn write_json(&self, value: &Value) -> Result<(), String>;
}

#[async_trait::async_trait]
impl JsonWriteSink for CodexAppServerHandle {
    async fn write_json(&self, value: &Value) -> Result<(), String> {
        CodexAppServerHandle::write_json(self, value).await
    }
}

#[derive(Debug)]
struct CodexRuntimeState {
    thread_id: Option<String>,
    turn_id: Option<String>,
    active_turn_start_request_id: Option<u64>,
    interrupt_requested_for: Option<u64>,
    turn_start_handshake_active: bool,
    interrupt_requested_during_start_handshake: bool,
    startup_error: Option<String>,
    requested_resume_id: Option<String>,
    resume_rejected: bool,
    cwd: String,
    model: ModelId,
    permission_profile_id: Option<String>,
    pending_methods: HashMap<u64, String>,
    pending_client_requests: PendingClientRequests,
    stdout_diagnostics: StdoutDiagnostics,
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
        let mut process = CodexAppServerProcess::spawn(
            &cli_path,
            &spec.session_id,
            Some(&spec.cwd),
            spec.base_branch.as_deref(),
            &spec.extra_env,
        )
        .await
        .map_err(AgentBackendError::Other)?;
        let handle = process.handle();
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let startup_request_id = 2;
        let startup_method = if spec.resume.is_some() {
            METHOD_THREAD_RESUME
        } else {
            METHOD_THREAD_START
        };
        let mut pending_client_requests = PendingClientRequests::default();
        pending_client_requests.register(1, METHOD_INITIALIZE);
        pending_client_requests.register(startup_request_id, startup_method);
        let state = Arc::new(Mutex::new(CodexRuntimeState {
            thread_id: None,
            turn_id: None,
            active_turn_start_request_id: None,
            interrupt_requested_for: None,
            turn_start_handshake_active: false,
            interrupt_requested_during_start_handshake: false,
            startup_error: None,
            requested_resume_id: spec.resume.clone(),
            resume_rejected: false,
            cwd: spec.cwd.clone(),
            model: spec.model.clone(),
            permission_profile_id: spec.permission_profile_id.clone(),
            pending_methods: HashMap::new(),
            pending_client_requests,
            stdout_diagnostics: StdoutDiagnostics::default(),
        }));
        let closed = Arc::new(AtomicBool::new(false));
        let read_state = Arc::clone(&state);
        let read_closed = Arc::clone(&closed);
        let requested_resume_id = spec.resume.clone();
        let next_id = Arc::new(AtomicU64::new(100));
        let reserved_interrupt_sender = ReservedInterruptSender {
            handle: handle.clone(),
            next_id: Arc::clone(&next_id),
        };
        let mut read_task = Some(tokio::spawn(async move {
            read_loop(
                process.stdout_mut(),
                reserved_interrupt_sender,
                read_state,
                events_tx,
                read_closed,
                requested_resume_id,
            )
            .await;
            process.shutdown().await;
        }));

        if let Err(error) = handle
            .write_json(&initialize_request(1, env!("CARGO_PKG_VERSION")))
            .await
        {
            shutdown_opening_process(&handle, &closed, &mut read_task).await;
            return Err(AgentBackendError::Other(error));
        }
        if let Err(error) = handle.write_json(&initialized_notification()).await {
            shutdown_opening_process(&handle, &closed, &mut read_task).await;
            return Err(AgentBackendError::Other(error));
        }
        let thread_request = if let Some(thread_id) = spec.resume.as_deref() {
            match build_thread_resume_request(startup_request_id, thread_id, &spec) {
                Ok(request) => request,
                Err(error) => {
                    shutdown_opening_process(&handle, &closed, &mut read_task).await;
                    return Err(error);
                }
            }
        } else {
            match build_thread_start_request(startup_request_id, &spec) {
                Ok(request) => request,
                Err(error) => {
                    shutdown_opening_process(&handle, &closed, &mut read_task).await;
                    return Err(error);
                }
            }
        };
        if let Err(error) = handle.write_json(&thread_request).await {
            shutdown_opening_process(&handle, &closed, &mut read_task).await;
            return Err(AgentBackendError::Other(error));
        }

        Ok(Self {
            handle,
            state,
            events: StdMutex::new(Some(events_rx)),
            read_task: StdMutex::new(read_task),
            next_id,
            closed,
        })
    }

    fn next_request_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    async fn write_tracked_request(
        &self,
        request_id: u64,
        method: &str,
        value: &Value,
    ) -> Result<(), AgentBackendError> {
        self.state
            .lock()
            .await
            .pending_client_requests
            .register(request_id, method);
        if let Err(error) = self.handle.write_json(value).await {
            self.state
                .lock()
                .await
                .pending_client_requests
                .remove(request_id);
            return Err(AgentBackendError::Other(error));
        }
        Ok(())
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
        {
            let mut state = self.state.lock().await;
            state.begin_turn_start_handshake();
        }
        let thread_id = match wait_for_thread_id(&self.state, Duration::from_secs(15)).await {
            Ok(thread_id) => thread_id,
            Err(error) => {
                self.state.lock().await.clear_turn_start_handshake();
                return Err(error);
            }
        };
        let request_id = self.next_request_id();
        let request = {
            let mut state = self.state.lock().await;
            state.permission_profile_id = input.permission_profile_id.clone();
            state.stdout_diagnostics.reset();
            let request = match build_turn_start_request(request_id, &thread_id, &state, input) {
                Ok(request) => request,
                Err(error) => {
                    state.clear_turn_start_handshake();
                    return Err(error);
                }
            };
            state.register_turn_start_request(request_id);
            request
        };
        if let Err(error) = self
            .write_tracked_request(request_id, METHOD_TURN_START, &request)
            .await
        {
            self.state
                .lock()
                .await
                .clear_failed_turn_start_request(request_id);
            Err(error)
        } else {
            Ok(())
        }
    }

    async fn interrupt(&self) -> Result<(), AgentBackendError> {
        let request_id = self.next_request_id();
        let value = {
            let mut state = self.state.lock().await;
            prepare_interrupt_request(&mut state, request_id)
        };
        let Some(value) = value else {
            return Ok(());
        };
        self.write_tracked_request(request_id, METHOD_TURN_INTERRUPT, &value)
            .await
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

    async fn set_model(&self, model: &ModelId) -> Result<(), AgentBackendError> {
        self.state.lock().await.model = model.clone();
        Ok(())
    }

    async fn close(&self) {
        self.closed.store(true, Ordering::Relaxed);
        self.handle.shutdown().await;
        let read_task = self
            .read_task
            .lock()
            .ok()
            .and_then(|mut read_task| read_task.take());
        if let Some(read_task) = read_task {
            if let Err(error) = read_task.await {
                log::warn!("failed to join Codex stdout reader: {error}");
            }
        }
    }
}

async fn shutdown_opening_process(
    handle: &CodexAppServerHandle,
    closed: &Arc<AtomicBool>,
    read_task: &mut Option<tokio::task::JoinHandle<()>>,
) {
    closed.store(true, Ordering::Relaxed);
    handle.shutdown().await;
    if let Some(read_task) = read_task.take() {
        if let Err(error) = read_task.await {
            log::warn!("failed to join Codex stdout reader: {error}");
        }
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

async fn read_loop<R, W>(
    stdout: &mut StdoutLineReader<R>,
    reserved_interrupt_sender: ReservedInterruptSender<W>,
    state: Arc<Mutex<CodexRuntimeState>>,
    events_tx: mpsc::UnboundedSender<AgentRuntimeEvent>,
    closed: Arc<AtomicBool>,
    requested_resume_id: Option<String>,
) where
    R: AsyncBufRead + Unpin,
    W: JsonWriteSink,
{
    let mut convert_state = CodexConvertState {
        requested_resume_id,
        ..CodexConvertState::default()
    };
    loop {
        match stdout.next().await {
            Ok(Some(StdoutItem::NonJson { probe })) => {
                if closed.load(Ordering::Relaxed) {
                    break;
                }
                let mut state = state.lock().await;
                state
                    .stdout_diagnostics
                    .record_non_json_skip("codex", &probe);
            }
            Ok(Some(StdoutItem::Oversize { probe })) => {
                if closed.load(Ordering::Relaxed) {
                    break;
                }
                let event = {
                    let mut state = state.lock().await;
                    let content = state
                        .stdout_diagnostics
                        .record_oversize_drop("codex", &probe);
                    oversize_drop_event(content)
                };
                let _ = events_tx.send(event);
            }
            Ok(Some(StdoutItem::Json(message))) => {
                if closed.load(Ordering::Relaxed) {
                    break;
                }
                match message_kind(&message) {
                    Some(AppServerMessageKind::Request { id, method }) => {
                        state.lock().await.pending_methods.insert(id, method);
                    }
                    Some(AppServerMessageKind::Response { .. }) => {
                        let pending_response = {
                            let mut state = state.lock().await;
                            state.pending_client_requests.take_response(&message)
                        };
                        match pending_response {
                            Ok(Some(response)) => {
                                convert_state
                                    .client_response_methods
                                    .insert(response.id, response.method);
                            }
                            Ok(None) => {}
                            Err(error) => {
                                emit_read_failure(&state, &events_tx, &closed, &error).await;
                                break;
                            }
                        }
                    }
                    _ => {}
                }
                let events = convert_jsonrpc_message(&message, &mut convert_state);
                let reserved_interrupt = {
                    let mut state = state.lock().await;
                    state.turn_id = convert_state.turn_id.clone();
                    if state.interrupt_requested_for.is_some() {
                        let request_id = reserved_interrupt_sender
                            .next_id
                            .fetch_add(1, Ordering::Relaxed);
                        let request = take_reserved_interrupt_request(&mut state, request_id);
                        if request.is_some() {
                            state
                                .pending_client_requests
                                .register(request_id, METHOD_TURN_INTERRUPT);
                        }
                        request.map(|request| (request_id, request))
                    } else {
                        None
                    }
                };
                if let Some((request_id, request)) = reserved_interrupt {
                    if let Err(error) = reserved_interrupt_sender.handle.write_json(&request).await
                    {
                        state
                            .lock()
                            .await
                            .pending_client_requests
                            .remove(request_id);
                        log::warn!("failed to send reserved Codex interrupt: {error}");
                    }
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
                            state.resume_rejected = true;
                        }
                        if let AgentRuntimeEvent::Fatal { message } = &event {
                            state.startup_error = Some(message.clone());
                        }
                        if matches!(event, AgentRuntimeEvent::TurnCompleted(_)) {
                            reset_completed_turn_state(&mut state);
                        }
                    }
                    let _ = events_tx.send(event);
                }
            }
            Ok(None) => {
                if closed.load(Ordering::Relaxed) {
                    break;
                }
                log::warn!("Codex app-server exited unexpectedly");
                emit_read_failure(
                    &state,
                    &events_tx,
                    &closed,
                    AGENT_PROCESS_EXITED_UNEXPECTEDLY,
                )
                .await;
                break;
            }
            Err(error) => {
                emit_read_failure(&state, &events_tx, &closed, &error).await;
                break;
            }
        }
    }
}

fn oversize_drop_event(content: String) -> AgentRuntimeEvent {
    AgentRuntimeEvent::PartsMerged(vec![MessagePart::Error {
        content,
        parent_tool_use_id: None,
    }])
}

async fn emit_read_failure(
    state: &Arc<Mutex<CodexRuntimeState>>,
    events_tx: &mpsc::UnboundedSender<AgentRuntimeEvent>,
    closed: &AtomicBool,
    error: &str,
) {
    if closed.load(Ordering::Relaxed) {
        return;
    }
    let turn_active = {
        let mut state = state.lock().await;
        state.startup_error = Some(error.to_string());
        state.turn_id.is_some()
    };
    if turn_active {
        let _ = events_tx.send(AgentRuntimeEvent::TurnCompleted(
            crate::domain::agent_session::entities::TurnResult::Interrupted {
                reason: crate::domain::agent_session::entities::InterruptReason::Crash,
                error: Some(error.to_string()),
            },
        ));
    }
    let _ = events_tx.send(AgentRuntimeEvent::Fatal {
        message: error.to_string(),
    });
}

fn prepare_interrupt_request(state: &mut CodexRuntimeState, request_id: u64) -> Option<Value> {
    let Some(thread_id) = state.thread_id.clone() else {
        state.reserve_interrupt_for_start_handshake();
        return None;
    };
    let Some(turn_id) = state.turn_id.clone() else {
        if let Some(active_request_id) = state.active_turn_start_request_id {
            state.interrupt_requested_for = Some(active_request_id);
        } else {
            state.reserve_interrupt_for_start_handshake();
        }
        return None;
    };
    state.interrupt_requested_for = None;
    Some(turn_interrupt_request(request_id, thread_id, turn_id))
}

fn take_reserved_interrupt_request(
    state: &mut CodexRuntimeState,
    request_id: u64,
) -> Option<Value> {
    let requested_for = state.interrupt_requested_for?;
    if state.active_turn_start_request_id != Some(requested_for) {
        state.interrupt_requested_for = None;
        return None;
    }
    prepare_interrupt_request(state, request_id)
}

fn turn_interrupt_request(request_id: u64, thread_id: String, turn_id: String) -> Value {
    request(
        request_id,
        METHOD_TURN_INTERRUPT,
        json!({
            "threadId": thread_id,
            "turnId": turn_id,
        }),
    )
}

fn reset_completed_turn_state(state: &mut CodexRuntimeState) {
    state.turn_id = None;
    state.active_turn_start_request_id = None;
    state.interrupt_requested_for = None;
    state.turn_start_handshake_active = false;
    state.interrupt_requested_during_start_handshake = false;
    state.pending_methods.clear();
}

impl CodexRuntimeState {
    fn begin_turn_start_handshake(&mut self) {
        self.turn_start_handshake_active = true;
        self.interrupt_requested_during_start_handshake = false;
    }

    fn reserve_interrupt_for_start_handshake(&mut self) {
        if self.turn_start_handshake_active {
            self.interrupt_requested_during_start_handshake = true;
        }
    }

    fn register_turn_start_request(&mut self, request_id: u64) {
        self.active_turn_start_request_id = Some(request_id);
        self.interrupt_requested_for = self
            .interrupt_requested_during_start_handshake
            .then_some(request_id);
        self.turn_start_handshake_active = false;
        self.interrupt_requested_during_start_handshake = false;
    }

    fn clear_turn_start_handshake(&mut self) {
        self.turn_start_handshake_active = false;
        self.interrupt_requested_during_start_handshake = false;
    }

    fn clear_failed_turn_start_request(&mut self, request_id: u64) {
        self.pending_client_requests.remove(request_id);
        if self.active_turn_start_request_id == Some(request_id) {
            self.active_turn_start_request_id = None;
            self.interrupt_requested_for = None;
            self.clear_turn_start_handshake();
        }
    }
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
                if state.resume_rejected {
                    if let Some(requested_resume_id) = state.requested_resume_id.clone() {
                        return Err(AgentBackendError::BackendSessionLost {
                            requested_resume_id,
                        });
                    }
                }
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
    use crate::domain::agent_session::entities::{InterruptReason, TurnResult};
    use crate::domain::agent_session::value_objects::ModelId;
    use crate::infrastructure::agent_session::codex::wire;
    use crate::infrastructure::agent_session::stdout_line_reader::{LineProbe, StdoutLineReader};
    use tokio::io::AsyncWriteExt;

    use super::*;

    struct ThreadLogCapture {
        thread_id: StdMutex<Option<std::thread::ThreadId>>,
        messages: StdMutex<Vec<String>>,
    }

    impl log::Log for ThreadLogCapture {
        fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
            metadata.level() <= log::Level::Warn
        }

        fn log(&self, record: &log::Record<'_>) {
            if self.enabled(record.metadata())
                && self.thread_id.lock().unwrap().as_ref() == Some(&std::thread::current().id())
            {
                self.messages
                    .lock()
                    .unwrap()
                    .push(record.args().to_string());
            }
        }

        fn flush(&self) {}
    }

    static TEST_LOG_CAPTURE: ThreadLogCapture = ThreadLogCapture {
        thread_id: StdMutex::new(None),
        messages: StdMutex::new(Vec::new()),
    };
    static TEST_LOG_CAPTURE_INIT: std::sync::Once = std::sync::Once::new();

    #[derive(Clone, Default)]
    struct RecordingJsonWriteSink {
        writes: Arc<Mutex<Vec<Value>>>,
    }

    #[async_trait::async_trait]
    impl JsonWriteSink for RecordingJsonWriteSink {
        async fn write_json(&self, value: &Value) -> Result<(), String> {
            self.writes.lock().await.push(value.clone());
            Ok(())
        }
    }

    fn test_interrupt_sender() -> ReservedInterruptSender<RecordingJsonWriteSink> {
        ReservedInterruptSender {
            handle: RecordingJsonWriteSink::default(),
            next_id: Arc::new(AtomicU64::new(100)),
        }
    }

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
            model: ModelId::parse("gpt-5.6-sol").unwrap(),
            system_prompt: Some("system".to_string()),
            resume: None,
            base_branch: Some("main".to_string()),
            startup_timeout: None,
            startup_max_retries: None,
            stale_timeout: None,
            extra_env: Vec::new(),
        }
    }

    fn runtime_state() -> CodexRuntimeState {
        CodexRuntimeState {
            thread_id: Some("thread".to_string()),
            turn_id: Some("turn".to_string()),
            active_turn_start_request_id: None,
            interrupt_requested_for: None,
            turn_start_handshake_active: false,
            interrupt_requested_during_start_handshake: false,
            startup_error: None,
            requested_resume_id: None,
            resume_rejected: false,
            cwd: "/repo".to_string(),
            model: ModelId::parse("gpt-5.6-sol").unwrap(),
            permission_profile_id: None,
            pending_methods: HashMap::new(),
            pending_client_requests: PendingClientRequests::default(),
            stdout_diagnostics: StdoutDiagnostics::default(),
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
        let mut state = runtime_state();
        state.turn_id = None;
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
            active_turn_start_request_id: None,
            interrupt_requested_for: None,
            turn_start_handshake_active: false,
            interrupt_requested_during_start_handshake: false,
            startup_error: None,
            requested_resume_id: None,
            resume_rejected: false,
            cwd: "/repo".to_string(),
            model: ModelId::parse("gpt-5.6-sol").unwrap(),
            permission_profile_id: None,
            pending_methods: HashMap::new(),
            pending_client_requests: PendingClientRequests::default(),
            stdout_diagnostics: StdoutDiagnostics::default(),
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

    #[tokio::test]
    async fn test_startup_response必須field欠落はread_loopで失敗する() {
        let input = br#"{"id":2}
"#;
        let mut stdout = StdoutLineReader::new(tokio::io::BufReader::new(&input[..]));
        let mut runtime_state = runtime_state();
        runtime_state.thread_id = None;
        runtime_state.turn_id = None;
        runtime_state
            .pending_client_requests
            .register(2, METHOD_THREAD_START);
        let state = Arc::new(Mutex::new(runtime_state));
        let (events_tx, mut events_rx) = mpsc::unbounded_channel();

        read_loop(
            &mut stdout,
            test_interrupt_sender(),
            Arc::clone(&state),
            events_tx,
            Arc::new(AtomicBool::new(false)),
            None,
        )
        .await;

        assert!(state
            .lock()
            .await
            .startup_error
            .as_deref()
            .is_some_and(|message| message.contains("expected exactly one")));
        assert!(matches!(
            events_rx.try_recv().unwrap(),
            AgentRuntimeEvent::Fatal { message }
                if message.contains("expected exactly one of result or error")
        ));
    }

    #[tokio::test]
    async fn test_wait_for_thread_id_resume拒否は_backend_session_lostを返す() {
        let mut runtime_state = runtime_state();
        runtime_state.thread_id = None;
        runtime_state.turn_id = None;
        runtime_state.startup_error = Some("not found".to_string());
        runtime_state.requested_resume_id = Some("thread-old".to_string());
        runtime_state.resume_rejected = true;
        let state = Arc::new(Mutex::new(runtime_state));

        assert!(matches!(
            wait_for_thread_id(&state, Duration::from_millis(1)).await,
            Err(AgentBackendError::BackendSessionLost { requested_resume_id })
                if requested_resume_id == "thread-old"
        ));
    }

    #[tokio::test]
    async fn test_turn_start_response必須field欠落はread_loopで失敗する() {
        let input = br#"{"id":100}
"#;
        let mut stdout = StdoutLineReader::new(tokio::io::BufReader::new(&input[..]));
        let mut runtime_state = runtime_state();
        runtime_state
            .pending_client_requests
            .register(100, METHOD_TURN_START);
        let state = Arc::new(Mutex::new(runtime_state));
        let (events_tx, mut events_rx) = mpsc::unbounded_channel();

        read_loop(
            &mut stdout,
            test_interrupt_sender(),
            state,
            events_tx,
            Arc::new(AtomicBool::new(false)),
            None,
        )
        .await;

        assert!(matches!(
            events_rx.try_recv().unwrap(),
            AgentRuntimeEvent::TurnCompleted(TurnResult::Interrupted {
                reason: InterruptReason::Crash,
                error: Some(message),
            }) if message.contains("expected exactly one of result or error")
        ));
        assert!(matches!(
            events_rx.try_recv().unwrap(),
            AgentRuntimeEvent::Fatal { .. }
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_read_loop_eofは_closed状態に応じて予期しない終了を報告する() {
        TEST_LOG_CAPTURE_INIT.call_once(|| {
            log::set_logger(&TEST_LOG_CAPTURE).unwrap();
            log::set_max_level(log::LevelFilter::Warn);
        });
        *TEST_LOG_CAPTURE.thread_id.lock().unwrap() = Some(std::thread::current().id());

        let input = b"";
        let mut stdout = StdoutLineReader::new(tokio::io::BufReader::new(&input[..]));
        let state = Arc::new(Mutex::new(runtime_state()));
        let (events_tx, mut events_rx) = mpsc::unbounded_channel();
        read_loop(
            &mut stdout,
            test_interrupt_sender(),
            state,
            events_tx,
            Arc::new(AtomicBool::new(false)),
            None,
        )
        .await;

        assert!(matches!(
            events_rx.try_recv().unwrap(),
            AgentRuntimeEvent::TurnCompleted(TurnResult::Interrupted {
                reason: InterruptReason::Crash,
                ..
            })
        ));
        assert!(matches!(
            events_rx.try_recv().unwrap(),
            AgentRuntimeEvent::Fatal { .. }
        ));
        assert!(TEST_LOG_CAPTURE
            .messages
            .lock()
            .unwrap()
            .iter()
            .any(|message| message == "Codex app-server exited unexpectedly"));

        TEST_LOG_CAPTURE.messages.lock().unwrap().clear();
        let mut stdout = StdoutLineReader::new(tokio::io::BufReader::new(&input[..]));
        let state = Arc::new(Mutex::new(runtime_state()));
        let (events_tx, mut events_rx) = mpsc::unbounded_channel();
        read_loop(
            &mut stdout,
            test_interrupt_sender(),
            state,
            events_tx,
            Arc::new(AtomicBool::new(true)),
            None,
        )
        .await;

        assert!(events_rx.try_recv().is_err());
        assert!(TEST_LOG_CAPTURE.messages.lock().unwrap().is_empty());
        *TEST_LOG_CAPTURE.thread_id.lock().unwrap() = None;
    }

    #[tokio::test]
    async fn test_codex_read_loopは_mixed_stdout_fixture後も処理を継続する() {
        let fixture =
            include_bytes!("../../../../tests/fixtures/agent_session/mixed_stdout_codex.jsonl");
        let (mut writer, reader) = tokio::io::duplex(4096);
        let mut stdout = StdoutLineReader::with_max_line_bytes(
            tokio::io::BufReader::with_capacity(64, reader),
            256,
        );
        let state = Arc::new(Mutex::new(runtime_state()));
        let read_state = Arc::clone(&state);
        let (events_tx, mut events_rx) = mpsc::unbounded_channel();
        let closed = Arc::new(AtomicBool::new(false));
        let read_closed = Arc::clone(&closed);
        let read_task = tokio::spawn(async move {
            read_loop(
                &mut stdout,
                test_interrupt_sender(),
                read_state,
                events_tx,
                read_closed,
                None,
            )
            .await;
        });

        writer.write_all(fixture).await.unwrap();
        let events = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            let mut events = Vec::new();
            loop {
                let event = events_rx.recv().await.unwrap();
                let processed_following_json = matches!(
                    &event,
                    AgentRuntimeEvent::PartsMerged(parts)
                        if parts.iter().any(|part| matches!(
                            part,
                            MessagePart::Text { content, .. } if content == "ok"
                        ))
                );
                events.push(event);
                if processed_following_json {
                    return events;
                }
            }
        })
        .await
        .unwrap();

        let state = state.lock().await;
        assert_eq!(state.stdout_diagnostics.skipped_non_json_count(), 2);
        assert_eq!(state.stdout_diagnostics.oversize_dropped_count(), 1);
        assert!(events.iter().any(|event| matches!(
            event,
            AgentRuntimeEvent::PartsMerged(parts)
                if parts.iter().any(|part| matches!(
                    part,
                    MessagePart::Error { content, .. }
                        if content.contains("推定種別: item/agentMessage/delta")
                ))
        )));
        assert!(!events.iter().any(|event| matches!(
            event,
            AgentRuntimeEvent::Fatal { .. }
                | AgentRuntimeEvent::TurnCompleted(TurnResult::Interrupted {
                    reason: InterruptReason::Crash,
                    ..
                })
        )));
        assert!(!read_task.is_finished());
        drop(state);
        closed.store(true, Ordering::Relaxed);
        read_task.abort();
    }

    #[tokio::test]
    async fn test_codex_read_loopは_pending中の非jsonをskipして応答と後続通知を処理する() {
        let input = br#"diagnostic output while request is pending
{"id":100,"result":{}}
{"method":"item/agentMessage/delta","params":{"delta":"after pending response"}}
"#;
        let (mut writer, reader) = tokio::io::duplex(1024);
        let mut stdout = StdoutLineReader::new(tokio::io::BufReader::new(reader));
        let mut runtime_state = runtime_state();
        runtime_state
            .pending_client_requests
            .register(100, METHOD_TURN_START);
        let state = Arc::new(Mutex::new(runtime_state));
        let read_state = Arc::clone(&state);
        let (events_tx, mut events_rx) = mpsc::unbounded_channel();
        let closed = Arc::new(AtomicBool::new(false));
        let read_closed = Arc::clone(&closed);
        let read_task = tokio::spawn(async move {
            read_loop(
                &mut stdout,
                test_interrupt_sender(),
                read_state,
                events_tx,
                read_closed,
                None,
            )
            .await;
        });

        writer.write_all(input).await.unwrap();
        let event = tokio::time::timeout(std::time::Duration::from_secs(1), events_rx.recv())
            .await
            .unwrap()
            .unwrap();

        assert!(matches!(
            event,
            AgentRuntimeEvent::PartsMerged(parts)
                if parts.iter().any(|part| matches!(
                    part,
                    MessagePart::Text { content, .. } if content == "after pending response"
                ))
        ));
        assert!(events_rx.try_recv().is_err());
        let mut state = state.lock().await;
        assert_eq!(state.stdout_diagnostics.skipped_non_json_count(), 1);
        assert!(state
            .pending_client_requests
            .take_response(&json!({ "id": 100, "result": {} }))
            .unwrap()
            .is_none());
        assert!(!read_task.is_finished());
        drop(state);
        closed.store(true, Ordering::Relaxed);
        read_task.abort();
    }

    #[test]
    fn test_codex_stdout診断カウントは次turn開始時にリセットする() {
        let mut state = runtime_state();
        let probe = LineProbe {
            kind_hint: None,
            bytes: 9 * 1024 * 1024,
        };
        let _ = state
            .stdout_diagnostics
            .record_oversize_drop("codex", &probe);
        state
            .stdout_diagnostics
            .record_non_json_skip("codex", &probe);

        state.stdout_diagnostics.reset();

        assert_eq!(state.stdout_diagnostics.oversize_dropped_count(), 0);
        assert_eq!(state.stdout_diagnostics.skipped_non_json_count(), 0);
    }

    #[tokio::test]
    async fn test_wait_for_thread_id新規sessionのstartup_errorは_otherを維持する() {
        let mut runtime_state = runtime_state();
        runtime_state.thread_id = None;
        runtime_state.turn_id = None;
        runtime_state.startup_error = Some("bad api key".to_string());
        let state = Arc::new(Mutex::new(runtime_state));

        assert!(matches!(
            wait_for_thread_id(&state, Duration::from_millis(1)).await,
            Err(AgentBackendError::Other(message)) if message == "bad api key"
        ));
    }

    #[tokio::test]
    async fn test_wait_for_thread_id_resume中の非thread_errorは_otherを維持する() {
        let mut runtime_state = runtime_state();
        runtime_state.thread_id = None;
        runtime_state.turn_id = None;
        runtime_state.startup_error = Some("bad api key".to_string());
        runtime_state.requested_resume_id = Some("thread-old".to_string());
        let state = Arc::new(Mutex::new(runtime_state));

        assert!(matches!(
            wait_for_thread_id(&state, Duration::from_millis(1)).await,
            Err(AgentBackendError::Other(message)) if message == "bad api key"
        ));
    }

    #[test]
    fn interrupt_before_turn_started_is_reserved_without_a_write_request() {
        let mut state = runtime_state();
        state.thread_id = Some("thread-1".to_string());
        state.turn_id = None;
        state.active_turn_start_request_id = Some(7);

        let request = prepare_interrupt_request(&mut state, 100);

        assert!(request.is_none());
        assert_eq!(state.interrupt_requested_for, Some(7));
    }

    #[test]
    fn interrupt_during_start_handshake_is_carried_to_the_registered_request() {
        let mut state = runtime_state();
        state.thread_id = None;
        state.turn_id = None;
        state.begin_turn_start_handshake();

        assert!(prepare_interrupt_request(&mut state, 100).is_none());
        assert!(state.interrupt_requested_during_start_handshake);

        state.thread_id = Some("thread-1".to_string());
        state.register_turn_start_request(7);

        assert_eq!(state.active_turn_start_request_id, Some(7));
        assert_eq!(state.interrupt_requested_for, Some(7));
        assert!(!state.turn_start_handshake_active);
        assert!(!state.interrupt_requested_during_start_handshake);
    }

    #[test]
    fn interrupt_after_provider_terminal_does_not_reserve_for_the_next_turn() {
        let mut state = runtime_state();
        state.turn_id = None;

        assert!(prepare_interrupt_request(&mut state, 100).is_none());
        assert_eq!(state.interrupt_requested_for, None);
    }

    #[test]
    fn turn_started_consumes_the_reserved_interrupt_immediately() {
        let mut state = runtime_state();
        state.thread_id = Some("thread-1".to_string());
        state.turn_id = Some("turn-1".to_string());
        state.active_turn_start_request_id = Some(7);
        state.interrupt_requested_for = Some(7);

        let request = take_reserved_interrupt_request(&mut state, 101).unwrap();

        assert_eq!(request["method"], METHOD_TURN_INTERRUPT);
        assert_eq!(request["params"]["threadId"], "thread-1");
        assert_eq!(request["params"]["turnId"], "turn-1");
        assert_eq!(state.interrupt_requested_for, None);
    }

    #[tokio::test]
    async fn read_loop_writes_the_reserved_interrupt_exactly_once_after_turn_started() {
        let input = br#"{"method":"turn/started","params":{"turn":{"id":"turn-1"}}}
{"method":"item/agentMessage/delta","params":{"delta":"after interrupt"}}
"#;
        let mut stdout = StdoutLineReader::new(tokio::io::BufReader::new(&input[..]));
        let mut runtime_state = runtime_state();
        runtime_state.thread_id = Some("thread-1".to_string());
        runtime_state.turn_id = None;
        runtime_state.active_turn_start_request_id = Some(7);
        assert!(prepare_interrupt_request(&mut runtime_state, 100).is_none());
        let state = Arc::new(Mutex::new(runtime_state));
        let sink = RecordingJsonWriteSink::default();
        let writes = Arc::clone(&sink.writes);
        let (events_tx, _events_rx) = mpsc::unbounded_channel();

        read_loop(
            &mut stdout,
            ReservedInterruptSender {
                handle: sink,
                next_id: Arc::new(AtomicU64::new(101)),
            },
            Arc::clone(&state),
            events_tx,
            Arc::new(AtomicBool::new(false)),
            None,
        )
        .await;

        let writes = writes.lock().await;
        let interrupts = writes
            .iter()
            .filter(|value| value["method"] == METHOD_TURN_INTERRUPT)
            .collect::<Vec<_>>();
        assert_eq!(interrupts.len(), 1);
        assert_eq!(interrupts[0]["params"]["threadId"], "thread-1");
        assert_eq!(interrupts[0]["params"]["turnId"], "turn-1");
        assert_eq!(state.lock().await.interrupt_requested_for, None);
    }

    #[test]
    fn interrupt_with_turn_id_builds_a_request_without_reservation() {
        let mut state = runtime_state();
        state.thread_id = Some("thread-1".to_string());
        state.turn_id = Some("turn-1".to_string());
        state.active_turn_start_request_id = Some(7);

        let request = prepare_interrupt_request(&mut state, 102).unwrap();

        assert_eq!(request["method"], METHOD_TURN_INTERRUPT);
        assert_eq!(state.interrupt_requested_for, None);
    }

    #[test]
    fn completed_turn_clears_interrupt_reservation_and_provider_turn_id() {
        let mut state = runtime_state();
        state.thread_id = Some("thread-1".to_string());
        state.turn_id = Some("turn-1".to_string());
        state.active_turn_start_request_id = Some(7);
        state.interrupt_requested_for = Some(7);
        state.pending_methods.insert(7, "permission".to_string());

        reset_completed_turn_state(&mut state);

        assert!(state.turn_id.is_none());
        assert_eq!(state.active_turn_start_request_id, None);
        assert_eq!(state.interrupt_requested_for, None);
        assert!(state.pending_methods.is_empty());
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    // env 変数の直列化のため await 越しにロックを保持する必要がある（テスト用グローバルロック）
    #[allow(clippy::await_holding_lock)]
    async fn test_open_once_initialize_write失敗時にpid登録を削除する() {
        let _env_lock = crate::test_support::TEST_ENV_LOCK.lock();
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
