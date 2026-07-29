#[cfg(test)]
fn queued_agent_message(
    session_store: &Arc<SessionStore>,
    data_dir: &Path,
    session_id: &str,
    queued: &QueuedTurnInput,
) -> Result<ChatMessage, String> {
    if let Some(message_id) = queued.existing_agent_message_id.as_deref() {
        if let Some(message) = session_store
            .load_full_session_for_restore(data_dir, session_id)?
            .and_then(|session| {
                session
                    .messages
                    .into_iter()
                    .find(|message| message.id == message_id)
            })
        {
            return Ok(message);
        }
    }
    add_message_internal(
        session_store,
        data_dir,
        session_id,
        MessageRole::Agent,
        "",
        None,
        None,
    )
}

fn committed_queued_message(
    session_store: &Arc<SessionStore>,
    data_dir: &Path,
    session_id: &str,
    message_id: &str,
    expected_role: MessageRole,
) -> Result<Option<ChatMessage>, String> {
    if let Some(message) = session_store.canonical_message_projection(session_id, message_id)? {
        return (message.role == expected_role)
            .then_some(message)
            .ok_or_else(|| "committed queued message role is incompatible".to_string())
            .map(Some);
    }
    let message = session_store
        .load_full_session_for_restore(data_dir, session_id)?
        .and_then(|session| {
            session
                .messages
                .into_iter()
                .find(|message| message.id == message_id)
        });
    match message {
        Some(message) if message.role == expected_role => Ok(Some(message)),
        Some(_) => Err("committed queued message role is incompatible".to_string()),
        None => Ok(None),
    }
}

fn build_queued_system_prompt(
    session_store: &Arc<SessionStore>,
    branch_diff_context: Option<&dyn BranchDiffContextPort>,
    instruction_source: &dyn InstructionSourcePort,
    data_dir: &Path,
    session: &ChatSession,
    queued: &QueuedTurnInput,
) -> Result<Option<String>, String> {
    let backend_id = session
        .backend_id
        .as_deref()
        .ok_or_else(|| format!("Session {} is missing backend id", session.id))?;
    let built = build_session_system_prompt(SessionSystemPromptBuildRequest {
        session_store,
        data_dir,
        session,
        branch_diff_context,
        instruction_source,
        backend_id,
        model_id: session.selected_model.as_deref(),
        mentions: &queued.mentions,
        editor_context: queued
            .editor_context
            .as_ref()
            .and_then(system_context_editor_input),
        workflow_instructions: Vec::new(),
    })?;
    let prompt = compose_system_prompt(None, &built.system_context);
    persist_session_system_prompt_build(session_store, data_dir, &session.id, &built)?;
    Ok(prompt)
}

fn permission_request_from_parts(
    projection_gateway: &dyn AgentRuntimeProjectionGateway,
    parts: &[MessagePart],
    request_id: &str,
) -> Option<PermissionRequestMsg> {
    parts.iter().rev().find_map(|part| match part {
        MessagePart::Permission { request, .. } if request.id == request_id => {
            Some(projection_gateway.permission_request(request))
        }
        _ => None,
    })
}

fn pending_queue_view(state: &RuntimeSessionState) -> Vec<QueuedAgentTurn> {
    let mut effects = state
        .accepted_input_effects
        .values()
        .collect::<Vec<_>>();
    effects.sort_by(|left, right| {
        left.created_at
            .total_cmp(&right.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    effects.into_iter().map(QueuedAgentTurn::from).collect()
}

#[cfg(test)]
fn add_human_message_internal(
    session_store: &SessionStore,
    data_dir: &Path,
    session_id: &str,
    content: &str,
    images: &[ImageAttachment],
    mentions: &[crate::domain::code::MentionReference],
) -> Result<(ChatMessage, SessionMeta), AgentRuntimeError> {
    let parts = human_parts(content, images);
    add_message_with_meta_internal(
        session_store,
        data_dir,
        session_id,
        MessageRole::Human,
        content,
        (!parts.is_empty()).then_some(parts),
        (!mentions.is_empty()).then_some(mentions.to_vec()),
    )
    .map_err(AgentRuntimeError::Other)
}

fn human_parts(content: &str, images: &[ImageAttachment]) -> Vec<MessagePart> {
    if images.is_empty() {
        return Vec::new();
    }
    let mut parts = Vec::new();
    if !content.is_empty() {
        parts.push(MessagePart::Text {
            content: content.to_string(),
            parent_tool_use_id: None,
        });
    }
    parts.extend(images.iter().map(|image| MessagePart::Image {
        data: image.data.clone(),
        media_type: image.media_type.clone(),
    }));
    parts
}

pub(super) fn required_backend_id(session: &ChatSession) -> Result<String, AgentRuntimeError> {
    session.backend_id.clone().ok_or_else(|| {
        AgentRuntimeError::Other(format!("Session {} is missing backend id", session.id))
    })
}

fn system_context_editor_input(context: &AgentEditorContext) -> Option<SystemContextEditorInput> {
    Some(SystemContextEditorInput {
        active_editor_path: context.active_editor_path.clone(),
        open_editor_paths: context.open_editor_paths.clone(),
        selection_file_path: context
            .selection
            .as_ref()
            .map(|selection| selection.file_path.clone()),
        payload: serde_json::to_string(context).ok(),
    })
}

fn compose_system_prompt(
    system_prompt: Option<String>,
    context: &BuiltSystemContext,
) -> Option<String> {
    let context_blocks = context
        .snapshots
        .iter()
        .filter_map(system_context_block)
        .collect::<Vec<_>>();
    let context_prompt = (!context_blocks.is_empty()).then(|| context_blocks.join("\n\n"));
    let system_prompt = system_prompt.filter(|prompt| !prompt.trim().is_empty());

    match (system_prompt, context_prompt) {
        (Some(prompt), Some(context_prompt)) => Some(format!("{prompt}\n\n{context_prompt}")),
        (None, Some(context_prompt)) => Some(context_prompt),
        (Some(prompt), _) => Some(prompt),
        (None, None) => None,
    }
}

fn system_context_block(snapshot: &ContextSnapshot) -> Option<String> {
    let payload = snapshot.payload.trim();
    if payload.is_empty() {
        return None;
    }
    let tag = match snapshot.kind {
        ContextSourceKind::RepoSummary => "releash_repo_summary",
        ContextSourceKind::DiffReviewSnapshot => "releash_diff_review_snapshot",
        ContextSourceKind::OpenEditorSelection => "releash_open_editor_selection",
        ContextSourceKind::Mentions => "releash_mentions",
        ContextSourceKind::TerminalLogSummary => "releash_terminal_log_summary",
        ContextSourceKind::WorkflowContext => "releash_workflow_state",
        ContextSourceKind::ProjectInstructions => "releash_project_instructions",
        ContextSourceKind::BackendModelIdentity => "releash_backend_model_identity",
    };
    Some(format!("<{tag}>\n{payload}\n</{tag}>"))
}

impl From<AgentEditorContext> for EditorContext {
    fn from(value: AgentEditorContext) -> Self {
        Self {
            active_editor_path: value.active_editor_path,
            open_editor_paths: value.open_editor_paths,
            selection: value.selection.map(|selection| {
                crate::domain::agent_session::value_objects::EditorSelection {
                    file_path: selection.file_path,
                    start_line: selection.start_line,
                    end_line: selection.end_line,
                }
            }),
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct StateChange {
    pub(super) turn_phase: TurnPhase,
    pub(super) queue_paused: Option<bool>,
    pub(super) pending_permission_request: Option<PermissionRequestMsg>,
    pub(super) pending_permission_state_revision: Option<u64>,
    pub(super) exit_code: Option<i64>,
    pub(super) completed_at: Option<f64>,
    pub(super) interrupted: bool,
    pub(super) session_state: Option<SessionState>,
}

fn next_turn_id(
    session_store: &Arc<SessionStore>,
    data_dir: &Path,
    session_id: &str,
) -> Result<u64, String> {
    session_store
        .next_turn_id(data_dir, session_id)
        .map_err(|error| error.to_string())
}

struct PendingPermissionForResponse {
    turn_id: Option<u64>,
    #[cfg(test)]
    from_runtime_state: bool,
}

fn durable_part_events(
    session_store: &Arc<SessionStore>,
    data_dir: &Path,
    session_id: &str,
    turn_id: u64,
    message_id: &str,
    domain_parts: &[DomainMessagePart],
    parts: &[MessagePart],
) -> Result<Vec<AgentSessionEvent>, String> {
    if !domain_parts
        .iter()
        .any(crate::domain::agent_session::services::part_records_durable_event)
    {
        return Ok(Vec::new());
    }
    let mut events = if domain_parts
        .iter()
        .any(crate::domain::agent_session::services::part_needs_event_history)
    {
        session_store.load_session_events(data_dir, session_id)?
    } else {
        Vec::new()
    };
    let before = events.len();
    append_part_events(
        &mut events,
        turn_id,
        message_id,
        parts,
        PartEventMode::DurableOnly,
    );
    Ok(events.into_iter().skip(before).collect())
}

fn patch_permission_response_in_state(
    state: &mut RuntimeSessionState,
    response: &PermissionResponse,
) -> Option<(String, u64, Vec<MessagePart>, u64)> {
    if !state.patch_stream_permission_response(response) {
        return None;
    }
    let sequence = state.advance_stream_sequence();
    let message_id = state
        .streaming_message_id
        .clone()
        .or_else(|| state.last_agent_message_id.clone())?;
    let turn_id = state.active_turn_id()?;
    Some((
        message_id,
        sequence,
        state.persisted_streaming_parts().to_vec(),
        turn_id,
    ))
}

#[cfg(test)]
fn permission_answers_from_response(
    response: &PermissionResponse,
) -> Option<crate::domain::agent_session::value_objects::JsonPayload> {
    match &response.decision {
        PermissionResponseDecision::Allow { answers, .. } => answers.clone(),
        PermissionResponseDecision::Deny { .. } => None,
    }
}

#[cfg(test)]
fn permission_resolved_event(turn_id: u64, response: &PermissionResponse) -> AgentSessionEvent {
    let decision = match &response.decision {
        PermissionResponseDecision::Allow { .. } => {
            crate::usecase::agent_session::event_log::PermissionDecision::Allowed
        }
        PermissionResponseDecision::Deny { .. } => {
            crate::usecase::agent_session::event_log::PermissionDecision::Denied
        }
    };
    let answers = permission_answers_from_response(response);
    AgentSessionEvent::PermissionResolved {
        turn_id,
        tool_use_id: None,
        request_id: Some(response.request_id.clone()),
        decision,
        answers,
    }
}

fn resync_permission_mode(
    session_store: &Arc<SessionStore>,
    data_dir: &Path,
    session_id: &str,
    reported_mode: PermissionMode,
) -> Option<PermissionMode> {
    let meta = match session_store.get_session_meta(data_dir, session_id) {
        Ok(meta) => meta,
        Err(error) => {
            log::warn!("failed to load permission mode for {session_id}: {error}");
            return None;
        }
    }?;
    let saved_mode = match PermissionMode::parse(&meta.permission_mode) {
        Ok(mode) => mode,
        Err(error) => {
            log::warn!("stored permission mode is invalid for {session_id}: {error}");
            return None;
        }
    };
    if saved_mode == reported_mode {
        return None;
    }
    Some(saved_mode)
}

fn final_turn_events(
    ctx: &RuntimeContext,
    session_id: &str,
    turn_id: u64,
    message_id: &str,
    parts: &[MessagePart],
    terminal: &TerminalProjection,
    completed_at: f64,
) -> Result<Vec<AgentSessionEvent>, String> {
    let existing_events = ctx
        .session_store
        .load_current_reducer_events(&ctx.data_dir, session_id)?;
    if crate::domain::agent_session::aggregates::session::Session::has_turn_terminal(
        &existing_events,
        turn_id,
    ) {
        return Ok(Vec::new());
    }
    let mut appended = vec![AgentSessionEvent::FinalPartsRecorded {
        turn_id,
        message_id: message_id.to_string(),
        parts: parts.to_vec(),
    }];
    match &terminal.event {
        TerminalEventProjection::Completed {
            stop_reason,
            token_usage,
        } => {
            appended.push(AgentSessionEvent::TurnCompleted {
                turn_id,
                exit_code: terminal.exit_code,
                stop_reason: *stop_reason,
                token_usage: *token_usage,
            });
        }
        TerminalEventProjection::Interrupted { reason, error } => {
            let mut events = existing_events.clone();
            events.extend(appended.iter().cloned());
            let before = events.len();
            finalize_turn(
                &mut events,
                turn_id,
                *reason,
                error.clone(),
                terminal.exit_code,
            );
            appended.extend(events.into_iter().skip(before));
        }
    }
    if terminal.pause_queue {
        appended.push(AgentSessionEvent::QueuePaused { at: completed_at });
    }
    Ok(
        crate::domain::agent_session::aggregates::session::Session::canonicalize_terminal_queue_pause(
            &existing_events,
            appended,
        ),
    )
}

pub(super) fn emit_session_state_change(
    session_store: &Arc<SessionStore>,
    notifier: &Arc<dyn AgentSessionEventNotifier>,
    status_center: &Arc<AgentStatusCenter>,
    status_notifier: &Arc<dyn AgentStatusNotifier>,
    data_dir: &Path,
    session_id: &str,
    change: StateChange,
) {
    notifier.session_state_changed(AgentSessionStateChangedPayload {
        chat_session_id: session_id.to_string(),
        turn_phase: change.turn_phase,
        exit_code: change.exit_code,
        completed_at: change.completed_at,
        interrupted: change.interrupted,
        session_state: change.session_state,
        queue_paused: change.queue_paused,
        pending_permission_request: change.pending_permission_request.clone(),
        pending_permission_state_revision: change.pending_permission_state_revision,
    });
    publish_status_change(
        session_store,
        status_center,
        status_notifier,
        data_dir,
        session_id,
        change,
    );
}

fn emit_session_state_change_from_session(
    session: &ChatSession,
    notifier: &Arc<dyn AgentSessionEventNotifier>,
    status_center: &Arc<AgentStatusCenter>,
    status_notifier: &Arc<dyn AgentStatusNotifier>,
    change: StateChange,
) {
    notifier.session_state_changed(AgentSessionStateChangedPayload {
        chat_session_id: session.id.clone(),
        turn_phase: change.turn_phase,
        exit_code: change.exit_code,
        completed_at: change.completed_at,
        interrupted: change.interrupted,
        session_state: change.session_state,
        queue_paused: change.queue_paused,
        pending_permission_request: change.pending_permission_request.clone(),
        pending_permission_state_revision: change.pending_permission_state_revision,
    });
    publish_status_change_from_session(session, status_center, status_notifier, change);
}

fn publish_status_change(
    session_store: &Arc<SessionStore>,
    status_center: &Arc<AgentStatusCenter>,
    status_notifier: &Arc<dyn AgentStatusNotifier>,
    data_dir: &Path,
    session_id: &str,
    change: StateChange,
) {
    let session = match session_store.get_session_shell(data_dir, session_id) {
        Ok(Some(session)) => session,
        Ok(None) => return,
        Err(error) => {
            log::warn!("failed to load session for status update {session_id}: {error}");
            return;
        }
    };
    publish_status_change_from_session(&session, status_center, status_notifier, change);
}

fn publish_status_change_from_session(
    session: &ChatSession,
    status_center: &Arc<AgentStatusCenter>,
    status_notifier: &Arc<dyn AgentStatusNotifier>,
    change: StateChange,
) {
    let session_state = change
        .session_state
        .unwrap_or(session.state);
    let worktree_path = session.worktree_path.clone();
    let workflow_context = session.workflow_node_context.clone();
    let workflow_execution_status = change
        .turn_phase
        .workflow_execution_is_running()
        .then(|| workflow_context.as_ref().map(|_| "running".to_string()))
        .flatten();
    let status = SessionStatus {
        chat_session_id: session.id.clone(),
        worktree_id: worktree_path.clone(),
        worktree_path,
        pty_id: None,
        agent_state: AgentStatusCenter::derive_agent_state(
            change.turn_phase,
            session_state,
        ),
        turn_phase: TurnPhaseRepr::from(change.turn_phase),
        session_state,
        pending_permission: change.turn_phase.has_pending_permission(),
        pending_permission_request: change.pending_permission_request,
        last_activity_at: crate::usecase::agent_session::session::now_timestamp(),
        workflow_node: workflow_context
            .as_ref()
            .map(|context| context.node_name.clone()),
        workflow_execution_status,
        workflow_execution_id: workflow_context
            .as_ref()
            .map(|context| context.execution_id.clone()),
        node_execution_id: workflow_context
            .as_ref()
            .map(|context| context.node_execution_id.clone()),
        workflow_attempt: workflow_context.as_ref().map(|context| context.attempt),
        notice: None,
        workflow_node_progress: None,
    };
    status_notifier.status_changed(status_center.update_session(status));
}

fn session_telemetry_dimensions(
    session_store: &Arc<SessionStore>,
    data_dir: &Path,
    session_id: &str,
) -> Option<crate::other::telemetry::AgentTurnDimensions> {
    let session = session_store
        .get_session_shell(data_dir, session_id)
        .ok()
        .flatten()?;
    Some(crate::other::telemetry::AgentTurnDimensions {
        resume: session.agent_session_id.is_some(),
        has_session: true,
        permission_mode: crate::other::telemetry::PermissionModeDim::normalize(
            &session.permission_mode,
        ),
        model: crate::other::telemetry::ModelFamily::normalize(session.selected_model.as_deref()),
        context: crate::other::telemetry::TurnContext::from_workflow_node(
            session.is_workflow_node_session(),
        ),
        channel: crate::other::telemetry::Payload::TauriEvent,
        warm_path: crate::other::telemetry::WarmPath::QueryDirect,
    })
}

#[cfg(test)]
struct TestNoopAgentRuntime;

#[cfg(test)]
#[async_trait::async_trait]
impl AgentSessionRuntime for TestNoopAgentRuntime {
    fn take_events(
        &mut self,
    ) -> std::pin::Pin<Box<dyn futures_util::Stream<Item = AgentRuntimeEvent> + Send>> {
        Box::pin(futures_util::stream::empty())
    }

    async fn start_turn(&self, _input: TurnInput) -> Result<(), AgentBackendError> {
        Ok(())
    }

    async fn interrupt(&self) -> Result<(), AgentBackendError> {
        Ok(())
    }

    async fn respond_permission(
        &self,
        _response: PermissionResponse,
    ) -> Result<(), AgentBackendError> {
        Err(AgentBackendError::Other(
            "injected test permission failure".to_string(),
        ))
    }

    async fn set_model(&self, _model: &ModelId) -> Result<(), AgentBackendError> {
        Err(AgentBackendError::Other(
            "injected test model failure".to_string(),
        ))
    }

    async fn close(&self) {}
}

#[cfg(test)]
struct TestFailingAgentRuntime;

#[cfg(test)]
#[async_trait::async_trait]
impl AgentSessionRuntime for TestFailingAgentRuntime {
    fn take_events(
        &mut self,
    ) -> std::pin::Pin<Box<dyn futures_util::Stream<Item = AgentRuntimeEvent> + Send>> {
        Box::pin(futures_util::stream::empty())
    }

    async fn start_turn(&self, _input: TurnInput) -> Result<(), AgentBackendError> {
        Err(AgentBackendError::Other(
            "injected test start failure".to_string(),
        ))
    }

    async fn interrupt(&self) -> Result<(), AgentBackendError> {
        Ok(())
    }

    async fn respond_permission(
        &self,
        _response: PermissionResponse,
    ) -> Result<(), AgentBackendError> {
        Ok(())
    }

    async fn set_model(&self, _model: &ModelId) -> Result<(), AgentBackendError> {
        Ok(())
    }

    async fn close(&self) {}
}
