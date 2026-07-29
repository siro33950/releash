async fn complete_turn_with_acceptance(
    ctx: &RuntimeContext,
    session_id: &str,
    expected_generation: Option<u64>,
    result: crate::domain::agent_session::entities::TurnResult,
) -> Result<(Option<WorkflowTurnCompleteNotification>, bool), String> {
    complete_turn_with_acceptance_and_persist_kind(
        ctx,
        session_id,
        expected_generation,
        result,
        PersistFailureKind::FinalPartsRecorded,
    )
    .await
}

async fn complete_turn_with_acceptance_and_persist_kind(
    ctx: &RuntimeContext,
    session_id: &str,
    expected_generation: Option<u64>,
    result: crate::domain::agent_session::entities::TurnResult,
    persist_kind: PersistFailureKind,
) -> Result<(Option<WorkflowTurnCompleteNotification>, bool), String> {
    let _queue_transition_guard = ctx.transitions.acquire(session_id).await;
    let interrupt_was_accepted = {
        let sessions = ctx.sessions.lock().await;
        sessions
            .get(session_id)
            .is_some_and(|state| {
                state.interrupt_requested_for_optional_generation(expected_generation)
            })
    };
    let emit_crash_snapshot = result.requires_crash_snapshot();
    let process_turn_id = {
        let sessions = ctx.sessions.lock().await;
        sessions.get(session_id).and_then(|state| {
            state
                .owns_optional_generation(expected_generation)
                .then(|| state.active_turn_id())
                .flatten()
        })
    };
    let Some(process_turn_id) = process_turn_id else {
        log::debug!(
            "skipping turn completion for {session_id}: turn already completed or generation mismatch (expected={expected_generation:?})"
        );
        return Ok((None, false));
    };
    let lifecycle_repository = ctx.lifecycle_repository();
    if lifecycle_repository.is_none() {
        #[cfg(not(test))]
        return Err("agent-session lifecycle repository is not configured".to_string());
    }
    let canonical_terminal = if let Some(repository) = lifecycle_repository {
        let mut session = repository
            .restore_session(session_id)
            .await
            .map_err(|error| format!("failed to restore terminal session aggregate: {error:?}"))?;
        if !session.owns_active_turn(process_turn_id) {
            return Ok((None, false));
        }
        let decision = session.apply_terminal(process_turn_id, result.clone());
        match decision.application {
            TerminalApplication::Current => Some((decision.pause_queue, session.state())),
            TerminalApplication::AlreadyApplied | TerminalApplication::Superseded => {
                return Ok((None, false));
            }
        }
    } else {
        None
    };
    flush_streaming_update(ctx, session_id, true).await?;
    let completed_at = crate::usecase::agent_session::session::now_timestamp();
    let mut terminal = ctx.projection_gateway.terminal_projection(
        &result,
        crate::domain::agent_session::aggregates::session::Session::terminal_outcome(&result),
    );
    if let Some((pause_queue, session_state)) = canonical_terminal {
        terminal.pause_queue = pause_queue;
        terminal.session_state = session_state;
    }
    let (message_id, parts, seq, turn_id, started_at, queue_was_paused_at) = {
        let sessions = ctx.sessions.lock().await;
        let Some(state) = sessions.get(session_id) else {
            return Ok((None, false));
        };
        if !state.owns_optional_generation(expected_generation) {
            return Ok((None, false));
        }
        (
            state.streaming_message_id.clone(),
            state.persisted_streaming_parts().to_vec(),
            state.stream_sequence(),
            state.active_turn_id(),
            state.turn_started_at(),
            state.queue_paused_at(),
        )
    };
    let terminal_identity = require_terminal_commit_identity(turn_id, message_id.clone()).map_err(
        |_| {
            format!(
                "cannot commit a terminal result without the durable turn and assistant-message identity for {session_id}"
            )
        },
    )?;
    let mut crash_snapshot = None;
    let projected = {
        let turn_id = terminal_identity.turn_id;
        let message_id = terminal_identity.message_id;
        let final_seq = result.terminal_stream_sequence(seq);
        let events = final_turn_events(
            ctx,
            session_id,
            turn_id,
            &message_id,
            &parts,
            &terminal,
            completed_at,
        )?;
        let (model, persisted_parts) = persist_with_retry(ctx, session_id, persist_kind, || {
            ctx.session_store.append_terminal_events_and_materialize(
                &ctx.data_dir,
                session_id,
                &events,
                &message_id,
                final_seq,
                completed_at,
                &result,
            )
        })
        .await?;
        {
            let mut sessions = ctx.sessions.lock().await;
            if let Some(state) = sessions.get_mut(session_id) {
                if state.owns_active_turn_id(turn_id) {
                    let _ = state.mark_terminal(turn_id);
                }
            }
        }
        if emit_crash_snapshot {
            crash_snapshot = Some(PendingStreamDelta {
                message_id,
                seq: final_seq,
                snapshot: true,
                parts: persisted_parts,
                message: None,
                authoritative: true,
            });
        }
        model
    };
    if let Some(snapshot) = crash_snapshot {
        emit_streaming_delta_or_retry(ctx, session_id, snapshot).await;
    }
    let session_state = Some(projected.status.session_state);
    let queue_paused_at = projected
        .queue_paused_at
        .or(queue_was_paused_at)
        .or_else(|| terminal.pause_queue.then_some(completed_at));
    let queue_paused = queue_paused_at.is_some();
    let pending_permission_state_revision = {
        let mut sessions = ctx.sessions.lock().await;
        let Some(state) = sessions.get_mut(session_id) else {
            return Ok((None, false));
        };
        if !state.owns_optional_generation(expected_generation) {
            return Ok((None, false));
        }
        state.release_turn_lease();
        state.replace_queue_pause(queue_paused_at);
        let pending_permission_state_revision = state.clear_pending_permission_request();
        state.clear_stall_observation();
        state.last_agent_message_id = message_id;
        let usage = result
            .token_usage()
            .map(|usage| ctx.projection_gateway.token_usage(usage));
        if let Some(usage) = usage {
            state.latest_token_usage = Some(usage);
        }
        state.finish_turn_progress();
        state.streaming_message_id = None;
        state.current_turn_input = None;
        state.clear_interrupt_request();
        state.reset_stream_buffer();
        state.reset_stream_sequence();
        state.finish_stream_delivery();
        pending_permission_state_revision
    };
    if let Some(started_at) = started_at {
        record_agent_turn_duration_detached(
            ctx,
            session_id.to_string(),
            crate::other::telemetry::AgentTurn::Complete,
            started_at.elapsed(),
        );
    }
    let workflow_notification = projected
        .workflow_turn_complete
        .as_ref()
        .map(|input| {
            ctx.projection_gateway
                .workflow_turn_complete(session_id, input)
        });
    emit_session_state_change(
        &ctx.session_store,
        &ctx.notifier,
        &ctx.status_center,
        &ctx.status_notifier,
        &ctx.data_dir,
        session_id,
        StateChange {
            turn_phase: TurnPhase::Idle,
            queue_paused: Some(queue_paused),
            pending_permission_request: None,
            pending_permission_state_revision: Some(pending_permission_state_revision),
            exit_code: Some(terminal.exit_code),
            completed_at: Some(completed_at),
            interrupted: terminal.interrupted,
            session_state,
        },
    );
    Ok((workflow_notification, interrupt_was_accepted))
}

async fn turn_owns_runtime(
    ctx: &RuntimeContext,
    session_id: &str,
    generation: u64,
    runtime: &Arc<dyn AgentSessionRuntime>,
) -> bool {
    let sessions = ctx.sessions.lock().await;
    sessions.get(session_id).is_some_and(|state| {
        state.admits_provider_effect(generation)
            && state
                .runtime
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, runtime))
    })
}

async fn turn_runtime_is_current(
    ctx: &RuntimeContext,
    session_id: &str,
    generation: u64,
    runtime: &Arc<dyn AgentSessionRuntime>,
) -> bool {
    let sessions = ctx.sessions.lock().await;
    sessions.get(session_id).is_some_and(|state| {
        state.owns_generation(generation)
            && state
                .runtime
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, runtime))
    })
}

async fn detach_runtime_if_current(
    ctx: &RuntimeContext,
    session_id: &str,
    runtime: &Arc<dyn AgentSessionRuntime>,
) {
    let detached = {
        let mut sessions = ctx.sessions.lock().await;
        let Some(state) = sessions.get_mut(session_id) else {
            return;
        };
        if !state
            .runtime
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, runtime))
        {
            return;
        }
        let detached = state.runtime.take();
        state.bump_runtime_epoch();
        detached
    };
    if let Some(runtime) = detached {
        let spawner = Arc::clone(&ctx.spawner);
        spawner.spawn(Box::pin(async move {
            runtime.close().await;
        }));
    }
}

fn accepted_queued_effect_identity(
    queued: &QueuedTurnInput,
) -> AcceptedQueuedEffectIdentity<'_> {
    AcceptedQueuedEffectIdentity {
        queue_item_id: &queued.id,
        human_message_id: queued.existing_human_message_id.as_deref(),
        reserved_turn_id: queued.reserved_turn_id,
        operation_id: queued.accepted_operation_id.as_deref(),
        obligation_id: queued.execution_obligation_id.as_deref(),
    }
}

fn queued_turn_has_accepted_identity(queued: &QueuedTurnInput) -> bool {
    accepted_queued_effect_has_durable_identity(accepted_queued_effect_identity(queued))
}

fn canonical_queued_effect_identity(
    queued: &CanonicalQueuedSend,
) -> CanonicalQueuedEffectIdentity<'_> {
    CanonicalQueuedEffectIdentity {
        queue_item_id: &queued.queue_item_id,
        human_message_id: &queued.human_message_id,
        reserved_turn_id: &queued.reserved_turn_id,
    }
}

fn cache_accepted_input_effect(
    accepted_input_effects: &mut std::collections::HashMap<String, QueuedTurnInput>,
    accepted_input: QueuedTurnInput,
    canonical_queue: &[CanonicalQueuedSend],
) -> Result<(), String> {
    canonical_queue
        .iter()
        .find(|entry| {
            accepted_queued_effect_matches(
                accepted_queued_effect_identity(&accepted_input),
                canonical_queued_effect_identity(entry),
            )
        })
        .ok_or_else(|| {
            "accepted queued send is absent from the canonical queue projection".to_string()
        })?;

    if let Some(existing) = accepted_input_effects.get(&accepted_input.id) {
        if !accepted_queued_effect_identity_is_consistent(
            accepted_queued_effect_identity(existing),
            accepted_queued_effect_identity(&accepted_input),
        ) {
            return Err("accepted queue identity changed during restoration".to_string());
        }
    }

    accepted_input_effects.retain(|_, queued| {
        accepted_queued_effect_should_retain(
            accepted_queued_effect_identity(queued),
            canonical_queue
                .iter()
                .map(canonical_queued_effect_identity),
        )
    });
    accepted_input_effects.insert(accepted_input.id.clone(), accepted_input);
    Ok(())
}

async fn remove_local_queue_front_if_matches(
    ctx: &RuntimeContext,
    session_id: &str,
    queue_item_id: &str,
) {
    let mut sessions = ctx.sessions.lock().await;
    let Some(state) = sessions.get_mut(session_id) else {
        return;
    };
    state.accepted_input_effects.remove(queue_item_id);
}

async fn arm_accepted_send_recovery_after_claim_release(
    driver: &dyn AcceptedSendObligationDriver,
    operation_id: &str,
    obligation_id: &str,
    accepted_claim: &mut Option<AcceptedSendExecutionClaim>,
) {
    let Some(recovery_wake) = driver
        .reconcile_turn_execution(operation_id, obligation_id)
        .await
    else {
        return;
    };
    match accepted_claim.take() {
        Some(claim) => {
            *accepted_claim = Some(claim.wake_after_release(recovery_wake));
        }
        None => recovery_wake.publish(),
    }
}

fn next_cached_input_effect<'a>(
    accepted_input_effects: &'a std::collections::HashMap<String, QueuedTurnInput>,
    canonical_queue: &[CanonicalQueuedSend],
) -> Option<&'a QueuedTurnInput> {
    if let Some(head) = canonical_queue.first() {
        return accepted_input_effects
            .get(&head.queue_item_id)
            .filter(|queued| {
                accepted_queued_effect_matches(
                    accepted_queued_effect_identity(queued),
                    canonical_queued_effect_identity(head),
                )
            });
    }
    #[cfg(test)]
    {
        return accepted_input_effects.values().min_by(|left, right| {
            left.created_at
                .total_cmp(&right.created_at)
                .then_with(|| left.id.cmp(&right.id))
        });
    }
    #[cfg(not(test))]
    None
}

pub(super) async fn start_next_queued_turn(ctx: &RuntimeContext, session_id: &str) {
    if let Err(failure) = ctx
        .session_store
        .ensure_no_unresolved_recovery(session_id)
        .await
    {
        log::warn!(
            "queued turn drain blocked by unresolved recovery {} for {session_id}: {failure}",
            failure.correlation_id
        );
        return;
    }
    let canonical_queue = match ctx.session_store.canonical_pending_send_queue(session_id) {
        Ok(queue) => queue,
        Err(error) => {
            #[cfg(not(test))]
            {
                log::warn!("accepted queue authority is unavailable for {session_id}: {error}");
                return;
            }
            #[cfg(test)]
            {
                log::debug!(
                    "legacy test queue has no canonical projection for {session_id}: {error}"
                );
                Vec::new()
            }
        }
    };
    let (queued, runtime) = {
        let sessions = ctx.sessions.lock().await;
        let Some(state) = sessions.get(session_id) else {
            return;
        };
        if !state.admits_queue_drain() {
            return;
        }
        let Some(queued) =
            next_cached_input_effect(&state.accepted_input_effects, &canonical_queue).cloned()
        else {
            return;
        };
        let runtime = state.runtime.clone();
        (queued, runtime)
    };

    match decide_accepted_queued_effect_queue(
        accepted_queued_effect_identity(&queued),
        canonical_queue.iter().map(canonical_queued_effect_identity),
    ) {
        AcceptedQueuedEffectQueueDecision::Start => {}
        AcceptedQueuedEffectQueueDecision::AwaitCanonicalFront => return,
        AcceptedQueuedEffectQueueDecision::DiscardStale => {
            remove_local_queue_front_if_matches(ctx, session_id, &queued.id).await;
            return;
        }
    }

    let Some(session) = (match ctx
        .session_store
        .get_session_shell(&ctx.data_dir, session_id)
    {
        Ok(session) => session,
        Err(error) => {
            log::warn!("failed to load queued turn session {session_id}: {error}");
            return;
        }
    }) else {
        log::warn!("queued turn session not found: {session_id}");
        return;
    };
    if accepted_queued_effect_has_durable_identity(accepted_queued_effect_identity(&queued)) {
        match ctx
            .session_store
            .accepted_queue_start_readiness(&ctx.data_dir, session_id)
        {
            Ok(Some(true)) => {}
            Ok(Some(false)) => return,
            Ok(None) => {
                log::warn!("accepted queue session projection not found: {session_id}");
                return;
            }
            Err(error) => {
                log::warn!("accepted queue readiness is unavailable for {session_id}: {error}");
                return;
            }
        }
    }
    let mut accepted_claim;
    let accepted_obligation = match (
        queued.accepted_operation_id.as_deref(),
        queued.execution_obligation_id.as_deref(),
    ) {
        (Some(operation_id), Some(obligation_id)) => {
            let driver = ctx
                .accepted_send_obligation_driver
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone();
            let Some(driver) = driver else {
                log::warn!("accepted queued send driver is unavailable [{operation_id}]");
                return;
            };
            accepted_claim = None;
            Some((operation_id.to_string(), obligation_id.to_string(), driver))
        }
        (None, None) => {
            #[cfg(test)]
            {
                accepted_claim = None;
                None
            }
            #[cfg(not(test))]
            {
                log::error!(
                    "queued turn has no durable accepted operation identity for {session_id}"
                );
                return;
            }
        }
        _ => {
            log::error!("accepted queued send has incomplete obligation identity");
            return;
        }
    };
    if !accepted_worktree_matches(&queued.worktree_path, &session.worktree_path) {
        if let Some((operation_id, obligation_id, driver)) = &accepted_obligation {
            arm_accepted_send_recovery_after_claim_release(
                driver.as_ref(),
                operation_id,
                obligation_id,
                &mut accepted_claim,
            )
            .await;
        }
        log::error!(
            "queued turn worktree mismatch for {session_id}: queued={}, session={}",
            queued.worktree_path,
            session.worktree_path
        );
        return;
    }
    let had_runtime = runtime.is_some();
    let system_prompt = match build_queued_system_prompt(
        &ctx.session_store,
        ctx.branch_diff_context.as_deref(),
        ctx.instruction_source.as_ref(),
        &ctx.data_dir,
        &session,
        &queued,
    ) {
        Ok(system_prompt) => system_prompt,
        Err(error) => {
            log::warn!(
                "queued turn system prompt preflight remains pending for {session_id}: {error}"
            );
            return;
        }
    };
    #[cfg(test)]
    let mut runtime = runtime;
    #[cfg(not(test))]
    let runtime = runtime;
    #[cfg(test)]
    if accepted_obligation.is_none() && runtime.is_none() {
        // Legacy direct-send queues exist only in unit tests. Preserve their
        // historical pre-TurnStarted reopen boundary so the fault-injection
        // tests continue to exercise a retryable queue item. Production
        // accepted queues must cross the combined durable claim first.
        let runtime_open_epoch = {
            let mut sessions = ctx.sessions.lock().await;
            let Some(state) = sessions.get_mut(session_id) else {
                return;
            };
            state.bump_runtime_epoch()
        };
        runtime = match open_runtime_for_session(
            ctx,
            &session,
            system_prompt.clone(),
            Some(runtime_open_epoch),
        )
        .await
        {
            Ok(runtime) => Some(runtime),
            Err(AgentRuntimeError::BackendSessionLost { .. }) => {
                if let Err(error) = recover_backend_session(
                    ctx,
                    session_id,
                    BackendSessionRecoveryReason::BackendSessionLost,
                )
                .await
                {
                    log::warn!(
                        "failed to recover backend session for queued turn {session_id}: {error}"
                    );
                }
                return;
            }
            Err(error) => {
                log::warn!("failed to reopen runtime for queued turn {session_id}: {error}");
                if let Err(persist_error) =
                    persist_with_retry(ctx, session_id, PersistFailureKind::ReopenRuntime, || {
                        ctx.session_store.set_session_state(
                            &ctx.data_dir,
                            session_id,
                            SessionState::Error,
                        )
                    })
                    .await
                {
                    log::error!(
                        "failed to persist queued runtime reopen error for {session_id}: {persist_error}"
                    );
                    return;
                }
                emit_session_state_change(
                    &ctx.session_store,
                    &ctx.notifier,
                    &ctx.status_center,
                    &ctx.status_notifier,
                    &ctx.data_dir,
                    session_id,
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
                return;
            }
        };
    }
    let turn_id = match (queued.reserved_turn_id, accepted_obligation.is_some()) {
        (Some(turn_id), _) => turn_id,
        (None, true) => {
            if let Some((operation_id, obligation_id, driver)) = &accepted_obligation {
                arm_accepted_send_recovery_after_claim_release(
                    driver.as_ref(),
                    operation_id,
                    obligation_id,
                    &mut accepted_claim,
                )
                .await;
            }
            log::error!("accepted queued send has no reserved turn identity");
            return;
        }
        (None, false) => match next_turn_id(&ctx.session_store, &ctx.data_dir, session_id) {
            Ok(turn_id) => turn_id,
            Err(error) => {
                log::warn!("failed to allocate queued turn id for {session_id}: {error}");
                return;
            }
        },
    };
    let queue_item_is_current = {
        let sessions = ctx.sessions.lock().await;
        sessions.get(session_id).is_some_and(|state| {
            state.accepted_input_effects.contains_key(&queued.id)
        })
    };
    if !queue_item_is_current {
        log::warn!("accepted queued send preflight lost its exact in-memory queue identity");
        return;
    }

    let human_message_id = queued
        .existing_human_message_id
        .as_deref()
        .unwrap_or(queued.id.as_str());
    let human_message = match committed_queued_message(
        &ctx.session_store,
        &ctx.data_dir,
        session_id,
        human_message_id,
        MessageRole::Human,
    ) {
        Ok(Some(message)) => message,
        Ok(None) => {
            #[cfg(test)]
            if accepted_obligation.is_none() {
                queued_human_message(&queued)
            } else {
                if let Some((operation_id, obligation_id, driver)) = &accepted_obligation {
                    arm_accepted_send_recovery_after_claim_release(
                        driver.as_ref(),
                        operation_id,
                        obligation_id,
                        &mut accepted_claim,
                    )
                    .await;
                }
                log::error!(
                    "accepted queued send has no committed human projection [{human_message_id}]"
                );
                return;
            }
            #[cfg(not(test))]
            {
                if let Some((operation_id, obligation_id, driver)) = &accepted_obligation {
                    arm_accepted_send_recovery_after_claim_release(
                        driver.as_ref(),
                        operation_id,
                        obligation_id,
                        &mut accepted_claim,
                    )
                    .await;
                }
                log::error!(
                    "accepted queued send has no committed human projection [{human_message_id}]"
                );
                return;
            }
        }
        Err(error) => {
            log::warn!(
                "accepted queued human projection remains unreadable for {session_id}: {error}"
            );
            return;
        }
    };
    let durably_accepted = accepted_obligation.is_some();
    let committed_prompt =
        crate::usecase::agent_session::event_log::prompt_input_from_human_message(&human_message);
    let accepted_payload_matches = accepted_prompt_matches(
        &committed_prompt,
        &crate::domain::agent_session::events::PromptInput {
                content: queued.content.clone(),
                mentions: queued.mentions.clone(),
                attachment_refs: Vec::new(),
                parts: queued
                    .images
                    .iter()
                    .map(|image| MessagePart::Image {
                        data: image.data.clone(),
                        media_type: image.media_type.clone(),
                    })
                    .collect(),
            },
    );
    if durably_accepted && !accepted_payload_matches {
        if let Some((operation_id, obligation_id, driver)) = &accepted_obligation {
            arm_accepted_send_recovery_after_claim_release(
                driver.as_ref(),
                operation_id,
                obligation_id,
                &mut accepted_claim,
            )
            .await;
        }
        log::error!("accepted queued human projection does not match its canonical payload");
        return;
    }
    let (agent_message_id, legacy_agent_message) = if durably_accepted {
        let Some(message_id) = queued.existing_agent_message_id.clone() else {
            if let Some((operation_id, obligation_id, driver)) = &accepted_obligation {
                arm_accepted_send_recovery_after_claim_release(
                    driver.as_ref(),
                    operation_id,
                    obligation_id,
                    &mut accepted_claim,
                )
                .await;
            }
            log::error!("accepted queued send has no reserved assistant identity");
            return;
        };
        (message_id, None)
    } else {
        #[cfg(test)]
        {
            // Legacy direct-send queues exist only in unit tests. Keep their
            // assistant append as a separate fault boundary for retry oracles.
            let agent_message = match queued_agent_message(
                &ctx.session_store,
                &ctx.data_dir,
                session_id,
                &queued,
            ) {
                Ok(message) => message,
                Err(error) => {
                    log::warn!("failed to append queued agent message for {session_id}: {error}");
                    return;
                }
            };
            (agent_message.id.clone(), Some(agent_message))
        }
        #[cfg(not(test))]
        {
            log::error!("queued turn reached the legacy test path in production");
            return;
        }
    };
    let agent_message = if durably_accepted {
        queued_agent_projection(agent_message_id.clone(), human_message.timestamp)
    } else {
        legacy_agent_message.expect("legacy test queue path must materialize its assistant")
    };
    let restore_policy = match context_restore_policy_before_human_message(
        ctx,
        session_id,
        &human_message.id,
        had_runtime,
    ) {
        Ok(policy) => policy,
        Err(error) => {
            log::warn!(
                "queued turn restore preflight remains pending for {session_id}; provider start was not claimed: {error}"
            );
            return;
        }
    };
    let context_was_reinjected =
        matches!(&restore_policy.plan, ContextRestorePlan::Reinject { .. });
    let clear_context_carry_after_start =
        !had_runtime && matches!(&restore_policy.plan, ContextRestorePlan::NoContext);
    let recovery_restore_required = restore_policy.recovery_restore_required;
    let expected_provider_session_generation = restore_policy.expected_provider_session_generation;
    let restore_plan = restore_policy.plan;
    let prompt = apply_restore_prompt_prefix(queued.content.clone(), &restore_plan);
    let selected_model = match had_runtime
        .then(|| selected_model_for_runtime(ctx, &session))
        .transpose()
    {
        Ok(model) => model,
        Err(error) => {
            log::warn!(
                "queued turn model preflight remains pending for {session_id}; provider start was not claimed: {error}"
            );
            return;
        }
    };
    let mut queued_for_turn = queued.clone();
    queued_for_turn.existing_human_message_id = Some(human_message.id.clone());
    queued_for_turn.existing_agent_message_id = Some(agent_message_id.clone());
    if !durably_accepted {
        let mut sessions = ctx.sessions.lock().await;
        let Some(state) = sessions.get_mut(session_id) else {
            return;
        };
        let Some(effect) = state.accepted_input_effects.get_mut(&queued.id) else {
            return;
        };
        effect.existing_human_message_id = queued_for_turn.existing_human_message_id.clone();
        effect.existing_agent_message_id = queued_for_turn.existing_agent_message_id.clone();
    }
    let mut generation = None;
    if durably_accepted {
        let (operation_id, obligation_id, driver) = accepted_obligation
            .as_ref()
            .expect("durably accepted queue has an obligation driver");
        let turn_started = AgentSessionEvent::TurnStarted {
            turn_id,
            message_id: human_message.id.clone(),
            assistant_message_id: Some(agent_message_id.clone()),
            prompt: committed_prompt.clone(),
            at: human_message.timestamp,
        };
        match driver
            .claim_queued_turn_execution(
                operation_id,
                obligation_id,
                session_id,
                &queued.id,
                turn_started,
            )
            .await
        {
            Ok(AcceptedQueuedTurnExecutionClaimOutcome::Claimed(claim)) => {
                accepted_claim = Some(claim);
            }
            Ok(AcceptedQueuedTurnExecutionClaimOutcome::Blocked) => {
                log::debug!(
                    "accepted queued turn remains blocked by canonical lifecycle state for {session_id}"
                );
                return;
            }
            Err(()) => {
                log::warn!(
                    "accepted queued turn atomic claim remains pending for {session_id}; recovery was notified"
                );
                return;
            }
        }
        generation = {
            let mut sessions = ctx.sessions.lock().await;
            sessions.get_mut(session_id).and_then(|state| {
                state
                    .accepted_input_effects
                    .remove(&queued.id)
                    .is_some()
                .then(|| {
                    state.reset_for_turn(turn_id, agent_message_id.clone());
                    state.current_turn_input = Some(queued_for_turn.clone());
                    state.generation()
                })
            })
        };
        if generation.is_none() {
            if let Some((operation_id, obligation_id, driver)) = &accepted_obligation {
                arm_accepted_send_recovery_after_claim_release(
                    driver.as_ref(),
                    operation_id,
                    obligation_id,
                    &mut accepted_claim,
                )
                .await;
            }
            log::error!(
                "accepted queued send committed TurnStarted but lost its in-memory queue identity"
            );
            return;
        }
    }
    if !durably_accepted {
        #[cfg(test)]
        {
            // Legacy tests retain the historical split boundary and keep the
            // queue visible until the provider accepts start_turn below.
            if let Err(error) = ctx.session_store.append_turn_started_and_project_state(
                &ctx.data_dir,
                session_id,
                AgentSessionEvent::TurnStarted {
                    turn_id,
                    message_id: human_message.id.clone(),
                    assistant_message_id: Some(agent_message_id.clone()),
                    prompt: committed_prompt,
                    at: human_message.timestamp,
                },
            ) {
                log::warn!("failed to append queued TurnStarted for {session_id}: {error}");
                return;
            }
            generation = {
                let mut sessions = ctx.sessions.lock().await;
                sessions.get_mut(session_id).and_then(|state| {
                    state.accepted_input_effects.contains_key(&queued.id).then(|| {
                        state.reset_for_turn(turn_id, agent_message_id.clone());
                        state.current_turn_input = Some(queued_for_turn.clone());
                        state.generation()
                    })
                })
            };
        }
        #[cfg(not(test))]
        unreachable!("production queues must have a durable accepted operation identity");
    }
    let Some(generation) = generation else {
        return;
    };
    let runtime = match runtime {
        Some(runtime) => runtime,
        None => {
            let runtime_open_epoch = {
                let mut sessions = ctx.sessions.lock().await;
                let Some(state) = sessions.get_mut(session_id) else {
                    return;
                };
                if !state.matches_generation(generation) {
                    return;
                }
                state.bump_runtime_epoch()
            };
            match open_runtime_for_session(
                ctx,
                &session,
                system_prompt.clone(),
                Some(runtime_open_epoch),
            )
            .await
            {
                Ok(runtime) => runtime,
                Err(AgentRuntimeError::BackendSessionLost { .. }) => {
                    if let Err(error) = recover_backend_session(
                        ctx,
                        session_id,
                        BackendSessionRecoveryReason::BackendSessionLost,
                    )
                    .await
                    {
                        if let Some((operation_id, obligation_id, driver)) = &accepted_obligation {
                            arm_accepted_send_recovery_after_claim_release(
                                driver.as_ref(),
                                operation_id,
                                obligation_id,
                                &mut accepted_claim,
                            )
                            .await;
                        }
                        log::warn!(
                            "failed to recover backend session for queued turn {session_id}: {error}"
                        );
                    }
                    return;
                }
                Err(error) => {
                    log::warn!("failed to reopen runtime for queued turn {session_id}: {error}");
                    let terminal = complete_turn_with_acceptance_and_persist_kind(
                        ctx,
                        session_id,
                        Some(generation),
                        TurnResult::Interrupted {
                            reason: DomainInterruptReason::Crash,
                            error: Some(error.to_string()),
                        },
                        PersistFailureKind::ReopenRuntime,
                    )
                    .await;
                    match terminal {
                        Ok((Some(notification), _)) => {
                            dispatch_workflow_turn_complete_notification(
                                &ctx.workflow_turn_complete_notifier,
                                notification,
                            )
                            .await;
                        }
                        Ok((None, _)) => {}
                        Err(persist_error) => {
                            if let Some((operation_id, obligation_id, driver)) =
                                &accepted_obligation
                            {
                                arm_accepted_send_recovery_after_claim_release(
                                    driver.as_ref(),
                                    operation_id,
                                    obligation_id,
                                    &mut accepted_claim,
                                )
                                .await;
                            }
                            log::error!(
                                "failed to persist queued runtime reopen error for {session_id}: {persist_error}"
                            );
                        }
                    }
                    return;
                }
            }
        }
    };
    if !turn_owns_runtime(ctx, session_id, generation, &runtime).await {
        if let Some((operation_id, obligation_id, driver)) = &accepted_obligation {
            arm_accepted_send_recovery_after_claim_release(
                driver.as_ref(),
                operation_id,
                obligation_id,
                &mut accepted_claim,
            )
            .await;
        }
        detach_runtime_if_current(ctx, session_id, &runtime).await;
        return;
    }
    let start_result = async {
        if let Some(model) = selected_model {
            runtime.set_model(&model).await?;
        }
        runtime
            .start_turn(TurnInput {
                prompt,
                images: queued
                    .images
                    .iter()
                    .cloned()
                    .map(|image| AttachmentPayload {
                        data: image.data,
                        media_type: image.media_type,
                    })
                    .collect(),
                system_prompt,
                permission_mode: queued.permission_mode,
                plan_mode: queued.plan_mode,
                permission_profile_id: queued.permission_profile_id.clone(),
                editor_context: queued.editor_context.clone().map(EditorContext::from),
            })
            .await
    }
    .await;
    let _runtime_event_guard = ctx.runtime_event_locks.acquire(session_id).await;
    if let Err(error) = start_result {
        if let Some((operation_id, obligation_id, driver)) = &accepted_obligation {
            arm_accepted_send_recovery_after_claim_release(
                driver.as_ref(),
                operation_id,
                obligation_id,
                &mut accepted_claim,
            )
            .await;
        }
        if !turn_runtime_is_current(ctx, session_id, generation, &runtime).await {
            return;
        }
        log::warn!("failed to start queued turn for {session_id}: {error}");
        match complete_turn_with_acceptance_and_persist_kind(
            ctx,
            session_id,
            Some(generation),
            TurnResult::Interrupted {
                reason: DomainInterruptReason::Crash,
                error: Some(error.to_string()),
            },
            PersistFailureKind::QueuedTurnInterrupt,
        )
        .await
        {
            Ok((Some(notification), _)) => {
                dispatch_workflow_turn_complete_notification(
                    &ctx.workflow_turn_complete_notifier,
                    notification,
                )
                .await;
            }
            Ok((None, _)) => {}
            Err(persist_error) => {
                log::warn!(
                    "failed to persist queued turn interruption for {session_id}: {persist_error}"
                );
            }
        }
    } else {
        spawn_stale_watchdog_task(
            ctx,
            session_id.to_string(),
            generation,
            stale_timeout_for_session(&session),
        );
        let runtime_epoch = {
            let sessions = ctx.sessions.lock().await;
            sessions
                .get(session_id)
                .filter(|state| state.matches_generation(generation))
                .map(RuntimeSessionState::runtime_epoch)
                .unwrap_or_default()
        };
        if let Some((operation_id, obligation_id, _)) = &accepted_obligation {
            mark_accepted_turn_running_or_retry(
                ctx,
                session_id,
                generation,
                operation_id.clone(),
                obligation_id.clone(),
                turn_id,
            );
        }
        if !turn_owns_runtime(ctx, session_id, generation, &runtime).await {
            return;
        }
        #[cfg(test)]
        if !durably_accepted {
            let mut sessions = ctx.sessions.lock().await;
            if let Some(state) = sessions.get_mut(session_id) {
                state.accepted_input_effects.remove(&queued.id);
            }
        }
        ctx.notifier.pending_message_consumed(
            session_id,
            Some(queued.id.clone()),
            Some(human_message.clone()),
            agent_message.clone(),
        );
        ctx.notifier
            .turn_prepared(&session, &human_message, &agent_message);
        drop(_runtime_event_guard);
        complete_context_restore_after_start_or_retry(
            ctx,
            session_id.to_string(),
            runtime_epoch,
            ContextRestoreCompletionRequest::after_started_turn(
                expected_provider_session_generation,
                turn_id,
                context_was_reinjected,
                clear_context_carry_after_start,
                recovery_restore_required,
            ),
        );
        emit_session_state_change_from_session(
            &session,
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
    }
}

fn queued_human_message(queued: &QueuedTurnInput) -> ChatMessage {
    ChatMessage {
        id: queued
            .existing_human_message_id
            .clone()
            .unwrap_or_else(|| queued.id.clone()),
        role: MessageRole::Human,
        content: queued.content.clone(),
        thinking: None,
        activities: None,
        parts: (!human_parts(&queued.content, &queued.images).is_empty())
            .then(|| human_parts(&queued.content, &queued.images)),
        streaming_final_seq: 0,
        timestamp: queued.created_at,
        mentions: (!queued.mentions.is_empty()).then(|| {
            queued
                .mentions
                .iter()
                .cloned()
                .map(crate::usecase::agent_session::session::MessageMention::from_domain)
                .collect()
        }),
    }
}

fn queued_agent_projection(message_id: String, timestamp: f64) -> ChatMessage {
    ChatMessage {
        id: message_id,
        role: MessageRole::Agent,
        content: String::new(),
        thinking: None,
        activities: None,
        parts: None,
        streaming_final_seq: 0,
        timestamp,
        mentions: None,
    }
}
