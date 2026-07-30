struct TurnContextRestorePolicy {
    plan: ContextRestorePlan,
    recovery_restore_required: bool,
    expected_provider_session_generation: u64,
}

fn context_restore_policy_for_turn(
    ctx: &RuntimeContext,
    session_id: &str,
    streaming_agent_message_id: &str,
    had_runtime: bool,
) -> Result<TurnContextRestorePolicy, String> {
    let Some(meta) = ctx
        .session_store
        .get_session_meta(&ctx.data_dir, session_id)?
    else {
        return Ok(TurnContextRestorePolicy {
            plan: ContextRestorePlan::NoContext,
            recovery_restore_required: false,
            expected_provider_session_generation: 0,
        });
    };
    let (reinjection_required, expected_provider_session_generation) =
        match decide_context_restore_preparation(
            had_runtime,
            meta.provider_session_generation,
            meta.context_reinjection_generation,
        ) {
            ContextRestorePreparationDecision::Skip {
                expected_provider_session_generation,
            } => {
                return Ok(TurnContextRestorePolicy {
                    plan: ContextRestorePlan::NoContext,
                    recovery_restore_required: false,
                    expected_provider_session_generation,
                })
            }
            ContextRestorePreparationDecision::Restore {
                reinjection_required,
                expected_provider_session_generation,
            } => (reinjection_required, expected_provider_session_generation),
        };

    let mut persisted = ctx
        .session_store
        .load_full_session_for_restore(&ctx.data_dir, session_id)?;
    if reinjection_required {
        if let Some(session) = persisted.as_mut() {
            session.agent_session_id = None;
            session.context_carry = None;
        }
    }
    Ok(TurnContextRestorePolicy {
        plan: context_restore_plan_for_session_before_turn(
            persisted.as_ref(),
            streaming_agent_message_id,
        ),
        recovery_restore_required: reinjection_required,
        expected_provider_session_generation,
    })
}

fn context_restore_policy_before_human_message(
    ctx: &RuntimeContext,
    session_id: &str,
    human_message_id: &str,
    had_runtime: bool,
) -> Result<TurnContextRestorePolicy, String> {
    let Some(meta) = ctx
        .session_store
        .get_session_meta(&ctx.data_dir, session_id)?
    else {
        return Ok(TurnContextRestorePolicy {
            plan: ContextRestorePlan::NoContext,
            recovery_restore_required: false,
            expected_provider_session_generation: 0,
        });
    };
    let (reinjection_required, expected_provider_session_generation) =
        match decide_context_restore_preparation(
            had_runtime,
            meta.provider_session_generation,
            meta.context_reinjection_generation,
        ) {
            ContextRestorePreparationDecision::Skip {
                expected_provider_session_generation,
            } => {
                return Ok(TurnContextRestorePolicy {
                    plan: ContextRestorePlan::NoContext,
                    recovery_restore_required: false,
                    expected_provider_session_generation,
                })
            }
            ContextRestorePreparationDecision::Restore {
                reinjection_required,
                expected_provider_session_generation,
            } => (reinjection_required, expected_provider_session_generation),
        };

    let mut persisted = ctx
        .session_store
        .load_full_session_for_restore(&ctx.data_dir, session_id)?;
    if let Some(session) = persisted.as_mut() {
        let boundary = session
            .messages
            .iter()
            .position(|message| message.id == human_message_id)
            .ok_or_else(|| {
                format!(
                    "accepted queued human message is absent from restore history: {human_message_id}"
                )
            })?;
        session.messages.truncate(boundary);
        if reinjection_required {
            session.agent_session_id = None;
            session.context_carry = None;
        }
    }
    Ok(TurnContextRestorePolicy {
        plan: context_restore_plan_for_session(persisted.as_ref()),
        recovery_restore_required: reinjection_required,
        expected_provider_session_generation,
    })
}

fn context_restore_plan_for_backend_recovery(
    ctx: &RuntimeContext,
    session_id: &str,
    streaming_agent_message_id: &str,
) -> Result<ContextRestorePlan, String> {
    let mut persisted = ctx
        .session_store
        .load_full_session_for_restore(&ctx.data_dir, session_id)?;
    if let Some(session) = persisted.as_mut() {
        // `begin_backend_session_recovery` deliberately clears the dead
        // provider identity and marks carry Failed. That durable marker fences
        // ordinary turns, but the already-accepted current turn must rebuild
        // the history that preceded its exact human input on the replacement
        // runtime.
        session.agent_session_id = None;
        session.context_carry = None;
    }
    Ok(context_restore_plan_for_session_before_turn(
        persisted.as_ref(),
        streaming_agent_message_id,
    ))
}

fn complete_context_restore_after_start(
    ctx: &RuntimeContext,
    session_id: &str,
    request: ContextRestoreCompletionRequest,
) -> Result<(), String> {
    if let Some(meta) = ctx
        .session_store
        .complete_context_restore_after_start_if_current(&ctx.data_dir, session_id, request)?
    {
        ctx.notifier.context_carry_updated(
            session_id,
            meta.agent_session_id,
            meta.context_carry,
            meta.updated_at,
        );
    }
    Ok(())
}

fn complete_context_restore_after_start_or_retry(
    ctx: &RuntimeContext,
    session_id: String,
    runtime_epoch: u64,
    request: ContextRestoreCompletionRequest,
) {
    if let Err(error) = complete_context_restore_after_start(ctx, &session_id, request) {
        log::warn!("context restore completion will retry for {session_id}: {error}");
        retry_context_restore_completion(ctx, session_id, runtime_epoch, request);
    }
}

fn retry_context_restore_completion(
    ctx: &RuntimeContext,
    session_id: String,
    runtime_epoch: u64,
    request: ContextRestoreCompletionRequest,
) {
    let ctx = ctx.clone();
    let spawner = Arc::clone(&ctx.spawner);
    spawner.spawn(Box::pin(async move {
        let mut retry_delay = Duration::from_millis(25);
        loop {
            let still_current = {
                let sessions = ctx.sessions.lock().await;
                sessions
                    .get(&session_id)
                    .is_some_and(|state| state.owns_runtime_epoch(runtime_epoch))
            };
            if !still_current {
                return;
            }
            match complete_context_restore_after_start(&ctx, &session_id, request) {
                Ok(()) => return,
                Err(error) => {
                    if matches!(
                        ctx.session_store
                            .get_session_meta(&ctx.data_dir, &session_id),
                        Ok(None)
                    ) {
                        return;
                    }
                    log::warn!(
                        "context restore completion remains pending for {session_id}: {error}"
                    );
                }
            }
            tokio::time::sleep(retry_delay).await;
            retry_delay = next_recovery_retry_delay(retry_delay);
        }
    }));
}

fn mark_accepted_turn_running_or_retry(
    ctx: &RuntimeContext,
    session_id: &str,
    generation: u64,
    operation_id: String,
    obligation_id: String,
    turn_id: u64,
) {
    let driver = ctx
        .accepted_send_obligation_driver
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    let Some(driver) = driver else {
        log::error!("accepted turn has no running-status driver [{operation_id}/{obligation_id}]");
        return;
    };
    let ctx = ctx.clone();
    let session_id = session_id.to_string();
    let spawner = Arc::clone(&ctx.spawner);
    spawner.spawn(Box::pin(async move {
        let mut retry_delay = Duration::from_millis(25);
        loop {
            let still_owned = {
                let sessions = ctx.sessions.lock().await;
                sessions
                    .get(&session_id)
                    .is_some_and(|state| state.owns_turn(generation, turn_id))
            };
            if !still_owned {
                return;
            }
            if driver
                .mark_turn_running(&operation_id, &obligation_id, turn_id)
                .await
                .is_ok()
            {
                return;
            }
            retry_delay = next_recovery_retry_delay(retry_delay);
            tokio::time::sleep(retry_delay).await;
        }
    }));
}

async fn recover_backend_session(
    ctx: &RuntimeContext,
    session_id: &str,
    reason: BackendSessionRecoveryReason,
) -> Result<(), AgentRuntimeError> {
    recover_backend_session_with_identity(ctx, session_id, reason, uuid::Uuid::new_v4().to_string())
        .await
}

async fn recover_backend_session_with_identity(
    ctx: &RuntimeContext,
    session_id: &str,
    reason: BackendSessionRecoveryReason,
    recovery_id: String,
) -> Result<(), AgentRuntimeError> {
    recover_backend_session_with_identity_lock_state(ctx, session_id, reason, recovery_id, false)
        .await
}

async fn recover_backend_session_with_identity_lock_state(
    ctx: &RuntimeContext,
    session_id: &str,
    reason: BackendSessionRecoveryReason,
    recovery_id: String,
    runtime_event_lock_held: bool,
) -> Result<(), AgentRuntimeError> {
    // Stop acceptance is the terminal owner for the active turn. Its durable
    // QueuePaused projection closes the small interval before the production
    // gate installs the matching process-generation fence; the in-memory
    // fence covers all later provider events. Reopening here would resubmit an
    // input that Stop already owns and make the old runtime's terminal stale.
    let durable_queue_paused = ctx
        .session_store
        .load_queue_paused_at(&ctx.data_dir, session_id)
        .map_err(AgentRuntimeError::Other)?
        .is_some();
    let stop_owns_current_turn = {
        let sessions = ctx.sessions.lock().await;
        sessions.get(session_id).is_some_and(|state| {
            state.has_active_turn_lease()
                && (durable_queue_paused || state.interrupt_requested_for_current())
        })
    };
    if stop_owns_current_turn {
        return Ok(());
    }

    let existing_recovery = {
        let sessions = ctx.sessions.lock().await;
        sessions
            .get(session_id)
            .and_then(|state| state.backend_recovery.as_ref())
            .map(|recovery| {
                (
                    recovery.attempt.recovery_id().to_string(),
                    recovery.attempt.pending_failure().map(str::to_string),
                )
            })
    };
    if let Some((existing_recovery_id, pending_failure)) = existing_recovery {
        if existing_recovery_id == recovery_id {
            if let Some(error) = pending_failure {
                return if schedule_backend_session_recovery_failure(ctx, session_id, error).await? {
                    Ok(())
                } else {
                    Err(AgentRuntimeError::Other(format!(
                        "backend recovery completion is already settling for {session_id}"
                    )))
                };
            }
        }
        // A duplicate provider event must join the recovery already owning
        // the session. Only a retained terminal persistence failure above
        // needs another write attempt.
        return Ok(());
    }

    let recovery_start = ctx
        .session_store
        .begin_backend_session_recovery(&ctx.data_dir, session_id, &recovery_id, reason)
        .map_err(AgentRuntimeError::Other)?;
    let meta = match recovery_start {
        crate::usecase::agent_session::session::BackendSessionRecoveryStartOutcome::Started(
            meta,
        ) => *meta,
        crate::usecase::agent_session::session::BackendSessionRecoveryStartOutcome::SuppressedByQueuePause => {
            return Ok(());
        }
    };

    let backend_id = meta.backend_id.clone();
    let (completion, _) = tokio::sync::watch::channel(false);
    let (old_runtime, accepted_turn) = {
        let mut sessions = ctx.sessions.lock().await;
        let state = sessions
            .entry(session_id.to_string())
            .or_insert_with(|| RuntimeSessionState::new(backend_id));
        let runtime = state.runtime.take();
        let current_turn_id = state.active_turn_id();
        let current_turn = state.current_turn_input.take();
        let accepted_turn = match current_turn {
            Some(current_turn) => match decide_runtime_turn_recovery(
                current_turn_id,
                current_turn.accepted_operation_id.as_deref(),
                current_turn.execution_obligation_id.as_deref(),
                current_turn.existing_agent_message_id.as_deref(),
            ) {
                RuntimeTurnRecoveryDecision::RetainAccepted {
                    turn_id,
                    assistant_message_id,
                } => {
                    // The durable TurnExecution is already claimed. Retain that
                    // exact input as the current process owner and start a new
                    // process generation for the replacement runtime; never route
                    // it through the normal queued Pending -> EffectReserved claim
                    // again.
                    state.rollback_started_turn();
                    let generation =
                        state.register_turn_start_intent(turn_id, assistant_message_id.clone());
                    state.commit_turn_start(assistant_message_id);
                    state.current_turn_input = Some(current_turn.clone());
                    Some((current_turn, generation))
                }
                RuntimeTurnRecoveryDecision::Requeue => {
                    let mut current_turn = current_turn;
                    // Legacy turns, and any incomplete process-local accepted
                    // identity, remain explicitly queued instead of disappearing
                    // during recovery. The accepted queue fence will reject a
                    // partial identity without provider I/O.
                    current_turn.id = uuid::Uuid::new_v4().to_string();
                    state.rollback_started_turn();
                    state
                        .accepted_input_effects
                        .insert(current_turn.id.clone(), current_turn);
                    None
                }
            },
            None => {
                state.rollback_started_turn();
                None
            }
        };
        state.backend_recovery = Some(BackendSessionRecoveryState {
            attempt: BackendRecoveryAttempt::start(
                recovery_id.clone(),
                meta.provider_session_generation,
                reason,
                accepted_turn.is_some(),
            ),
            completion,
        });
        (runtime, accepted_turn)
    };
    if let Some(runtime) = old_runtime {
        runtime.close().await;
    }

    let session = match ctx
        .session_store
        .get_session_shell(&ctx.data_dir, session_id)
        .map_err(AgentRuntimeError::Other)
        .and_then(|session| {
            session
                .ok_or_else(|| AgentRuntimeError::Other(format!("Session not found: {session_id}")))
        }) {
        Ok(session) => session,
        Err(error) => {
            return fail_backend_recovery_before_claimed_turn_resume(
                ctx,
                session_id,
                accepted_turn.as_ref(),
                error.to_string(),
            )
            .await;
        }
    };
    let queued = if let Some((accepted_turn, _)) = &accepted_turn {
        Some(accepted_turn.clone())
    } else {
        let sessions = ctx.sessions.lock().await;
        sessions
            .get(session_id)
            .and_then(|state| {
                state.accepted_input_effects.values().min_by(|left, right| {
                    left.created_at
                        .total_cmp(&right.created_at)
                        .then_with(|| left.id.cmp(&right.id))
                })
            })
            .cloned()
    };
    let system_prompt = match queued
        .as_ref()
        .map(|queued| {
            build_queued_system_prompt(
                &ctx.session_store,
                ctx.branch_diff_context.as_deref(),
                ctx.instruction_source.as_ref(),
                &ctx.data_dir,
                &session,
                queued,
            )
        })
        .transpose()
        .map_err(AgentRuntimeError::Other)
        .map(Option::flatten)
    {
        Ok(system_prompt) => system_prompt,
        Err(error) => {
            return fail_backend_recovery_before_claimed_turn_resume(
                ctx,
                session_id,
                accepted_turn.as_ref(),
                error.to_string(),
            )
            .await;
        }
    };

    let runtime = match open_runtime_for_session(ctx, &session, system_prompt.clone(), None).await {
        Ok(runtime) => runtime,
        Err(error) => {
            return fail_backend_recovery_before_claimed_turn_resume(
                ctx,
                session_id,
                accepted_turn.as_ref(),
                error.to_string(),
            )
            .await;
        }
    };
    if let Some((accepted_turn, generation)) = accepted_turn {
        resume_claimed_turn_during_backend_recovery(
            ctx,
            &session,
            runtime,
            accepted_turn,
            generation,
            system_prompt,
            runtime_event_lock_held,
        )
        .await?;
    }
    Ok(())
}

fn reconcile_claimed_turn_after_backend_recovery_failure(
    ctx: &RuntimeContext,
    input: &QueuedTurnInput,
) {
    let (Some(operation_id), Some(obligation_id)) = (
        input.accepted_operation_id.clone(),
        input.execution_obligation_id.clone(),
    ) else {
        return;
    };
    let driver = ctx
        .accepted_send_obligation_driver
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    let Some(driver) = driver else {
        log::error!(
            "accepted backend recovery lost its obligation driver [{operation_id}/{obligation_id}]"
        );
        return;
    };
    let spawner = Arc::clone(&ctx.spawner);
    spawner.spawn(Box::pin(async move {
        if let Some(recovery_wake) = driver
            .reconcile_turn_execution(&operation_id, &obligation_id)
            .await
        {
            recovery_wake.publish();
        }
    }));
}

async fn fail_claimed_turn_backend_recovery(
    ctx: &RuntimeContext,
    session_id: &str,
    generation: u64,
    _input: &QueuedTurnInput,
    error: String,
) -> Result<(), AgentRuntimeError> {
    let still_current = {
        let sessions = ctx.sessions.lock().await;
        sessions
            .get(session_id)
            .is_some_and(|state| state.matches_generation(generation))
    };
    if !still_current {
        return Ok(());
    }
    persist_backend_session_recovery_failure(ctx, session_id, error).await
}

async fn fail_backend_recovery_before_claimed_turn_resume(
    ctx: &RuntimeContext,
    session_id: &str,
    accepted_turn: Option<&(QueuedTurnInput, u64)>,
    error: String,
) -> Result<(), AgentRuntimeError> {
    if let Some((input, generation)) = accepted_turn {
        return fail_claimed_turn_backend_recovery(ctx, session_id, *generation, input, error)
            .await;
    }
    persist_backend_session_recovery_failure(ctx, session_id, error).await
}

/// Continue the exact accepted execution on a replacement provider runtime.
///
/// `TurnExecution` was already claimed before the original runtime open, so
/// this path must neither enqueue the input nor repeat the durable claim. In
/// particular it submits the first input before waiting for
/// `SessionEstablished`; Claude reports that identity only after receiving
/// the input.
async fn resume_claimed_turn_during_backend_recovery(
    ctx: &RuntimeContext,
    session: &ChatSession,
    runtime: Arc<dyn AgentSessionRuntime>,
    input: QueuedTurnInput,
    generation: u64,
    system_prompt: Option<String>,
    runtime_event_lock_held: bool,
) -> Result<(), AgentRuntimeError> {
    let agent_message_id = match input.existing_agent_message_id.as_deref() {
        Some(agent_message_id) => agent_message_id,
        None => {
            return fail_claimed_turn_backend_recovery(
                ctx,
                &session.id,
                generation,
                &input,
                "accepted backend recovery has no assistant identity".to_string(),
            )
            .await;
        }
    };
    let restore_plan =
        match context_restore_plan_for_backend_recovery(ctx, &session.id, agent_message_id) {
            Ok(plan) => plan,
            Err(error) => {
                return fail_claimed_turn_backend_recovery(
                    ctx,
                    &session.id,
                    generation,
                    &input,
                    error,
                )
                .await;
            }
        };
    let context_was_reinjected = matches!(&restore_plan, ContextRestorePlan::Reinject { .. });
    if !turn_owns_runtime(ctx, &session.id, generation, &runtime).await {
        detach_runtime_if_current(ctx, &session.id, &runtime).await;
        return Ok(());
    }
    let prompt = apply_restore_prompt_prefix(input.content.clone(), &restore_plan);
    let start_result = runtime
        .start_turn(TurnInput {
            prompt,
            images: input
                .images
                .iter()
                .cloned()
                .map(|image| AttachmentPayload {
                    data: image.data,
                    media_type: image.media_type,
                })
                .collect(),
            system_prompt,
            permission_mode: input.permission_mode,
            plan_mode: input.plan_mode,
            permission_profile_id: input.permission_profile_id.clone(),
            editor_context: input.editor_context.clone().map(EditorContext::from),
        })
        .await;
    let _runtime_event_guard = if runtime_event_lock_held {
        None
    } else {
        Some(ctx.runtime_event_locks.acquire(&session.id).await)
    };
    if let Err(error) = start_result {
        if !turn_runtime_is_current(ctx, &session.id, generation, &runtime).await {
            return Ok(());
        }
        return fail_claimed_turn_backend_recovery(
            ctx,
            &session.id,
            generation,
            &input,
            error.to_string(),
        )
        .await;
    }
    if !turn_owns_runtime(ctx, &session.id, generation, &runtime).await {
        return Ok(());
    }
    spawn_stale_watchdog_task(
        ctx,
        session.id.clone(),
        generation,
        stale_timeout_for_session(session),
    );
    let recovery_id = {
        let mut sessions = ctx.sessions.lock().await;
        let Some(state) = sessions.get_mut(&session.id) else {
            return Ok(());
        };
        if !state.matches_generation(generation) {
            return Ok(());
        }
        let Some(recovery) = state.backend_recovery.as_mut() else {
            return Ok(());
        };
        if !recovery
            .attempt
            .accepted_turn_started(context_was_reinjected)
        {
            return Err(AgentRuntimeError::Other(format!(
                "backend recovery turn start is not applicable for {}",
                session.id
            )));
        }
        recovery.attempt.recovery_id().to_string()
    };
    retry_backend_session_recovery_completion(ctx, session.id.clone(), generation, recovery_id);
    let turn_id = {
        let sessions = ctx.sessions.lock().await;
        sessions
            .get(&session.id)
            .filter(|state| state.matches_generation(generation))
            .and_then(|state| state.active_turn_id())
    }
    .ok_or_else(|| {
        AgentRuntimeError::Other(format!(
            "accepted backend recovery lost its turn identity for {}",
            session.id
        ))
    })?;
    let (operation_id, obligation_id) = match (
        input.accepted_operation_id.clone(),
        input.execution_obligation_id.clone(),
    ) {
        (Some(operation_id), Some(obligation_id)) => (operation_id, obligation_id),
        _ => {
            return Err(AgentRuntimeError::Other(format!(
                "accepted backend recovery lost its operation identity for {}",
                session.id
            )));
        }
    };
    mark_accepted_turn_running_or_retry(
        ctx,
        &session.id,
        generation,
        operation_id,
        obligation_id,
        turn_id,
    );
    emit_session_state_change_from_session(
        session,
        &ctx.notifier,
        &ctx.status_center,
        &ctx.status_notifier,
        StateChange {
            turn_phase: TurnPhase::Streaming,
            queue_paused: None,
            pending_permission_request: None,
            pending_permission_state_revision: None,
            exit_code: None,
            completed_at: None,
            interrupted: false,
            session_state: Some(SessionState::Active),
        },
    );
    Ok(())
}

async fn claim_backend_session_recovery_completion(
    ctx: &RuntimeContext,
    session_id: &str,
    generation: u64,
    recovery_id: &str,
) -> Option<BackendRecoveryCompletion> {
    let mut sessions = ctx.sessions.lock().await;
    let state = sessions
        .get_mut(session_id)
        .filter(|state| state.matches_generation(generation))?;
    let recovery = state
        .backend_recovery
        .as_mut()
        .filter(|recovery| recovery.attempt.matches_recovery_id(recovery_id))?;
    recovery.attempt.claim_completion()
}

async fn persist_backend_session_recovery_completion(
    ctx: &RuntimeContext,
    session_id: &str,
    generation: u64,
    completion_input: &BackendRecoveryCompletion,
) -> Result<bool, AgentRuntimeError> {
    let mut meta = ctx
        .session_store
        .complete_backend_session_recovery(
            &ctx.data_dir,
            session_id,
            &completion_input.recovery_id,
            completion_input.old_provider_session_generation,
            completion_input.backend_session_id.clone(),
        )
        .map_err(AgentRuntimeError::Other)?;
    if let Some(context_was_reinjected) = completion_input.context_was_reinjected {
        if let Some(updated) = ctx
            .session_store
            .complete_context_reinjection_if_required(
                &ctx.data_dir,
                session_id,
                meta.provider_session_generation,
                context_was_reinjected,
            )
            .map_err(AgentRuntimeError::Other)?
        {
            meta = updated;
        }
    }
    let completion = {
        let _runtime_event_guard = ctx.runtime_event_locks.acquire(session_id).await;
        let mut sessions = ctx.sessions.lock().await;
        let state = sessions
            .get_mut(session_id)
            .filter(|state| state.matches_generation(generation));
        state.and_then(|state| {
            let owns_exact_recovery = state
                .backend_recovery
                .as_ref()
                .is_some_and(|recovery| {
                    recovery
                        .attempt
                        .owns_completion(&completion_input.recovery_id)
                });
            owns_exact_recovery
                .then(|| state.backend_recovery.take())
                .flatten()
                .map(|recovery| recovery.completion)
        })
    };
    let Some(completion) = completion else {
        return Ok(false);
    };
    let _ = completion.send(true);
    let notifier = Arc::clone(&ctx.notifier);
    let notification_session_id = session_id.to_string();
    let notification_spawner = Arc::clone(&ctx.spawner);
    notification_spawner.spawn(Box::pin(async move {
        notifier.context_carry_updated(
            &notification_session_id,
            meta.agent_session_id,
            meta.context_carry,
            meta.updated_at,
        );
    }));
    log::debug!(
        "completed backend session recovery for {session_id} ({:?}, recovery_id={})",
        completion_input.reason,
        completion_input.recovery_id
    );
    reconcile_pending_recovery_message_detached(
        ctx,
        session_id.to_string(),
        "backend recovery notice",
    );
    Ok(true)
}

fn retry_backend_session_recovery_completion(
    ctx: &RuntimeContext,
    session_id: String,
    generation: u64,
    recovery_id: String,
) {
    let ctx = ctx.clone();
    let spawner = Arc::clone(&ctx.spawner);
    spawner.spawn(Box::pin(async move {
        let Some(completion_input) =
            claim_backend_session_recovery_completion(&ctx, &session_id, generation, &recovery_id)
                .await
        else {
            return;
        };
        let mut retry_delay = Duration::from_millis(25);
        loop {
            let still_current = {
                let sessions = ctx.sessions.lock().await;
                sessions.get(&session_id).is_some_and(|state| {
                    state.matches_generation(generation)
                        && state.backend_recovery.as_ref().is_some_and(|recovery| {
                            recovery.attempt.owns_completion(&recovery_id)
                        })
                })
            };
            if !still_current {
                return;
            }
            match persist_backend_session_recovery_completion(
                &ctx,
                &session_id,
                generation,
                &completion_input,
            )
            .await
            {
                Ok(true) => {
                    let _session_guard = ctx.session_locks.acquire(&session_id).await;
                    start_next_queued_turn(&ctx, &session_id).await;
                    return;
                }
                Ok(false) => return,
                Err(error) => {
                    log::warn!(
                        "backend recovery completion remains pending for {session_id}: {error}"
                    );
                }
            }
            tokio::time::sleep(retry_delay).await;
            retry_delay = next_recovery_retry_delay(retry_delay);
        }
    }));
}

async fn persist_backend_session_recovery_failure(
    ctx: &RuntimeContext,
    session_id: &str,
    error: String,
) -> Result<(), AgentRuntimeError> {
    if schedule_backend_session_recovery_failure(ctx, session_id, error).await? {
        Ok(())
    } else {
        Err(AgentRuntimeError::Other(format!(
            "backend recovery completion is already settling for {session_id}"
        )))
    }
}

async fn schedule_backend_session_recovery_failure(
    ctx: &RuntimeContext,
    session_id: &str,
    error: String,
) -> Result<bool, AgentRuntimeError> {
    let (recovery_id, claim) = {
        let mut sessions = ctx.sessions.lock().await;
        let Some(recovery) = sessions
            .get_mut(session_id)
            .and_then(|state| state.backend_recovery.as_mut())
        else {
            return Err(AgentRuntimeError::Other(format!(
                "cannot durably fail backend recovery without an active recovery identity for {session_id}"
            )));
        };
        (
            recovery.attempt.recovery_id().to_string(),
            recovery.attempt.claim_failure(&error),
        )
    };
    match claim {
        BackendRecoveryFailureClaim::Rejected => return Ok(false),
        BackendRecoveryFailureClaim::Joined => return Ok(true),
        BackendRecoveryFailureClaim::Claimed => {}
    }
    retry_backend_session_recovery_failure(ctx, session_id.to_string(), recovery_id, error);
    Ok(true)
}

fn retry_backend_session_recovery_failure(
    ctx: &RuntimeContext,
    session_id: String,
    recovery_id: String,
    error: String,
) {
    let ctx = ctx.clone();
    let spawner = Arc::clone(&ctx.spawner);
    spawner.spawn(Box::pin(async move {
        let mut retry_delay = Duration::from_millis(25);
        loop {
            let still_current = {
                let sessions = ctx.sessions.lock().await;
                sessions.get(&session_id).is_some_and(|state| {
                    state.backend_recovery.as_ref().is_some_and(|recovery| {
                        recovery.attempt.owns_failure(&recovery_id, &error)
                    })
                })
            };
            if !still_current {
                return;
            }
            match ctx.session_store.fail_backend_session_recovery(
                &ctx.data_dir,
                &session_id,
                &recovery_id,
                &error,
            ) {
                Ok(_) => break,
                Err(persist_error) => {
                    log::warn!(
                        "backend recovery failure remains pending for {session_id}: {persist_error}"
                    );
                }
            }
            tokio::time::sleep(retry_delay).await;
            retry_delay = next_recovery_retry_delay(retry_delay);
        }

        let settled = {
            let _runtime_event_guard = ctx.runtime_event_locks.acquire(&session_id).await;
            let mut sessions = ctx.sessions.lock().await;
            sessions.get_mut(&session_id).and_then(|state| {
                let owns_exact_recovery =
                    state.backend_recovery.as_ref().is_some_and(|recovery| {
                        recovery.attempt.owns_failure(&recovery_id, &error)
                    });
                if !owns_exact_recovery {
                    return None;
                }
                let accepted_turn = state.current_turn_input.clone().filter(|input| {
                    accepted_effect_has_durable_execution_identity(
                        input.accepted_operation_id.as_deref(),
                        input.execution_obligation_id.as_deref(),
                    )
                });
                state.rollback_started_turn();
                state.bump_runtime_epoch();
                let runtime = state.runtime.take();
                let completion = state
                    .backend_recovery
                    .take()
                    .map(|recovery| recovery.completion)?;
                Some((runtime, completion, accepted_turn))
            })
        };
        let Some((runtime, completion, accepted_turn)) = settled else {
            return;
        };
        let _ = completion.send(true);
        if let Some(runtime) = runtime {
            let close_spawner = Arc::clone(&ctx.spawner);
            close_spawner.spawn(Box::pin(async move {
                runtime.close().await;
            }));
        }
        if let Some(input) = accepted_turn.as_ref() {
            reconcile_claimed_turn_after_backend_recovery_failure(&ctx, input);
        }
        emit_session_state_change(
            &ctx.session_store,
            &ctx.notifier,
            &ctx.status_center,
            &ctx.status_notifier,
            &ctx.data_dir,
            &session_id,
            StateChange {
                turn_phase: TurnPhase::Idle,
                queue_paused: None,
                pending_permission_request: None,
                pending_permission_state_revision: None,
                exit_code: Some(1),
                completed_at: Some(crate::usecase::agent_session::session::now_timestamp()),
                interrupted: true,
                session_state: Some(SessionState::Error),
            },
        );
        reconcile_pending_recovery_message_detached(&ctx, session_id, "backend recovery error");
    }));
}

async fn wait_for_backend_session_recovery(ctx: &RuntimeContext, session_id: &str) {
    let receiver = {
        let sessions = ctx.sessions.lock().await;
        sessions
            .get(session_id)
            .and_then(|state| state.backend_recovery.as_ref())
            .map(|recovery| recovery.completion.subscribe())
    };
    let Some(mut receiver) = receiver else {
        return;
    };
    while !*receiver.borrow() {
        if receiver.changed().await.is_err() {
            break;
        }
    }
}

pub(super) async fn acquire_session_control_after_recovery(
    ctx: &RuntimeContext,
    session_id: &str,
) -> SessionRuntimeLockGuard {
    loop {
        wait_for_backend_session_recovery(ctx, session_id).await;
        let guard = ctx.session_locks.acquire(session_id).await;
        let recovery_started = {
            let sessions = ctx.sessions.lock().await;
            sessions
                .get(session_id)
                .is_some_and(|state| state.backend_recovery.is_some())
        };
        if !recovery_started {
            reconcile_incomplete_backend_recovery(ctx, session_id).await;
            return guard;
        }
        drop(guard);
    }
}

fn backend_recovery_projection(
    ctx: &RuntimeContext,
    session_id: &str,
) -> Result<DurableBackendRecovery, AgentRuntimeError> {
    let events = ctx
        .session_store
        .load_session_events(&ctx.data_dir, session_id)
        .map_err(AgentRuntimeError::Other)?;
    Ok(project_durable_backend_recovery(&events))
}

pub(super) fn ensure_backend_recovery_operation_allowed(
    ctx: &RuntimeContext,
    session_id: &str,
) -> Result<(), AgentRuntimeError> {
    let meta = ctx
        .session_store
        .get_session_meta(&ctx.data_dir, session_id)
        .map_err(AgentRuntimeError::Other)?;
    let pending_publication_recovery_id = meta
        .as_ref()
        .and_then(|meta| meta.pending_recovery_message.as_ref())
        .map(|pending| match pending {
            PendingRecoveryMessage::Notice { recovery_id, .. }
            | PendingRecoveryMessage::Error { recovery_id, .. } => recovery_id,
        });
    let recovery_may_be_incomplete = meta.as_ref().is_some_and(|meta| {
        backend_recovery_may_be_incomplete(
            meta.agent_session_id.is_some(),
            meta.context_carry.is_some_and(ContextCarryState::is_failed),
            meta.context_reinjection_generation.is_some(),
        )
    });
    let recovery = if recovery_may_be_incomplete {
        backend_recovery_projection(ctx, session_id)?
    } else {
        DurableBackendRecovery::None
    };
    admit_backend_recovery_sensitive_operation(
        pending_publication_recovery_id.map(String::as_str),
        recovery_may_be_incomplete,
        &recovery,
    )
    .map_err(|rejection| match rejection {
        BackendRecoveryOperationRejection::PublicationPending { recovery_id } => {
            AgentRuntimeError::Other(format!(
                "backend session recovery publication {recovery_id} is still pending"
            ))
        }
        BackendRecoveryOperationRejection::Recovering { recovery_id } => {
            AgentRuntimeError::Other(format!(
                "backend session recovery {recovery_id} is still in progress"
            ))
        }
        BackendRecoveryOperationRejection::ReconciliationRequired { recovery_id, error } => {
            AgentRuntimeError::Other(format!(
                "backend session recovery {recovery_id} requires reconciliation: {error}"
            ))
        }
    })
}

async fn reconcile_incomplete_backend_recovery(ctx: &RuntimeContext, session_id: &str) {
    if let Err(error) = reconcile_pending_recovery_message(ctx, session_id).await {
        log::warn!(
            "failed to reconcile pending backend recovery message for {session_id}: {error}"
        );
    }
    let recovery_may_be_incomplete = match ctx
        .session_store
        .get_session_meta(&ctx.data_dir, session_id)
    {
        Ok(meta) => meta.is_some_and(|meta| {
            backend_recovery_may_be_incomplete(
                meta.agent_session_id.is_some(),
                meta.context_carry.is_some_and(ContextCarryState::is_failed),
                meta.context_reinjection_generation.is_some(),
            )
        }),
        Err(error) => {
            log::warn!("failed to load backend recovery metadata for {session_id}: {error}");
            return;
        }
    };
    if !recovery_may_be_incomplete {
        return;
    }
    let projection = match backend_recovery_projection(ctx, session_id) {
        Ok(projection) => projection,
        Err(error) => {
            log::warn!("failed to restore backend recovery state for {session_id}: {error}");
            return;
        }
    };
    let DurableBackendRecovery::Recovering { recovery_id, .. } = projection else {
        return;
    };
    let error = "backend session recovery was interrupted before completion";
    if let Err(persist_error) = ctx.session_store.fail_backend_session_recovery(
        &ctx.data_dir,
        session_id,
        &recovery_id,
        error,
    ) {
        log::warn!(
            "failed to persist interrupted backend recovery for {session_id}: {persist_error}"
        );
        return;
    }
    if let Err(persist_error) = reconcile_pending_recovery_message(ctx, session_id).await {
        log::warn!(
            "failed to publish interrupted backend recovery for {session_id}: {persist_error}"
        );
    }
}
