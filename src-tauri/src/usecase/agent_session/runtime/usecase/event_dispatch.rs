struct TurnStartPayload {
    prompt: String,
    images: Vec<ImageAttachment>,
    mentions: Vec<crate::domain::code::MentionReference>,
    permission_mode: PermissionMode,
    plan_mode: bool,
    permission_profile_id: Option<String>,
    editor_context: Option<AgentEditorContext>,
    system_prompt: Option<String>,
    accepted_execution_identity: Option<AcceptedTurnExecutionIdentity>,
}

#[derive(Default)]
pub(super) struct RuntimeEventPostActions {
    workflow_notification: Option<WorkflowTurnCompleteNotification>,
    runtime_shutdowns: Vec<RuntimeShutdown>,
    drain_next_queued_turn: bool,
}

enum RuntimeShutdown {
    Close(Arc<dyn AgentSessionRuntime>),
}

impl RuntimeEventPostActions {
    fn workflow(notification: Option<WorkflowTurnCompleteNotification>) -> Self {
        Self {
            workflow_notification: notification,
            ..Self::default()
        }
    }

    fn drain(&mut self) {
        self.drain_next_queued_turn = true;
    }

    pub(super) fn close_runtime(&mut self, runtime: Option<Arc<dyn AgentSessionRuntime>>) {
        if let Some(runtime) = runtime {
            self.runtime_shutdowns.push(RuntimeShutdown::Close(runtime));
        }
    }
}

pub(super) async fn run_runtime_event_post_actions(
    ctx: &RuntimeContext,
    session_id: &str,
    actions: RuntimeEventPostActions,
) {
    if let Some(notification) = actions.workflow_notification {
        dispatch_workflow_turn_complete_notification(
            &ctx.workflow_turn_complete_notifier,
            notification,
        )
        .await;
    }
    for shutdown in actions.runtime_shutdowns {
        match shutdown {
            RuntimeShutdown::Close(runtime) => {
                runtime.close().await;
            }
        }
    }
    if actions.drain_next_queued_turn {
        let _session_guard = ctx.session_locks.acquire(session_id).await;
        start_next_queued_turn(ctx, session_id).await;
    }
}

pub(super) async fn turn_completion_post_actions(
    ctx: &RuntimeContext,
    session_id: &str,
    workflow_notification: Option<WorkflowTurnCompleteNotification>,
) -> RuntimeEventPostActions {
    let queue_paused = {
        let sessions = ctx.sessions.lock().await;
        sessions
            .get(session_id)
            .is_some_and(|state| state.queue_is_paused())
    };
    let mut actions = RuntimeEventPostActions::workflow(workflow_notification);
    if !queue_paused {
        actions.drain();
    }
    actions
}

#[cfg(test)]
pub(super) async fn append_session_events_blocking(
    ctx: &RuntimeContext,
    session_id: &str,
    events: Vec<AgentSessionEvent>,
) -> Result<(), String> {
    let session_store = Arc::clone(&ctx.session_store);
    let data_dir = Arc::clone(&ctx.data_dir);
    let session_id = session_id.to_string();
    tokio::task::spawn_blocking(move || {
        session_store.append_session_events(&data_dir, &session_id, &events)
    })
    .await
    .map_err(|error| format!("Failed to join session event append task: {error}"))?
}

pub(super) async fn append_user_session_events_blocking(
    ctx: &RuntimeContext,
    session_id: &str,
    events: Vec<AgentSessionEvent>,
) -> Result<(), String> {
    let session_store = Arc::clone(&ctx.session_store);
    let data_dir = Arc::clone(&ctx.data_dir);
    let session_id = session_id.to_string();
    tokio::task::spawn_blocking(move || {
        session_store.append_session_events_from_user(&data_dir, &session_id, &events)
    })
    .await
    .map_err(|error| format!("Failed to join user session event append task: {error}"))?
}

async fn dispatch_workflow_turn_complete_notification(
    workflow_turn_complete_notifier: &Arc<RwLock<Option<Arc<dyn WorkflowTurnCompleteNotifier>>>>,
    notification: WorkflowTurnCompleteNotification,
) {
    let workflow_notifier = workflow_turn_complete_notifier
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    if let Some(workflow_notifier) = workflow_notifier {
        workflow_notifier.turn_completed(notification).await;
    }
}

async fn dispatch_workflow_stall_observed_notification(
    workflow_stall_notifier: &Arc<RwLock<Option<Arc<dyn WorkflowStallNotifier>>>>,
    notification: WorkflowStallObservedNotification,
) {
    let workflow_notifier = workflow_stall_notifier
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    if let Some(workflow_notifier) = workflow_notifier {
        workflow_notifier.stall_observed(notification).await;
    }
}

async fn dispatch_workflow_stall_cleared_notification(
    workflow_stall_notifier: &Arc<RwLock<Option<Arc<dyn WorkflowStallNotifier>>>>,
    notification: WorkflowStallClearedNotification,
) -> Result<(), WorkflowError> {
    let workflow_notifier = workflow_stall_notifier
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    if let Some(workflow_notifier) = workflow_notifier {
        workflow_notifier.stall_cleared(notification).await?;
    }
    Ok(())
}

async fn dispatch_stall_cleared_notifications(
    ctx: &RuntimeContext,
    session_id: &str,
) -> Result<(), WorkflowError> {
    dispatch_workflow_stall_cleared_notification(
        &ctx.workflow_stall_notifier,
        ctx.projection_gateway.workflow_stall_cleared(session_id),
    )
    .await?;
    let cleared_stall = {
        let mut sessions = ctx.sessions.lock().await;
        sessions
            .get_mut(session_id)
            .map(|state| state.mark_progress(std::time::Instant::now()))
            .unwrap_or(false)
    };
    if cleared_stall {
        ctx.notifier.stall_cleared(session_id);
    }
    Ok(())
}

async fn record_first_backend_event_if_needed(ctx: &RuntimeContext, session_id: &str) {
    let elapsed = {
        let mut sessions = ctx.sessions.lock().await;
        let Some(state) = sessions.get_mut(session_id) else {
            return;
        };
        state.record_first_backend_event(std::time::Instant::now())
    };
    if let Some(elapsed) = elapsed {
        record_agent_turn_duration_detached(
            ctx,
            session_id.to_string(),
            crate::other::telemetry::AgentTurn::FirstBackendEvent,
            elapsed,
        );
    }
}

fn record_agent_turn_duration_detached(
    ctx: &RuntimeContext,
    session_id: String,
    metric: crate::other::telemetry::AgentTurn,
    elapsed: Duration,
) {
    let session_store = Arc::clone(&ctx.session_store);
    let data_dir = ctx.data_dir.clone();
    let spawner = Arc::clone(&ctx.spawner);
    spawner.spawn(Box::pin(async move {
        let Some(dims) = session_telemetry_dimensions(&session_store, &data_dir, &session_id)
        else {
            return;
        };
        crate::other::telemetry::record_agent_turn_duration(metric, &dims, elapsed);
    }));
}

fn runtime_event_kind(event: &AgentRuntimeEvent) -> &'static str {
    match event {
        AgentRuntimeEvent::SessionEstablished { .. } => "SessionEstablished",
        AgentRuntimeEvent::BackendSessionCleared => "BackendSessionCleared",
        AgentRuntimeEvent::PartsMerged(_) => "PartsMerged",
        AgentRuntimeEvent::PermissionRequested(_) => "PermissionRequested",
        AgentRuntimeEvent::PermissionModeChanged(_) => "PermissionModeChanged",
        AgentRuntimeEvent::SlashCommandsUpdated(_) => "SlashCommandsUpdated",
        AgentRuntimeEvent::TokenUsageUpdated(_) => "TokenUsageUpdated",
        AgentRuntimeEvent::KeepAlive => "KeepAlive",
        AgentRuntimeEvent::TurnCompleted(_) => "TurnCompleted",
        AgentRuntimeEvent::Fatal { .. } => "Fatal",
    }
}

async fn reconcile_pending_recovery_message(
    ctx: &RuntimeContext,
    session_id: &str,
) -> Result<(), String> {
    let Some(meta) = ctx
        .session_store
        .get_session_meta(&ctx.data_dir, session_id)?
    else {
        return Ok(());
    };
    let Some(pending) = meta.pending_recovery_message else {
        return Ok(());
    };
    match &pending {
        PendingRecoveryMessage::Notice { message_id, .. } => {
            persist_and_publish_recovery_notice(ctx, session_id, &pending, message_id)?;
        }
        PendingRecoveryMessage::Error {
            message_id, error, ..
        } => {
            persist_and_publish_recovery_error(ctx, session_id, &pending, message_id, error)
                .await?;
        }
    }
    Ok(())
}

fn reconcile_pending_recovery_message_detached(
    ctx: &RuntimeContext,
    session_id: String,
    publication_kind: &'static str,
) {
    let ctx = ctx.clone();
    let spawner = Arc::clone(&ctx.spawner);
    spawner.spawn(Box::pin(async move {
        if let Err(error) = reconcile_pending_recovery_message(&ctx, &session_id).await {
            log::warn!(
                "failed to persist {publication_kind} for {session_id}; it remains pending: {error}"
            );
        }
    }));
}

async fn clear_provider_session_establishment_if_current(
    ctx: &RuntimeContext,
    session_id: &str,
    runtime_epoch: u64,
    observation_id: &str,
) {
    let _runtime_event_guard = ctx.runtime_event_locks.acquire(session_id).await;
    let mut sessions = ctx.sessions.lock().await;
    if let Some(state) = sessions.get_mut(session_id) {
        if state.owns_runtime_epoch(runtime_epoch) {
            state.clear_provider_establishment_if_current(observation_id);
        }
    }
}

fn retry_provider_session_establishment(
    ctx: &RuntimeContext,
    session_id: String,
    runtime_epoch: u64,
    observation_id: String,
    backend_session_id: String,
    context_carry: Option<ContextCarryState>,
) {
    let ctx = ctx.clone();
    let spawner = Arc::clone(&ctx.spawner);
    spawner.spawn(Box::pin(async move {
        let mut retry_delay = Duration::from_millis(25);
        loop {
            let still_current = {
                let sessions = ctx.sessions.lock().await;
                sessions.get(&session_id).is_some_and(|state| {
                    state.owns_runtime_epoch(runtime_epoch)
                        && state.provider_establishment_is_current(&observation_id)
                })
            };
            if !still_current {
                return;
            }

            let expected_provider_session_generation = match ctx
                .session_store
                .get_session_meta(&ctx.data_dir, &session_id)
            {
                Ok(Some(meta)) => meta.provider_session_generation,
                Ok(None) => {
                    clear_provider_session_establishment_if_current(
                        &ctx,
                        &session_id,
                        runtime_epoch,
                        &observation_id,
                    )
                    .await;
                    return;
                }
                Err(error) => {
                    log::warn!(
                        "provider establishment generation read remains pending for {session_id}: {error}"
                    );
                    tokio::time::sleep(retry_delay).await;
                    retry_delay = next_recovery_retry_delay(retry_delay);
                    continue;
                }
            };

            match ctx.session_store.record_backend_session_established(
                &ctx.data_dir,
                &session_id,
                expected_provider_session_generation,
                &observation_id,
                backend_session_id.clone(),
                context_carry,
            ) {
                Ok(ProviderSessionEstablishmentOutcome::Settled(meta)) => {
                    let settled = {
                        let _runtime_event_guard =
                            ctx.runtime_event_locks.acquire(&session_id).await;
                        let mut sessions = ctx.sessions.lock().await;
                        sessions.get_mut(&session_id).is_some_and(|state| {
                            state.owns_runtime_epoch(runtime_epoch)
                                && state
                                    .settle_provider_establishment_if_current(&observation_id)
                        })
                    };
                    if !settled {
                        return;
                    }
                    let notifier = Arc::clone(&ctx.notifier);
                    let notification_session_id = session_id.clone();
                    let notification_spawner = Arc::clone(&ctx.spawner);
                    notification_spawner.spawn(Box::pin(async move {
                        notifier.context_carry_updated(
                            &notification_session_id,
                            meta.agent_session_id,
                            meta.context_carry,
                            meta.updated_at,
                        );
                    }));
                    return;
                }
                Ok(
                    ProviderSessionEstablishmentOutcome::Missing
                    | ProviderSessionEstablishmentOutcome::Fenced,
                ) => {
                    clear_provider_session_establishment_if_current(
                        &ctx,
                        &session_id,
                        runtime_epoch,
                        &observation_id,
                    )
                    .await;
                    return;
                }
                Err(error) => {
                    log::warn!(
                        "provider establishment observation remains pending for {session_id}: {error}"
                    );
                }
            }
            tokio::time::sleep(retry_delay).await;
            retry_delay = next_recovery_retry_delay(retry_delay);
        }
    }));
}

fn persist_and_publish_recovery_notice(
    ctx: &RuntimeContext,
    session_id: &str,
    pending: &PendingRecoveryMessage,
    message_id: &str,
) -> Result<(), String> {
    let parts = vec![DomainMessagePart::SystemNotification {
        notification_type: DomainSystemNotificationType::SessionRecovery,
        status: "recovered".to_string(),
        label: "backend セッションを作り直したため文脈は引き継がれません".to_string(),
        detail: None,
        hook_id: None,
    }];
    let message = ChatMessage {
        id: message_id.to_string(),
        role: MessageRole::Agent,
        content: String::new(),
        thinking: None,
        activities: None,
        parts: Some(parts),
        streaming_final_seq: 0,
        timestamp: crate::usecase::agent_session::session::now_timestamp(),
        mentions: None,
    };
    let inserted = ctx.session_store.publish_pending_recovery_message(
        &ctx.data_dir,
        session_id,
        pending,
        message.clone(),
    )?;
    if inserted {
        ctx.notifier
            .pending_message_consumed(session_id, None, None, message);
    }
    Ok(())
}

async fn persist_and_publish_recovery_error(
    ctx: &RuntimeContext,
    session_id: &str,
    pending: &PendingRecoveryMessage,
    message_id: &str,
    error: &str,
) -> Result<(), String> {
    let content = format!("backend session recovery failed: {error}");
    let persisted = ctx
        .session_store
        .canonical_message_projection(session_id, message_id)?;
    if let Some(mut message) = persisted {
        let mut parts = message.parts.clone().unwrap_or_default();
        let error_part = MessagePart::Error {
            content,
            parent_tool_use_id: None,
        };
        if !parts.contains(&error_part) {
            crate::domain::agent_session::entities::merge_part(&mut parts, error_part);
            message.streaming_final_seq =
                crate::domain::agent_session::services::next_stream_sequence(
                    message.streaming_final_seq,
                );
            message.timestamp = crate::usecase::agent_session::session::now_timestamp();
            message.parts = Some(parts.clone());
            ctx.session_store.publish_pending_recovery_message(
                &ctx.data_dir,
                session_id,
                pending,
                message.clone(),
            )?;
            let _ = ctx.notifier.streaming_delta(AgentStreamingDeltaPayload {
                chat_session_id: session_id.to_string(),
                message_id: message.id,
                seq: message.streaming_final_seq,
                snapshot: true,
                parts,
                message: None,
            });
        } else {
            ctx.session_store.publish_pending_recovery_message(
                &ctx.data_dir,
                session_id,
                pending,
                message,
            )?;
        }
        return Ok(());
    }

    let message = ChatMessage {
        id: message_id.to_string(),
        role: MessageRole::Agent,
        content: String::new(),
        thinking: None,
        activities: None,
        parts: Some(vec![MessagePart::Error {
            content,
            parent_tool_use_id: None,
        }]),
        streaming_final_seq: 0,
        timestamp: crate::usecase::agent_session::session::now_timestamp(),
        mentions: None,
    };
    let inserted = ctx.session_store.publish_pending_recovery_message(
        &ctx.data_dir,
        session_id,
        pending,
        message.clone(),
    )?;
    if inserted {
        ctx.notifier
            .pending_message_consumed(session_id, None, None, message);
    }
    Ok(())
}

async fn apply_runtime_event(
    ctx: &RuntimeContext,
    session_id: &str,
    runtime_epoch: u64,
    event_received_at: f64,
    event: AgentRuntimeEvent,
) -> Result<RuntimeEventPostActions, String> {
    let (
        is_current_runtime,
        terminal_committed,
        provider_establishment_in_flight,
        recovery_completion_in_flight,
        recovery_failure_in_flight,
    ) = {
        let sessions = ctx.sessions.lock().await;
        sessions
            .get(session_id)
            .map_or((false, false, false, false, false), |state| {
                (
                    state.owns_runtime_epoch(runtime_epoch),
                    state.has_active_turn_lease() && state.terminal_matches_current_or_last(),
                    state.has_pending_provider_establishment(),
                    state
                        .backend_recovery
                        .as_ref()
                        .is_some_and(|recovery| recovery.attempt.completion_in_flight()),
                    state
                        .backend_recovery
                        .as_ref()
                        .is_some_and(|recovery| recovery.attempt.failure_in_flight()),
                )
            })
    };
    match decide_runtime_event_admission(RuntimeEventAdmissionFacts {
        event: &event,
        current_runtime: is_current_runtime,
        terminal_committed,
        provider_establishment_in_flight,
        recovery_completion_in_flight,
        recovery_failure_in_flight,
    }) {
        RuntimeEventAdmission::Apply => {}
        RuntimeEventAdmission::DropStaleRuntime => {
            log::debug!(
                "dropping {} from stale runtime epoch {runtime_epoch} for {session_id}",
                runtime_event_kind(&event)
            );
            return Ok(RuntimeEventPostActions::default());
        }
        RuntimeEventAdmission::DropAfterTerminal => {
            log::debug!(
                "dropping {} after durable terminal commit for {session_id}",
                runtime_event_kind(&event)
            );
            return Ok(RuntimeEventPostActions::default());
        }
        RuntimeEventAdmission::RejectRecoveryFailureSettling => {
            return Err("backend recovery failure is still settling".to_string());
        }
        RuntimeEventAdmission::RejectProviderEstablishmentSettling => {
            return Err("provider establishment observation is still settling".to_string());
        }
        RuntimeEventAdmission::RejectRecoveryCompletionSettling => {
            return Err("backend recovery completion is still settling".to_string());
        }
    }
    record_first_backend_event_if_needed(ctx, session_id).await;
    ctx.notifier.runtime_event_debug(session_id, &event);
    match event {
        AgentRuntimeEvent::SessionEstablished {
            backend_session_id,
            resume,
        } => {
            let (already_established, recovery_active) = {
                let sessions = ctx.sessions.lock().await;
                sessions
                    .get(session_id)
                    .map(|state| {
                        (
                            state.provider_session_is_established(),
                            state.backend_recovery.is_some(),
                        )
                    })
                    .unwrap_or((false, false))
            };
            match decide_session_established_event(
                &resume,
                already_established,
                recovery_active,
            ) {
                SessionEstablishedEventDecision::RecoverResumeMismatch => {
                    let recovery_id = runtime_event_recovery_id(
                        session_id,
                        runtime_epoch,
                        event_received_at,
                        BackendSessionRecoveryReason::ResumeMismatch,
                        &backend_session_id,
                    );
                    recover_backend_session_with_identity_lock_state(
                        ctx,
                        session_id,
                        BackendSessionRecoveryReason::ResumeMismatch,
                        recovery_id,
                        true,
                    )
                    .await
                    .map_err(|error| {
                        format!(
                            "resume-mismatch recovery trigger could not be durably handled: {error}"
                        )
                    })?;
                    return Ok(RuntimeEventPostActions::default());
                }
                SessionEstablishedEventDecision::IgnoreAlreadyEstablished => {
                    return Ok(RuntimeEventPostActions::default());
                }
                SessionEstablishedEventDecision::Observe => {}
            }
            let recovery_identity = {
                let mut sessions = ctx.sessions.lock().await;
                let Some(state) = sessions.get_mut(session_id) else {
                    return Ok(RuntimeEventPostActions::default());
                };
                if !state.owns_runtime_epoch(runtime_epoch) {
                    return Ok(RuntimeEventPostActions::default());
                }
                let generation = state.generation();
                let recovery_id = match state.backend_recovery.as_mut() {
                    Some(recovery) => {
                        if recovery.attempt.observe_provider_identity(&backend_session_id)
                            == ProviderIdentityObservation::Conflict
                        {
                            return Err(
                                "backend recovery observed conflicting provider identities"
                                    .to_string(),
                            );
                        }
                        Some(recovery.attempt.recovery_id().to_string())
                    }
                    None => None,
                };
                if recovery_id.is_some() {
                    state.mark_provider_session_established();
                }
                recovery_id.map(|recovery_id| (generation, recovery_id))
            };
            if let Some((generation, recovery_id)) = recovery_identity {
                retry_backend_session_recovery_completion(
                    ctx,
                    session_id.to_string(),
                    generation,
                    recovery_id,
                );
                return Ok(RuntimeEventPostActions::default());
            }
            let context_carry = context_carry_for_established_resume(&resume)
                .map_err(|_| "resume mismatch reached provider metadata update".to_string())?;
            let observation_id = runtime_provider_session_observation_id(
                session_id,
                runtime_epoch,
                event_received_at,
                &backend_session_id,
                context_carry.as_ref(),
            );
            {
                let mut sessions = ctx.sessions.lock().await;
                let Some(state) = sessions.get_mut(session_id) else {
                    return Ok(RuntimeEventPostActions::default());
                };
                if !state.owns_runtime_epoch(runtime_epoch) {
                    return Ok(RuntimeEventPostActions::default());
                }
                match state.observe_provider_establishment(&observation_id) {
                    ProviderEstablishmentObservation::Start => {}
                    ProviderEstablishmentObservation::AlreadyEstablished
                    | ProviderEstablishmentObservation::AlreadyPending => {
                        return Ok(RuntimeEventPostActions::default());
                    }
                    ProviderEstablishmentObservation::Conflict => {
                        return Err(
                            "runtime observed conflicting provider establishment identities"
                                .to_string(),
                        );
                    }
                }
            }
            retry_provider_session_establishment(
                ctx,
                session_id.to_string(),
                runtime_epoch,
                observation_id,
                backend_session_id,
                context_carry,
            );
        }
        AgentRuntimeEvent::BackendSessionCleared => {
            let recovery_id = runtime_event_recovery_id(
                session_id,
                runtime_epoch,
                event_received_at,
                BackendSessionRecoveryReason::BackendSessionLost,
                "backend-session-cleared",
            );
            recover_backend_session_with_identity_lock_state(
                ctx,
                session_id,
                BackendSessionRecoveryReason::BackendSessionLost,
                recovery_id,
                true,
            )
            .await
            .map_err(|error| {
                format!("backend-session-cleared recovery trigger could not be durably handled: {error}")
            })?;
        }
        AgentRuntimeEvent::PartsMerged(parts) => {
            apply_parts(ctx, session_id, parts, StreamingApplyMode::Coalesced)
                .await
                .map_err(|error| format!("streaming parts commit failed: {error}"))?;
        }
        AgentRuntimeEvent::PermissionRequested(request) => {
            let lifecycle_repository = ctx.lifecycle_repository();
            if lifecycle_repository.is_none() {
                #[cfg(not(test))]
                return Err(
                    "agent-session lifecycle repository is not configured".to_string()
                );
            }
            if let Some(repository) = lifecycle_repository {
                let mut session = repository
                    .restore_session(session_id)
                    .await
                    .map_err(|error| {
                        format!("failed to restore permission session aggregate: {error:?}")
                    })?;
                let turn_id = session.active_turn_id().ok_or_else(|| {
                    "permission request has no canonical active turn".to_string()
                })?;
                match session.request_permission(turn_id, request.clone()) {
                    TransitionOutcome::Applied | TransitionOutcome::AlreadyApplied => {}
                    TransitionOutcome::NotApplicable
                    | TransitionOutcome::Rejected(_) => {
                        return Err(
                            "permission request was rejected by the session aggregate"
                                .to_string(),
                        );
                    }
                }
            }
            let pending = ctx.projection_gateway.pending_permission_request(&request);
            let persisted = apply_parts(
                ctx,
                session_id,
                vec![DomainMessagePart::permission(request)],
                StreamingApplyMode::Immediate,
            )
            .await
            .map_err(|error| format!("permission request commit failed: {error}"))?;
            if persisted {
                if let Some(pending) = pending {
                    let pending_permission_state_revision = {
                        let mut sessions = ctx.sessions.lock().await;
                        sessions.get_mut(session_id).map(|state| {
                            state.set_pending_permission_request(pending.clone())
                        })
                    };
                    emit_session_state_change(
                        &ctx.session_store,
                        &ctx.notifier,
                        &ctx.status_center,
                        &ctx.status_notifier,
                        &ctx.data_dir,
                        session_id,
                        StateChange {
                            turn_phase: TurnPhase::WaitingPermission,
                            queue_paused: None,
                            pending_permission_request: Some(pending),
                            pending_permission_state_revision,
                            exit_code: None,
                            completed_at: None,
                            interrupted: false,
                            session_state: Some(SessionState::Active),
                        },
                    );
                }
            }
        }
        AgentRuntimeEvent::PermissionModeChanged(mode) => {
            if let Some(saved_mode) =
                resync_permission_mode(&ctx.session_store, &ctx.data_dir, session_id, mode)
            {
                ctx.notifier
                    .permission_mode_changed(session_id, saved_mode.as_str());
            }
        }
        AgentRuntimeEvent::SlashCommandsUpdated(commands) => {
            ctx.notifier
                .supported_commands_updated(session_id, commands);
        }
        AgentRuntimeEvent::TokenUsageUpdated(usage) => {
            let usage = ctx.projection_gateway.token_usage(usage);
            {
                let mut sessions = ctx.sessions.lock().await;
                if let Some(state) = sessions.get_mut(session_id) {
                    state.latest_token_usage = Some(usage);
                }
            }
            ctx.notifier.token_usage_updated(session_id, usage);
        }
        AgentRuntimeEvent::KeepAlive => {
            let cleared_stall = {
                let mut sessions = ctx.sessions.lock().await;
                if let Some(state) = sessions.get_mut(session_id) {
                    if state.has_active_turn_lease() {
                        state.record_progress(std::time::Instant::now())
                    } else {
                        false
                    }
                } else {
                    false
                }
            };
            if cleared_stall {
                if let Err(error) = dispatch_stall_cleared_notifications(ctx, session_id).await {
                    log::warn!(
                        "workflow stall-cleared notification failed for {session_id}: {error}"
                    );
                }
            }
        }
        AgentRuntimeEvent::TurnCompleted(result) => {
            let trailing_fatal_message = result.trailing_fatal_message().map(str::to_owned);
            let wait_for_trailing_fatal = if trailing_fatal_message.is_some() {
                let sessions = ctx.sessions.lock().await;
                sessions
                    .get(session_id)
                    .is_some_and(|state| state.admits_trailing_fatal_wait(true))
            } else {
                false
            };
            let workflow_notification = match complete_turn(ctx, session_id, None, result).await {
                Ok(notification) => notification,
                Err(error) => {
                    return Err(format!("terminal commit failed: {error}"));
                }
            };
            if wait_for_trailing_fatal {
                let mut sessions = ctx.sessions.lock().await;
                if let Some(state) = sessions.get_mut(session_id) {
                    state.defer_trailing_fatal(trailing_fatal_message);
                }
                return Ok(RuntimeEventPostActions::workflow(workflow_notification));
            }
            return Ok(turn_completion_post_actions(ctx, session_id, workflow_notification).await);
        }
        AgentRuntimeEvent::Fatal { message } => {
            log::warn!("agent runtime fatal for {session_id}: {message}");
            let recovery_in_progress = {
                let sessions = ctx.sessions.lock().await;
                sessions
                    .get(session_id)
                    .is_some_and(|state| state.backend_recovery.is_some())
            };
            if recovery_in_progress {
                let failure_owned =
                    schedule_backend_session_recovery_failure(ctx, session_id, message.clone())
                        .await
                        .map_err(|error| {
                            format!(
                            "backend recovery fatal observation could not be handed off: {error}"
                        )
                        })?;
                return if failure_owned {
                    Ok(RuntimeEventPostActions::default())
                } else {
                    Err("backend recovery completion is still settling".to_string())
                };
            }
            let (should_complete_crash, trailing_completed_crash) = {
                let mut sessions = ctx.sessions.lock().await;
                sessions
                    .get_mut(session_id)
                    .map_or((false, false), |state| match state.observe_fatal(&message) {
                        RuntimeFatalObservation::CompleteCurrentTurn => (true, false),
                        RuntimeFatalObservation::MatchesCompletedCrash => (false, true),
                        RuntimeFatalObservation::Unrelated => (false, false),
                    })
            };
            let mut actions = RuntimeEventPostActions::default();
            if should_complete_crash {
                match complete_turn(
                    ctx,
                    session_id,
                    None,
                    TurnResult::Interrupted {
                        reason: DomainInterruptReason::Crash,
                        error: Some(message.clone()),
                    },
                )
                .await
                {
                    Ok(notification) => actions.workflow_notification = notification,
                    Err(error) => {
                        return Err(format!("fatal terminal commit failed: {error}"));
                    }
                }
            }
            let runtime = {
                let mut sessions = ctx.sessions.lock().await;
                sessions
                    .get_mut(session_id)
                    .and_then(|state| state.runtime.take())
            };
            actions.close_runtime(runtime);
            {
                let mut sessions = ctx.sessions.lock().await;
                if let Some(state) = sessions.get_mut(session_id) {
                    state.release_turn_lease();
                    state.clear_stall_observation();
                }
            }
            if !should_complete_crash && !trailing_completed_crash {
                let completed_at = event_received_at;
                let message_id = runtime_error_message_id(
                    session_id,
                    runtime_epoch,
                    event_received_at,
                    &message,
                );
                let projected_message = ctx
                    .session_store
                    .append_error_episode_and_pause_queue(
                        &ctx.data_dir,
                        session_id,
                        ErrorEpisodeInput {
                            message_id: message_id.clone(),
                            reason: message.clone(),
                            at: completed_at,
                        },
                    )
                    .map(|(_, projected_message)| projected_message);
                match projected_message {
                    Ok(projected_message) => {
                        let parts = projected_message.parts.clone().unwrap_or_default();
                        {
                            let mut sessions = ctx.sessions.lock().await;
                            if let Some(state) = sessions.get_mut(session_id) {
                                state.last_agent_message_id = Some(message_id.clone());
                                state.observe_emitted_stream_sequence(1);
                                state.pause_queue_at(completed_at);
                            }
                        }
                        emit_streaming_delta_or_retry(
                            ctx,
                            session_id,
                            PendingStreamDelta {
                                message_id,
                                seq: 1,
                                snapshot: true,
                                parts,
                                message: Some(projected_message),
                                authoritative: true,
                            },
                        )
                        .await;
                        emit_session_state_change(
                            &ctx.session_store,
                            &ctx.notifier,
                            &ctx.status_center,
                            &ctx.status_notifier,
                            &ctx.data_dir,
                            session_id,
                            StateChange {
                                turn_phase: TurnPhase::Idle,
                                queue_paused: Some(true),
                                pending_permission_request: None,
                                pending_permission_state_revision: None,
                                exit_code: Some(1),
                                // Idle-Fatal creates a standalone message that already carries its
                                // backend timestamp. It must not finalize an older agent turn.
                                completed_at: None,
                                interrupted: true,
                                session_state: Some(SessionState::Error),
                            },
                        );
                    }
                    Err(error) => {
                        if let Some(RuntimeShutdown::Close(runtime)) =
                            actions.runtime_shutdowns.pop()
                        {
                            if let Some(state) = ctx.sessions.lock().await.get_mut(session_id) {
                                state.runtime = Some(runtime);
                            }
                        }
                        return Err(format!("idle fatal projection commit failed: {error}"));
                    }
                }
            } else if trailing_completed_crash {
                log::debug!(
                    "suppressed trailing fatal projection for completed crash in {session_id}"
                );
            }
            return Ok(actions);
        }
    };
    Ok(RuntimeEventPostActions::default())
}
