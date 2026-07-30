#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamingApplyMode {
    Coalesced,
    Immediate,
}

async fn apply_parts(
    ctx: &RuntimeContext,
    session_id: &str,
    parts: Vec<DomainMessagePart>,
    mode: StreamingApplyMode,
) -> Result<bool, String> {
    let domain_parts = parts;
    let delta_parts = domain_parts.clone();
    if delta_parts.is_empty() {
        return Ok(false);
    }
    let (
        turn_id,
        message_id,
        candidate_domain_parts,
        candidate_parts,
        next_streaming_seq,
        requires_snapshot,
    ) = {
        let sessions = ctx.sessions.lock().await;
        let Some(state) = sessions.get(session_id) else {
            return Ok(false);
        };
        if !state.has_active_turn_lease() {
            log::debug!("dropping late message parts after terminal commit for {session_id}");
            return Ok(false);
        }
        let message_id = state
            .streaming_message_id
            .clone()
            .or_else(|| state.last_agent_message_id.clone());
        let Some(message_id) = message_id else {
            return Ok(false);
        };
        let Some(turn_id) = state.active_turn_id() else {
            return Ok(false);
        };
        let stream_plan =
            state.prepare_stream_apply(&domain_parts, mode == StreamingApplyMode::Immediate);
        (
            turn_id,
            message_id,
            stream_plan.candidate_parts.clone(),
            stream_plan.candidate_parts,
            state.next_stream_sequence(),
            stream_plan.requires_snapshot,
        )
    };
    let durable_events = durable_part_events(
        &ctx.session_store,
        &ctx.data_dir,
        session_id,
        turn_id,
        &message_id,
        &domain_parts,
        &delta_parts,
    )?;
    let persisted_parts = ctx.session_store.persist_streaming_parts_with_events(
        &ctx.data_dir,
        session_id,
        &durable_events,
        &message_id,
        &candidate_parts,
        next_streaming_seq,
    )?;
    let persisted_at = std::time::Instant::now();
    let (emit_now, schedule_delay, cleared_stall) = {
        let mut sessions = ctx.sessions.lock().await;
        let Some(state) = sessions.get_mut(session_id) else {
            return Ok(false);
        };
        if !state.owns_stream_target(turn_id, &message_id) {
            return Ok(false);
        }
        state.commit_persisted_stream(
            candidate_domain_parts,
            persisted_parts,
            &delta_parts,
            requires_snapshot,
        );
        state.last_stream_persist_at = Some(persisted_at);
        let cleared_stall = state.record_progress(persisted_at);
        let pending = state.pending_stream_facts();
        let decision =
            crate::domain::agent_session::services::streaming_flush_decision_for_apply(
                mode == StreamingApplyMode::Immediate,
                pending.has_pending,
                state.has_coalesced_stream_retry(),
                pending.part_count,
                pending.byte_size,
                state.last_stream_emit_at,
                std::time::Instant::now(),
            );
        let mut schedule_delay = None;
        let emit_now = match decision {
            StreamingFlushDecision::Now => true,
            StreamingFlushDecision::Later(delay) => {
                if state.schedule_stream_flush() {
                    schedule_delay = Some(delay);
                }
                false
            }
            StreamingFlushDecision::NotNeeded => false,
        };
        (emit_now, schedule_delay, cleared_stall)
    };
    if cleared_stall {
        if let Err(error) = dispatch_stall_cleared_notifications(ctx, session_id).await {
            log::warn!("workflow stall-cleared notification failed for {session_id}: {error}");
        }
    }
    if emit_now {
        if let Err(error) = flush_streaming_update(ctx, session_id, false).await {
            log::warn!("failed to persist coalesced streaming parts for {session_id}: {error}");
        }
    } else if let Some(delay) = schedule_delay {
        spawn_delayed_stream_flush(ctx, session_id.to_string(), delay);
    }
    Ok(true)
}

fn spawn_delayed_stream_flush(
    ctx: &RuntimeContext,
    session_id: String,
    delay: std::time::Duration,
) {
    let ctx = ctx.clone();
    let spawner = Arc::clone(&ctx.spawner);
    spawner.spawn(Box::pin(async move {
        tokio::time::sleep(delay).await;
        let _session_guard = acquire_session_runtime_lock(&ctx.session_locks, &session_id).await;
        if let Err(error) = flush_streaming_update(&ctx, &session_id, false).await {
            log::warn!("failed to persist delayed streaming parts for {session_id}: {error}");
        }
    }));
}

async fn flush_streaming_update(
    ctx: &RuntimeContext,
    session_id: &str,
    force_persist: bool,
) -> Result<(), String> {
    let now = std::time::Instant::now();
    let (payload, persist_snapshot, emit_suppressed) = {
        let mut sessions = ctx.sessions.lock().await;
        let Some(state) = sessions.get_mut(session_id) else {
            return Ok(());
        };
        state.clear_stream_flush_schedule();
        let message_id = state
            .streaming_message_id
            .clone()
            .or_else(|| state.last_agent_message_id.clone());
        let retry = state.take_coalesced_stream_retry();
        let payload = if let Some(retry) = retry {
            Some(retry)
        } else if state.pending_stream_facts().has_pending {
            let Some(message_id) = message_id.clone() else {
                return Ok(());
            };
            let batch = state
                .take_pending_stream_flush()
                .expect("pending stream facts and flush transition must agree");
            Some(PendingStreamDelta {
                message_id,
                seq: state.next_stream_sequence(),
                snapshot: batch.snapshot,
                parts: batch.parts,
                message: None,
                authoritative: false,
            })
        } else {
            None
        };
        let persist = message_id.and_then(|message_id| {
            should_persist_streaming_snapshot(state.last_stream_persist_at, now, force_persist)
                .then(|| {
                    let seq = payload
                        .as_ref()
                        .map(|payload| payload.seq)
                        .unwrap_or_else(|| state.next_stream_sequence());
                    (
                        message_id,
                        seq,
                        state.persisted_streaming_parts().to_vec(),
                    )
                })
        });
        (payload, persist, state.stream_emit_is_suppressed())
    };

    let persist_result = if let Some((message_id, seq, parts)) = persist_snapshot {
        match ctx.session_store.persist_message_parts(
            &ctx.data_dir,
            session_id,
            &message_id,
            &parts,
            seq,
            None,
        ) {
            Ok(_) => {
                if let Some(state) = ctx.sessions.lock().await.get_mut(session_id) {
                    state.last_stream_persist_at = Some(now);
                }
                Ok(())
            }
            Err(error) => Err(error),
        }
    } else {
        Ok(())
    };

    if let Err(error) = persist_result {
        if payload.is_some() {
            let mut sessions = ctx.sessions.lock().await;
            if let Some(state) = sessions.get_mut(session_id) {
                // The attempted delta was removed from the pending fields above. Quarantine it
                // as a full snapshot and force the next flush to cross persistence again before
                // anything derived from it can become live.
                state.quarantine_stream_after_persist_failure();
                state.clear_coalesced_stream_retry();
                state.last_stream_persist_at = None;
            }
        }
        return Err(error);
    }

    let Some(payload) = payload else {
        return Ok(());
    };

    if emit_suppressed {
        return Ok(());
    }

    let emitted = ctx
        .notifier
        .streaming_delta(payload.to_delta_payload(session_id));

    let mut retry_delay = None;
    {
        let mut sessions = ctx.sessions.lock().await;
        let Some(state) = sessions.get_mut(session_id) else {
            return Ok(());
        };
        if emitted {
            state.observe_emitted_stream_sequence(payload.seq);
            state.last_stream_emit_at = Some(now);
            state.record_stream_emit_success();
        } else {
            retry_delay = on_stream_emit_failure(state, session_id, &payload);
        }
    };
    if let Some(delay) = retry_delay {
        spawn_delayed_stream_flush(ctx, session_id.to_string(), delay);
    }
    Ok(())
}

async fn emit_streaming_delta_or_retry(
    ctx: &RuntimeContext,
    session_id: &str,
    payload: PendingStreamDelta,
) {
    if payload.authoritative {
        emit_authoritative_streaming_delta_or_retry(ctx, session_id, payload).await;
        return;
    }
    {
        let mut sessions = ctx.sessions.lock().await;
        let Some(state) = sessions.get_mut(session_id) else {
            return;
        };
        if state.stream_emit_is_suppressed() {
            return;
        }
    }
    let now = std::time::Instant::now();
    let emitted = ctx
        .notifier
        .streaming_delta(payload.to_delta_payload(session_id));
    let retry_delay = {
        let mut sessions = ctx.sessions.lock().await;
        let Some(state) = sessions.get_mut(session_id) else {
            return;
        };
        if emitted {
            state.last_stream_emit_at = Some(now);
            state.record_stream_emit_success();
            return;
        }
        on_stream_emit_failure(state, session_id, &payload)
    };
    if let Some(delay) = retry_delay {
        spawn_delayed_stream_flush(ctx, session_id.to_string(), delay);
    }
}

async fn emit_authoritative_streaming_delta_or_retry(
    ctx: &RuntimeContext,
    session_id: &str,
    payload: PendingStreamDelta,
) {
    let retry_delay = {
        let mut sessions = ctx.sessions.lock().await;
        let Some(state) = sessions.get_mut(session_id) else {
            return;
        };
        prepare_authoritative_stream_emit(state, &payload.message_id);
        if state.authoritative_stream_retries_are_empty() {
            None
        } else {
            state.upsert_authoritative_stream_retry(payload.clone());
            if state.authoritative_stream_flush_is_scheduled() {
                return;
            }
            state.schedule_authoritative_stream_flush();
            Some(super::streaming::STREAMING_EMIT_INTERVAL)
        }
    };
    if let Some(delay) = retry_delay {
        spawn_delayed_authoritative_stream_flush(ctx, session_id.to_string(), delay);
        return;
    }
    let now = std::time::Instant::now();
    let emitted = ctx
        .notifier
        .streaming_delta(payload.to_delta_payload(session_id));
    let retry_delay = {
        let mut sessions = ctx.sessions.lock().await;
        let Some(state) = sessions.get_mut(session_id) else {
            return;
        };
        if emitted {
            state.last_stream_emit_at = Some(now);
            state.record_authoritative_stream_emit_success();
            None
        } else {
            on_authoritative_stream_emit_failure(state, session_id, &payload)
        }
    };
    if let Some(delay) = retry_delay {
        spawn_delayed_authoritative_stream_flush(ctx, session_id.to_string(), delay);
    }
}

fn prepare_authoritative_stream_emit(state: &mut RuntimeSessionState, message_id: &str) {
    // A backend-owned snapshot supersedes any older coalesced retry before the notifier call.
    // Delayed flushes are serialized by the session runtime lock and therefore observe this
    // updated state after the authoritative attempt completes.
    state.clear_coalesced_stream_retry();
    state.reset_stream_delivery();
    state.prepare_authoritative_stream_retry(message_id);
}

fn spawn_delayed_authoritative_stream_flush(
    ctx: &RuntimeContext,
    session_id: String,
    delay: std::time::Duration,
) {
    let ctx = ctx.clone();
    let spawner = Arc::clone(&ctx.spawner);
    spawner.spawn(Box::pin(async move {
        tokio::time::sleep(delay).await;
        let _session_guard = acquire_session_runtime_lock(&ctx.session_locks, &session_id).await;
        flush_authoritative_stream_retry(&ctx, &session_id).await;
    }));
}

async fn flush_authoritative_stream_retry(ctx: &RuntimeContext, session_id: &str) {
    {
        let mut sessions = ctx.sessions.lock().await;
        let Some(state) = sessions.get_mut(session_id) else {
            return;
        };
        state.clear_authoritative_stream_flush_schedule();
    }
    loop {
        let payload = {
            let sessions = ctx.sessions.lock().await;
            let Some(state) = sessions.get(session_id) else {
                return;
            };
            let Some(payload) = state.authoritative_stream_retry_front().cloned() else {
                return;
            };
            payload
        };
        let emitted = ctx
            .notifier
            .streaming_delta(payload.to_delta_payload(session_id));
        let retry_delay = {
            let mut sessions = ctx.sessions.lock().await;
            let Some(state) = sessions.get_mut(session_id) else {
                return;
            };
            if emitted {
                state.acknowledge_authoritative_stream_retry(&payload.message_id, payload.seq);
                state.record_authoritative_stream_emit_success();
                None
            } else {
                on_authoritative_stream_emit_failure(state, session_id, &payload)
            }
        };
        if let Some(delay) = retry_delay {
            spawn_delayed_authoritative_stream_flush(ctx, session_id.to_string(), delay);
            return;
        }
    }
}

fn on_stream_emit_failure(
    state: &mut RuntimeSessionState,
    session_id: &str,
    payload: &PendingStreamDelta,
) -> Option<std::time::Duration> {
    let decision = state.record_stream_emit_failure(state.has_coalesced_stream_retry());
    let failures = decision.failures();
    log::warn!(
        "agent-streaming-delta emit failure: chat_session={} message_id={} seq={} snapshot={} part_count={} consecutive_failures={}",
        session_id,
        payload.message_id,
        payload.seq,
        payload.snapshot,
        payload.parts.len(),
        failures
    );
    match decision {
        StreamEmitFailureDecision::Stop { .. } => {
        log::error!(
            "agent-streaming-delta emit failed {failures} consecutive times for chat_session={session_id}; stopping streaming emit until turn end"
        );
        state.clear_coalesced_stream_retry();
        state.stop_pending_stream_delivery();
            None
        }
        StreamEmitFailureDecision::FallbackToSnapshot {
            fallback_started,
            schedule_retry,
            ..
        } => {
            if fallback_started {
            log::warn!(
                "agent-streaming-delta emit failed {failures} consecutive times for chat_session={session_id}; falling back to full snapshot resync"
            );
            }
            state.clear_coalesced_stream_retry();
            state.fallback_pending_stream_to_snapshot();
            schedule_retry.then_some(super::streaming::STREAMING_EMIT_INTERVAL)
        }
        StreamEmitFailureDecision::RetryDelta {
            install_snapshot_retry,
            schedule_retry,
            ..
        } => {
            if install_snapshot_retry {
                state.replace_coalesced_stream_retry(Some(PendingStreamDelta {
                    snapshot: true,
                    parts: state.persisted_streaming_parts().to_vec(),
                    ..payload.clone()
                }));
            }
            schedule_retry.then_some(super::streaming::STREAMING_EMIT_INTERVAL)
        }
    }
}

fn on_authoritative_stream_emit_failure(
    state: &mut RuntimeSessionState,
    session_id: &str,
    payload: &PendingStreamDelta,
) -> Option<std::time::Duration> {
    let decision = state.record_authoritative_stream_emit_failure();
    let failures = decision.failures();
    log::warn!(
        "authoritative agent-streaming-delta emit failure: chat_session={} message_id={} seq={} part_count={} consecutive_failures={}",
        session_id,
        payload.message_id,
        payload.seq,
        payload.parts.len(),
        failures
    );
    match decision {
        StreamEmitFailureDecision::Stop { .. } => {
            log::error!(
                "authoritative agent-streaming-delta emit failed {failures} consecutive times for chat_session={session_id}; stopping delivery retry"
            );
            state.clear_authoritative_stream_retries();
            None
        }
        StreamEmitFailureDecision::RetryDelta { schedule_retry, .. }
        | StreamEmitFailureDecision::FallbackToSnapshot { schedule_retry, .. } => {
            state.upsert_authoritative_stream_retry(payload.clone());
            schedule_retry.then_some(super::streaming::STREAMING_EMIT_INTERVAL)
        }
    }
}

pub(super) async fn complete_turn(
    ctx: &RuntimeContext,
    session_id: &str,
    expected_generation: Option<u64>,
    result: crate::domain::agent_session::entities::TurnResult,
) -> Result<Option<WorkflowTurnCompleteNotification>, String> {
    complete_turn_with_acceptance_and_persist_kind(
        ctx,
        session_id,
        expected_generation,
        result,
        PersistFailureKind::FinalPartsRecorded,
    )
    .await
    .map(|(notification, _)| notification)
}
