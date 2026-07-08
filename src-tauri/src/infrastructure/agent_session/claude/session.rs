use std::collections::{HashMap, HashSet};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

use futures_util::stream::{self, Stream};
use serde_json::Value;
use tokio::sync::{mpsc, Mutex};

use crate::domain::agent_session::entities::{
    AttachmentPayload, InterruptReason, MessagePart, PermissionResponse, TurnResult,
};
use crate::domain::agent_session::gateway::{
    AgentBackendError, AgentRuntimeEvent, AgentSessionRuntime, SessionSpec, TurnInput,
};
use crate::domain::agent_session::value_objects::{ModelId, PermissionMode};

use super::convert::{convert_claude_message, ClaudeConvertState};
use super::permission::claude_permission_response;
use super::process::{ClaudeStdioHandle, ClaudeStdioProcess, ClaudeStdoutItem};
use super::wire::{
    claude_wire_mode, control_request_subtype, initialize_request, interrupt_request, message_type,
    set_model_request, set_permission_mode_request, user_message, ClaudeWireMode,
    SUBTYPE_CAN_USE_TOOL, TYPE_CONTROL_REQUEST,
};

const AGENT_PROCESS_EXITED_UNEXPECTEDLY: &str = "Agent process exited unexpectedly";

#[cfg(not(test))]
const ABORT_SYNTHESIS_DELAY: std::time::Duration = std::time::Duration::from_secs(10);
#[cfg(test)]
const ABORT_SYNTHESIS_DELAY: std::time::Duration = std::time::Duration::from_millis(20);

pub(crate) struct ClaudeSessionRuntime {
    cli_path: String,
    state: Arc<Mutex<ClaudeRuntimeState>>,
    process: Mutex<ClaudeRuntimeProcess>,
    events: StdMutex<Option<mpsc::UnboundedReceiver<AgentRuntimeEvent>>>,
    events_tx: mpsc::UnboundedSender<AgentRuntimeEvent>,
    next_id: AtomicU64,
}

struct ClaudeRuntimeProcess {
    handle: ClaudeStdioHandle,
    closed: Arc<AtomicBool>,
}

#[derive(Debug)]
struct ClaudeRuntimeState {
    session_id: String,
    backend_session_id: Option<String>,
    cwd: String,
    model: ModelId,
    permission_mode: PermissionMode,
    plan_mode: bool,
    system_prompt: Option<String>,
    resume: Option<String>,
    base_branch: Option<String>,
    startup_timeout: Option<std::time::Duration>,
    startup_max_retries: Option<u32>,
    stale_timeout: Option<std::time::Duration>,
    turn_active: bool,
    aborting: bool,
    abort_generation: u64,
    synthetic_abort_pending: bool,
    last_successful_backend_session_id: Option<String>,
    pending_inputs: HashMap<String, Value>,
    turn_generation: u64,
    synthetic_abort_turn_generation: Option<u64>,
    discarded_synthetic_abort_generations: HashSet<u64>,
    oversize_dropped_count: u64,
}

impl ClaudeSessionRuntime {
    pub(crate) async fn open(
        cli_path: String,
        spec: SessionSpec,
    ) -> Result<Self, AgentBackendError> {
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let state = Arc::new(Mutex::new(ClaudeRuntimeState::from_spec(&spec)));
        let process = spawn_runtime_process(
            cli_path.clone(),
            &spec,
            Arc::clone(&state),
            events_tx.clone(),
        )
        .await?;

        Ok(Self {
            cli_path,
            state,
            process: Mutex::new(process),
            events: StdMutex::new(Some(events_rx)),
            events_tx,
            next_id: AtomicU64::new(1),
        })
    }

    fn next_request_id(&self) -> String {
        format!("releash-{}", self.next_id.fetch_add(1, Ordering::Relaxed))
    }

    async fn current_handle(&self) -> ClaudeStdioHandle {
        self.process.lock().await.handle.clone()
    }

    async fn replace_process(&self, spec: SessionSpec) -> Result<(), AgentBackendError> {
        let replacement = spawn_runtime_process(
            self.cli_path.clone(),
            &spec,
            Arc::clone(&self.state),
            self.events_tx.clone(),
        )
        .await?;
        let mut process = self.process.lock().await;
        process.closed.store(true, Ordering::Relaxed);
        process.handle.shutdown().await;
        *process = replacement;
        Ok(())
    }
}

impl ClaudeRuntimeState {
    fn from_spec(spec: &SessionSpec) -> Self {
        Self {
            session_id: spec.session_id.clone(),
            backend_session_id: spec.resume.clone(),
            cwd: spec.cwd.clone(),
            model: spec.model.clone(),
            permission_mode: spec.permission_mode,
            plan_mode: spec.plan_mode,
            system_prompt: normalize_system_prompt(spec.system_prompt.clone()),
            resume: spec.resume.clone(),
            base_branch: spec.base_branch.clone(),
            startup_timeout: spec.startup_timeout,
            startup_max_retries: spec.startup_max_retries,
            stale_timeout: spec.stale_timeout,
            turn_active: false,
            aborting: false,
            abort_generation: 0,
            synthetic_abort_pending: false,
            last_successful_backend_session_id: spec.resume.clone(),
            pending_inputs: HashMap::new(),
            turn_generation: 0,
            synthetic_abort_turn_generation: None,
            discarded_synthetic_abort_generations: HashSet::new(),
            oversize_dropped_count: 0,
        }
    }

    fn session_spec_with_system_prompt(&self, system_prompt: Option<String>) -> SessionSpec {
        SessionSpec {
            session_id: self.session_id.clone(),
            cwd: self.cwd.clone(),
            permission_mode: self.permission_mode,
            plan_mode: self.plan_mode,
            permission_profile_id: None,
            model: self.model.clone(),
            system_prompt,
            resume: self
                .backend_session_id
                .clone()
                .or_else(|| self.resume.clone()),
            base_branch: self.base_branch.clone(),
            startup_timeout: self.startup_timeout,
            startup_max_retries: self.startup_max_retries,
            stale_timeout: self.stale_timeout,
        }
    }
}

#[async_trait::async_trait]
impl AgentSessionRuntime for ClaudeSessionRuntime {
    fn take_events(&mut self) -> Pin<Box<dyn Stream<Item = AgentRuntimeEvent> + Send>> {
        let Some(mut receiver) = self.events.lock().ok().and_then(|mut events| events.take())
        else {
            return Box::pin(stream::empty());
        };
        Box::pin(stream::poll_fn(move |cx| receiver.poll_recv(cx)))
    }

    async fn start_turn(&self, input: TurnInput) -> Result<(), AgentBackendError> {
        let system_prompt = normalize_system_prompt(input.system_prompt.clone());
        let (replace_spec, mode_update) = {
            let mut state = self.state.lock().await;
            prepare_start_turn_state(&mut state, &input, system_prompt.clone())
        };

        if let Some(spec) = replace_spec {
            self.replace_process(spec).await?;
            let mut state = self.state.lock().await;
            state.system_prompt = system_prompt.clone();
            state.permission_mode = input.permission_mode;
            state.plan_mode = input.plan_mode;
            state.discarded_synthetic_abort_generations.clear();
        }

        let handle = self.current_handle().await;
        if let Some(mode) = mode_update {
            handle
                .write_json(&set_permission_mode_request(self.next_request_id(), mode))
                .await
                .map_err(AgentBackendError::Other)?;
            let mut state = self.state.lock().await;
            state.permission_mode = input.permission_mode;
            state.plan_mode = input.plan_mode;
        }
        handle
            .write_json(&user_message(&input.prompt, claude_images(input.images)))
            .await
            .map_err(AgentBackendError::Other)
    }

    async fn interrupt(&self) -> Result<(), AgentBackendError> {
        let abort_generation = {
            let mut state = self.state.lock().await;
            state.aborting = true;
            state.abort_generation = state.turn_generation;
            state.abort_generation
        };
        spawn_abort_synthesis_timer(
            Arc::clone(&self.state),
            self.events_tx.clone(),
            abort_generation,
        );
        self.current_handle()
            .await
            .write_json(&interrupt_request(self.next_request_id()))
            .await
            .map_err(AgentBackendError::Other)
    }

    async fn respond_permission(
        &self,
        response: PermissionResponse,
    ) -> Result<(), AgentBackendError> {
        let original_input = {
            let mut state = self.state.lock().await;
            state.pending_inputs.remove(&response.request_id)
        };
        let value = claude_permission_response(response, original_input)
            .map_err(AgentBackendError::Invalid)?;
        self.current_handle()
            .await
            .write_json(&value)
            .await
            .map_err(AgentBackendError::Other)
    }

    async fn set_permission_mode(
        &self,
        mode: PermissionMode,
        plan_mode: bool,
    ) -> Result<(), AgentBackendError> {
        let wire_mode = claude_wire_mode(mode, plan_mode);
        self.current_handle()
            .await
            .write_json(&set_permission_mode_request(
                self.next_request_id(),
                wire_mode,
            ))
            .await
            .map_err(AgentBackendError::Other)?;
        let mut state = self.state.lock().await;
        state.permission_mode = mode;
        state.plan_mode = plan_mode;
        Ok(())
    }

    async fn set_model(&self, model: &ModelId) -> Result<(), AgentBackendError> {
        self.state.lock().await.model = model.clone();
        self.current_handle()
            .await
            .write_json(&set_model_request(
                self.next_request_id(),
                Some(model.as_str()),
            ))
            .await
            .map_err(AgentBackendError::Other)
    }

    async fn set_session_title(&self, _title: &str) -> Result<(), AgentBackendError> {
        Ok(())
    }

    async fn close(&self) {
        let process = self.process.lock().await;
        process.closed.store(true, Ordering::Relaxed);
        process.handle.shutdown().await;
    }
}

async fn spawn_runtime_process(
    cli_path: String,
    spec: &SessionSpec,
    state: Arc<Mutex<ClaudeRuntimeState>>,
    events_tx: mpsc::UnboundedSender<AgentRuntimeEvent>,
) -> Result<ClaudeRuntimeProcess, AgentBackendError> {
    let mut process = ClaudeStdioProcess::spawn(cli_path, spec)
        .await
        .map_err(AgentBackendError::Other)?;
    let handle = process.handle();
    let closed = Arc::new(AtomicBool::new(false));
    let read_closed = Arc::clone(&closed);
    let requested_resume_id = spec.resume.clone();
    let initial_wire_mode = claude_wire_mode(spec.permission_mode, spec.plan_mode);
    let read_state = Arc::clone(&state);
    let read_handle = handle.clone();
    tokio::spawn(async move {
        read_loop(
            &mut process,
            read_handle,
            read_state,
            events_tx,
            read_closed,
            requested_resume_id,
            initial_wire_mode,
        )
        .await;
    });

    if let Err(error) = handle
        .write_json(&initialize_request("releash-initialize"))
        .await
    {
        closed.store(true, Ordering::Relaxed);
        handle.shutdown().await;
        return Err(AgentBackendError::Other(error));
    }
    Ok(ClaudeRuntimeProcess { handle, closed })
}

async fn read_loop(
    process: &mut ClaudeStdioProcess,
    handle: ClaudeStdioHandle,
    state: Arc<Mutex<ClaudeRuntimeState>>,
    events_tx: mpsc::UnboundedSender<AgentRuntimeEvent>,
    closed: Arc<AtomicBool>,
    requested_resume_id: Option<String>,
    initial_wire_mode: super::wire::ClaudeWireMode,
) {
    let mut convert_state = ClaudeConvertState::new(requested_resume_id, initial_wire_mode);
    loop {
        match process.next_json().await {
            Ok(Some(ClaudeStdoutItem::OversizeDropped { bytes })) => {
                if closed.load(Ordering::Relaxed) {
                    break;
                }
                let event = {
                    let mut state = state.lock().await;
                    record_oversize_drop(&mut state, bytes)
                };
                let _ = events_tx.send(event);
            }
            Ok(Some(ClaudeStdoutItem::Json(message))) => {
                if closed.load(Ordering::Relaxed) {
                    break;
                }
                {
                    let state = state.lock().await;
                    convert_state.wire_mode =
                        claude_wire_mode(state.permission_mode, state.plan_mode);
                }
                let conversion = convert_claude_message(&message, &mut convert_state);
                if conversion_requires_permission_input(&conversion.events) {
                    remember_permission_input(&message, &state).await;
                }
                for response in conversion.auto_responses {
                    let _ = handle.write_json(&response).await;
                }
                for event in conversion.events {
                    let event = {
                        let mut state = state.lock().await;
                        normalize_runtime_event(&mut state, event)
                    };
                    if let Some(event) = event {
                        let _ = events_tx.send(event);
                    }
                }
            }
            Ok(None) => {
                log::warn!("Claude CLI exited unexpectedly");
                emit_crash_if_unexpected(
                    &state,
                    &events_tx,
                    &closed,
                    AGENT_PROCESS_EXITED_UNEXPECTEDLY,
                )
                .await;
                break;
            }
            Err(error) => {
                emit_crash_if_unexpected(&state, &events_tx, &closed, &error).await;
                break;
            }
        }
    }
}

fn record_oversize_drop(state: &mut ClaudeRuntimeState, bytes: usize) -> AgentRuntimeEvent {
    state.oversize_dropped_count = state.oversize_dropped_count.saturating_add(1);
    log::warn!("claude stdout dropped an oversized line: {bytes} bytes");
    AgentRuntimeEvent::PartsMerged(vec![MessagePart::Error {
        content: "backend からの応答 1 件がサイズ上限（8MB）を超えたため破棄しました".to_string(),
        parent_tool_use_id: None,
    }])
}

async fn remember_permission_input(message: &Value, state: &Arc<Mutex<ClaudeRuntimeState>>) {
    if message_type(message) != Some(TYPE_CONTROL_REQUEST)
        || control_request_subtype(message) != Some(SUBTYPE_CAN_USE_TOOL)
    {
        return;
    }
    let Some(request_id) = message.get("request_id").and_then(Value::as_str) else {
        return;
    };
    let input = message
        .get("request")
        .and_then(|request| request.get("input"))
        .cloned()
        .unwrap_or(Value::Null);
    state
        .lock()
        .await
        .pending_inputs
        .insert(request_id.to_string(), input);
}

fn conversion_requires_permission_input(events: &[AgentRuntimeEvent]) -> bool {
    events
        .iter()
        .any(|event| matches!(event, AgentRuntimeEvent::PermissionRequested(_)))
}

fn normalize_runtime_event(
    state: &mut ClaudeRuntimeState,
    event: AgentRuntimeEvent,
) -> Option<AgentRuntimeEvent> {
    match &event {
        AgentRuntimeEvent::SessionEstablished {
            backend_session_id, ..
        } => {
            state.backend_session_id = Some(backend_session_id.clone());
            state.last_successful_backend_session_id = state
                .last_successful_backend_session_id
                .clone()
                .or_else(|| Some(backend_session_id.clone()));
            Some(event)
        }
        AgentRuntimeEvent::TurnCompleted(_) => {
            if state.synthetic_abort_pending
                && state.synthetic_abort_turn_generation == Some(state.turn_generation)
            {
                log::debug!(
                    "discarding late turn result for generation {} already terminated by synthetic abort",
                    state.turn_generation
                );
                state.synthetic_abort_pending = false;
                state.synthetic_abort_turn_generation = None;
                state
                    .discarded_synthetic_abort_generations
                    .remove(&state.turn_generation);
                state.pending_inputs.clear();
                return None;
            }
            if state.aborting {
                state.aborting = false;
                state.synthetic_abort_pending = false;
                state.synthetic_abort_turn_generation = None;
                state.turn_active = false;
                state.backend_session_id = state.last_successful_backend_session_id.clone();
                state.pending_inputs.clear();
                return Some(AgentRuntimeEvent::TurnCompleted(TurnResult::Interrupted {
                    reason: InterruptReason::Abort,
                    error: None,
                }));
            }
            if matches!(
                event,
                AgentRuntimeEvent::TurnCompleted(TurnResult::Completed { .. })
            ) {
                state.last_successful_backend_session_id = state.backend_session_id.clone();
            }
            state.turn_active = false;
            state.pending_inputs.clear();
            Some(event)
        }
        _ => Some(event),
    }
}

fn spawn_abort_synthesis_timer(
    state: Arc<Mutex<ClaudeRuntimeState>>,
    events_tx: mpsc::UnboundedSender<AgentRuntimeEvent>,
    abort_generation: u64,
) {
    tokio::spawn(async move {
        tokio::time::sleep(ABORT_SYNTHESIS_DELAY).await;
        let should_emit = {
            let mut state = state.lock().await;
            if state.abort_generation != abort_generation {
                return;
            }
            if !state.aborting {
                return;
            }
            if !state.turn_active {
                state.aborting = false;
                state.synthetic_abort_pending = false;
                state.synthetic_abort_turn_generation = None;
                return;
            }
            state.aborting = false;
            state.turn_active = false;
            state.synthetic_abort_pending = true;
            state.synthetic_abort_turn_generation = Some(abort_generation);
            state
                .discarded_synthetic_abort_generations
                .insert(abort_generation);
            state.backend_session_id = state.last_successful_backend_session_id.clone();
            state.pending_inputs.clear();
            true
        };
        if should_emit {
            let _ = events_tx.send(AgentRuntimeEvent::TurnCompleted(TurnResult::Interrupted {
                reason: InterruptReason::Abort,
                error: None,
            }));
        }
    });
}

async fn emit_crash_if_unexpected(
    state: &Arc<Mutex<ClaudeRuntimeState>>,
    events_tx: &mpsc::UnboundedSender<AgentRuntimeEvent>,
    closed: &AtomicBool,
    message: &str,
) {
    if closed.load(Ordering::Relaxed) {
        return;
    }
    let (was_turn_active, aborting, oversize_dropped_count) = {
        let mut state = state.lock().await;
        let was_turn_active = state.turn_active;
        let aborting = state.aborting;
        state.turn_active = false;
        state.aborting = false;
        state.pending_inputs.clear();
        (was_turn_active, aborting, state.oversize_dropped_count)
    };
    let message = if oversize_dropped_count > 0 {
        format!("{message}（サイズ超過破棄 {oversize_dropped_count} 件）")
    } else {
        message.to_string()
    };
    if was_turn_active {
        let _ = events_tx.send(AgentRuntimeEvent::TurnCompleted(TurnResult::Interrupted {
            reason: if aborting {
                InterruptReason::Abort
            } else {
                InterruptReason::Crash
            },
            error: (!aborting).then(|| message.clone()),
        }));
    }
    let _ = events_tx.send(AgentRuntimeEvent::Fatal { message });
}

fn prepare_start_turn_state(
    state: &mut ClaudeRuntimeState,
    input: &TurnInput,
    system_prompt: Option<String>,
) -> (Option<SessionSpec>, Option<ClaudeWireMode>) {
    let previous_wire_mode = claude_wire_mode(state.permission_mode, state.plan_mode);
    let restart_after_synthetic_abort = !state.discarded_synthetic_abort_generations.is_empty();
    state.aborting = false;
    state.synthetic_abort_pending = false;
    state.synthetic_abort_turn_generation = None;
    state.turn_generation = state.turn_generation.saturating_add(1);
    state.turn_active = true;
    state.oversize_dropped_count = 0;
    let next_wire_mode = claude_wire_mode(input.permission_mode, input.plan_mode);
    let replace_spec = if restart_after_synthetic_abort || state.system_prompt != system_prompt {
        let mut spec = state.session_spec_with_system_prompt(system_prompt);
        spec.permission_mode = input.permission_mode;
        spec.plan_mode = input.plan_mode;
        Some(spec)
    } else {
        None
    };
    let mode_update =
        (replace_spec.is_none() && previous_wire_mode != next_wire_mode).then_some(next_wire_mode);
    (replace_spec, mode_update)
}

fn claude_images(images: Vec<AttachmentPayload>) -> Vec<(String, String)> {
    images
        .into_iter()
        .map(|image| (image.media_type, image.data))
        .collect()
}

fn normalize_system_prompt(system_prompt: Option<String>) -> Option<String> {
    system_prompt.filter(|value| !value.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agent_session::entities::{
        PermissionRequestStatus, TokenUsage, TurnStopReason,
    };
    use crate::domain::agent_session::value_objects::JsonPayload;

    #[cfg(unix)]
    fn write_fake_claude_cli(dir: &std::path::Path) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let path = dir.join("fake-claude");
        std::fs::write(
            &path,
            r#"#!/bin/sh
if [ "${1:-}" = "--version" ]; then
  echo "Claude Code 2.0.0"
  exit 0
fi
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

    fn test_spec() -> SessionSpec {
        SessionSpec {
            session_id: "session-1".to_string(),
            cwd: ".".to_string(),
            permission_mode: PermissionMode::Edit,
            plan_mode: false,
            permission_profile_id: None,
            model: ModelId::parse("claude-4-sonnet").unwrap(),
            system_prompt: None,
            resume: Some("backend-good".to_string()),
            base_branch: None,
            startup_timeout: None,
            startup_max_retries: None,
            stale_timeout: None,
        }
    }

    fn test_state() -> ClaudeRuntimeState {
        ClaudeRuntimeState::from_spec(&test_spec())
    }

    #[test]
    fn test_normalize_system_prompt_treats_empty_as_none() {
        assert_eq!(normalize_system_prompt(Some("  ".to_string())), None);
        assert_eq!(
            normalize_system_prompt(Some("system".to_string())),
            Some("system".to_string())
        );
    }

    #[test]
    fn test_claude_images_preserve_media_type_and_data() {
        let images = claude_images(vec![AttachmentPayload {
            data: "abc".to_string(),
            media_type: "image/png".to_string(),
        }]);

        assert_eq!(images, vec![("image/png".to_string(), "abc".to_string())]);
    }

    #[test]
    fn test_conversion_requires_permission_inputは_prompt変換分だけ保存対象にする() {
        let prompt_events = vec![AgentRuntimeEvent::PermissionRequested(
            crate::domain::agent_session::entities::PermissionRequest {
                id: "req-1".to_string(),
                tool_use_id: None,
                parent_tool_use_id: None,
                tool_name: "Bash".to_string(),
                body: crate::domain::agent_session::entities::PermissionRequestBody::ToolApproval {
                    input: JsonPayload::new_unchecked(r#"{"command":"echo hi"}"#.to_string()),
                },
                title: None,
                display_name: None,
                description: None,
                decision_reason: None,
                status: PermissionRequestStatus::Pending,
            },
        )];

        assert!(conversion_requires_permission_input(&prompt_events));
        assert!(!conversion_requires_permission_input(&[]));
        assert!(!conversion_requires_permission_input(&[
            AgentRuntimeEvent::PartsMerged(Vec::new())
        ]));
    }

    #[test]
    fn test_normalize_runtime_event_aborting中の_completedも_abortへ変換してrollbackする() {
        let mut state = test_state();
        state.backend_session_id = Some("backend-new".to_string());
        state.last_successful_backend_session_id = Some("backend-good".to_string());
        state.turn_active = true;
        state.aborting = true;

        let event = normalize_runtime_event(
            &mut state,
            AgentRuntimeEvent::TurnCompleted(TurnResult::Completed {
                stop_reason: Some(TurnStopReason::Refusal),
                token_usage: Some(TokenUsage {
                    input_tokens: 1,
                    output_tokens: 2,
                    total_tokens: Some(3),
                    context_window_tokens: None,
                }),
            }),
        );

        assert_eq!(
            event,
            Some(AgentRuntimeEvent::TurnCompleted(TurnResult::Interrupted {
                reason: InterruptReason::Abort,
                error: None,
            }))
        );
        assert!(!state.aborting);
        assert!(!state.turn_active);
        assert_eq!(state.backend_session_id.as_deref(), Some("backend-good"));
    }

    #[test]
    fn test_normalize_runtime_event_aborting中の_failedも_abortへ変換する() {
        let mut state = test_state();
        state.turn_active = true;
        state.aborting = true;

        let event = normalize_runtime_event(
            &mut state,
            AgentRuntimeEvent::TurnCompleted(TurnResult::Failed {
                error: "boom".to_string(),
                token_usage: None,
            }),
        );

        assert!(matches!(
            event,
            Some(AgentRuntimeEvent::TurnCompleted(TurnResult::Interrupted {
                reason: InterruptReason::Abort,
                ..
            }))
        ));
        assert!(!state.aborting);
        assert!(!state.turn_active);
    }

    #[test]
    fn test_normalize_runtime_event_synthetic_abort後の同一turn結果は破棄する() {
        let mut state = test_state();
        state.turn_generation = 7;
        state.synthetic_abort_pending = true;
        state.synthetic_abort_turn_generation = Some(7);

        let event = normalize_runtime_event(
            &mut state,
            AgentRuntimeEvent::TurnCompleted(TurnResult::Failed {
                error: "late".to_string(),
                token_usage: None,
            }),
        );

        assert_eq!(event, None);
        assert!(!state.synthetic_abort_pending);
        assert_eq!(state.synthetic_abort_turn_generation, None);
    }

    #[test]
    fn test_prepare_start_turn_stateは_abortフラグを次turnへ持ち越さない() {
        let mut state = test_state();
        state.aborting = true;
        state.synthetic_abort_pending = true;
        state.synthetic_abort_turn_generation = Some(1);
        state.turn_generation = 1;

        let input = TurnInput {
            prompt: "next".to_string(),
            images: Vec::new(),
            system_prompt: state.system_prompt.clone(),
            permission_mode: state.permission_mode,
            plan_mode: state.plan_mode,
            permission_profile_id: None,
            editor_context: None,
        };
        let (replace_spec, mode_update) =
            prepare_start_turn_state(&mut state, &input, input.system_prompt.clone());

        assert!(!state.aborting);
        assert!(!state.synthetic_abort_pending);
        assert_eq!(state.synthetic_abort_turn_generation, None);
        assert_eq!(state.turn_generation, 2);
        assert!(state.turn_active);
        assert!(replace_spec.is_none());
        assert!(mode_update.is_none());
    }

    #[test]
    fn test_record_oversize_dropは_error_partを合成しカウントを加算する() {
        let mut state = test_state();

        let event = record_oversize_drop(&mut state, 9 * 1024 * 1024);
        let _ = record_oversize_drop(&mut state, 10 * 1024 * 1024);

        assert_eq!(state.oversize_dropped_count, 2);
        assert_eq!(
            event,
            AgentRuntimeEvent::PartsMerged(vec![MessagePart::Error {
                content: "backend からの応答 1 件がサイズ上限（8MB）を超えたため破棄しました"
                    .to_string(),
                parent_tool_use_id: None,
            }])
        );
    }

    #[test]
    fn test_prepare_start_turn_stateは_サイズ超過破棄カウントをリセットする() {
        let mut state = test_state();
        state.oversize_dropped_count = 3;

        let input = TurnInput {
            prompt: "next".to_string(),
            images: Vec::new(),
            system_prompt: state.system_prompt.clone(),
            permission_mode: state.permission_mode,
            plan_mode: state.plan_mode,
            permission_profile_id: None,
            editor_context: None,
        };
        prepare_start_turn_state(&mut state, &input, input.system_prompt.clone());

        assert_eq!(state.oversize_dropped_count, 0);
    }

    #[tokio::test]
    async fn test_emit_crash_if_unexpectedは_サイズ超過破棄件数を終端メッセージに含める() {
        let state = Arc::new(Mutex::new(test_state()));
        {
            let mut state = state.lock().await;
            state.turn_active = true;
            state.oversize_dropped_count = 2;
        }
        let (tx, mut rx) = mpsc::unbounded_channel();
        let closed = AtomicBool::new(false);

        emit_crash_if_unexpected(&state, &tx, &closed, "boom").await;

        assert_eq!(
            rx.try_recv().unwrap(),
            AgentRuntimeEvent::TurnCompleted(TurnResult::Interrupted {
                reason: InterruptReason::Crash,
                error: Some("boom（サイズ超過破棄 2 件）".to_string()),
            })
        );
        assert_eq!(
            rx.try_recv().unwrap(),
            AgentRuntimeEvent::Fatal {
                message: "boom（サイズ超過破棄 2 件）".to_string(),
            }
        );
    }

    #[tokio::test]
    async fn test_emit_crash_if_unexpectedは_超過破棄なしならメッセージを変えない() {
        let state = Arc::new(Mutex::new(test_state()));
        {
            let mut state = state.lock().await;
            state.turn_active = true;
        }
        let (tx, mut rx) = mpsc::unbounded_channel();
        let closed = AtomicBool::new(false);

        emit_crash_if_unexpected(&state, &tx, &closed, "boom").await;

        assert_eq!(
            rx.try_recv().unwrap(),
            AgentRuntimeEvent::TurnCompleted(TurnResult::Interrupted {
                reason: InterruptReason::Crash,
                error: Some("boom".to_string()),
            })
        );
        assert_eq!(
            rx.try_recv().unwrap(),
            AgentRuntimeEvent::Fatal {
                message: "boom".to_string(),
            }
        );
    }

    #[tokio::test]
    async fn test_abort_synthesis_timer_idle_early_returnで_abortフラグを復帰する() {
        let state = Arc::new(Mutex::new(test_state()));
        {
            let mut state = state.lock().await;
            state.aborting = true;
            state.turn_active = false;
            state.abort_generation = 3;
            state.turn_generation = 3;
        }
        let (tx, mut rx) = mpsc::unbounded_channel();

        spawn_abort_synthesis_timer(Arc::clone(&state), tx, 3);
        tokio::time::sleep(ABORT_SYNTHESIS_DELAY + std::time::Duration::from_millis(10)).await;

        assert!(rx.try_recv().is_err());
        let state = state.lock().await;
        assert!(!state.aborting);
        assert!(!state.synthetic_abort_pending);
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    // env 変数の直列化のため await 越しにロックを保持する必要がある（テスト用グローバルロック）
    #[allow(clippy::await_holding_lock)]
    async fn test_spawn_runtime_process_initialize_write失敗時にpid登録を削除する() {
        let _env_lock = crate::test_support::TEST_ENV_LOCK.lock().unwrap();
        let data_dir = tempfile::tempdir().unwrap();
        let _env_guard =
            crate::test_support::EnvVarGuard::set_path("RELEASH_DATA_DIR", data_dir.path());
        let _fail_write_guard = crate::test_support::EnvVarGuard::set_value(
            "RELEASH_TEST_FAIL_CLAUDE_STDIN_WRITE",
            "1",
        );
        let cli_path = write_fake_claude_cli(data_dir.path());
        let mut spec = test_spec();
        spec.session_id = "claude-startup-cleanup".to_string();
        spec.cwd = data_dir.path().to_string_lossy().to_string();
        spec.resume = None;
        let state = Arc::new(Mutex::new(ClaudeRuntimeState::from_spec(&spec)));
        let (events_tx, _events_rx) = mpsc::unbounded_channel();

        let result = spawn_runtime_process(
            cli_path.to_string_lossy().to_string(),
            &spec,
            state,
            events_tx,
        )
        .await;

        assert!(matches!(
            result,
            Err(AgentBackendError::Other(message))
                if message.contains("failed to write claude stdin")
                    || message.contains("claude stdin is closed")
                    || message.contains("injected claude stdin write failure")
        ));
        assert_agent_process_registry_empty(data_dir.path());
    }
}
