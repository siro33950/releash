fn spawn_stale_watchdog_task(
    ctx: &RuntimeContext,
    session_id: String,
    generation: u64,
    timeout: std::time::Duration,
) {
    let ctx = ctx.clone();
    let spawner = Arc::clone(&ctx.spawner);
    spawner.spawn(Box::pin(async move {
        loop {
            let next = {
                let _session_guard = ctx.session_locks.acquire(&session_id).await;
                let mut sessions = ctx.sessions.lock().await;
                let Some(state) = sessions.get_mut(&session_id) else {
                    return;
                };
                maybe_mark_permission_wait_diagnostic(
                    &session_id,
                    state,
                    std::time::Instant::now(),
                );
                let effective_timeout = effective_stale_timeout(
                    timeout,
                    has_in_flight_tool_use(state.canonical_streaming_parts()),
                );
                if !turn_is_stale(
                    state.projected_turn_phase(),
                    generation,
                    state.generation(),
                    state.last_progress_at(),
                    effective_timeout,
                    std::time::Instant::now(),
                ) {
                    if !stale_watchdog_should_continue_waiting(
                        state.projected_turn_phase(),
                        generation,
                        state.generation(),
                    ) {
                        return;
                    }
                    if state.permission_request_cache.is_some() {
                        std::time::Duration::from_secs(5).min(timeout)
                    } else {
                        remaining_until_stale(
                            state.last_progress_at(),
                            effective_timeout,
                            std::time::Instant::now(),
                        )
                        .unwrap_or(effective_timeout)
                        // ツール実行中に延長された timeout はツール完了（ToolResult 到着）で
                        // 基準値へ戻るため、待機は基準 timeout を上限にして再評価する。
                        .min(timeout)
                    }
                    .max(std::time::Duration::from_millis(1))
                } else {
                    std::time::Duration::ZERO
                }
            };
            if !next.is_zero() {
                tokio::time::sleep(next).await;
                continue;
            }

            let observation = {
                let _session_guard = ctx.session_locks.acquire(&session_id).await;
                let observation = {
                    let mut sessions = ctx.sessions.lock().await;
                    let Some(state) = sessions.get_mut(&session_id) else {
                        return;
                    };
                    let effective_timeout = effective_stale_timeout(
                        timeout,
                        has_in_flight_tool_use(state.canonical_streaming_parts()),
                    );
                    if !turn_is_stale(
                        state.projected_turn_phase(),
                        generation,
                        state.generation(),
                        state.last_progress_at(),
                        effective_timeout,
                        std::time::Instant::now(),
                    ) {
                        continue;
                    }
                    let now = std::time::Instant::now();
                    let has_runtime = state.runtime.is_some();
                    let RuntimeStallDecision::Observe {
                        signal,
                        request_recovery,
                        should_rearm,
                    } = state.observe_stall(has_runtime, now)
                    else {
                        return;
                    };
                    let payload = signal.map(|signal| AgentStallObservedPayload {
                            chat_session_id: session_id.clone(),
                            turn_phase: state.projected_turn_phase(),
                            idle_secs: signal.idle_secs,
                            signal_count: signal.signal_count,
                            cap_reached: signal.cap_reached,
                        });
                    let runtime = request_recovery
                        .then(|| state.runtime.clone())
                        .flatten();
                    StallObservation {
                        payload,
                        runtime,
                        should_rearm,
                        rearm_delay: effective_timeout.min(timeout),
                    }
                };
                if let Some(payload) = observation.payload.clone() {
                    let workflow_notification =
                        ctx.projection_gateway.workflow_stall_observed(&payload);
                    ctx.notifier.stall_observed(payload);
                    // WorkflowStallObserved dispatch intentionally completes while the
                    // per-session runtime lock is held. The event pump dispatches
                    // WorkflowStallCleared for KeepAlive/PartsMerged under the same lock, so
                    // observe and clear are serialized here; moving this await outside the lock
                    // would reintroduce the clear-overtakes-observe race fixed in 1d4105e9.
                    dispatch_workflow_stall_observed_notification(
                        &ctx.workflow_stall_notifier,
                        workflow_notification,
                    )
                    .await;
                }
                observation
            };

            if let Some(runtime) = observation.runtime {
                match runtime.reconnect().await {
                    Ok(()) => {}
                    Err(AgentBackendError::Unavailable(message)) => {
                        log::debug!(
                            "agent runtime reconnect unavailable for {session_id}: {message}"
                        );
                    }
                    Err(error) => {
                        log::warn!("agent runtime reconnect failed for {session_id}: {error}");
                    }
                }
            }
            if !observation.should_rearm {
                return;
            }
            if !observation.rearm_delay.is_zero() {
                tokio::time::sleep(observation.rearm_delay).await;
            }
        }
    }));
}

struct StallObservation {
    payload: Option<AgentStallObservedPayload>,
    runtime: Option<Arc<dyn AgentSessionRuntime>>,
    should_rearm: bool,
    rearm_delay: std::time::Duration,
}

async fn open_runtime_for_session(
    ctx: &RuntimeContext,
    session: &ChatSession,
    system_prompt: Option<String>,
    expected_runtime_epoch: Option<u64>,
) -> Result<Arc<dyn AgentSessionRuntime>, AgentRuntimeError> {
    let backend_id = required_backend_id(session)?;
    let backend = ctx.registry.get(&backend_id).ok_or_else(|| {
        AgentRuntimeError::Other(format!("Agent backend not found: {backend_id}"))
    })?;
    let model_id = match session.selected_model.as_deref() {
        Some(model) => model.to_string(),
        None => ctx
            .registry
            .default_model_for(&backend_id)
            .map_err(AgentRuntimeError::Other)?,
    };
    let base_branch = ctx.branch_diff_context.as_ref().and_then(|port| {
        match port.get_branch_diff_context(&session.worktree_path) {
            Ok(summary) => (!summary.base_branch.trim().is_empty()).then_some(summary.base_branch),
            Err(error) => {
                log::debug!(
                    "failed to resolve base branch for agent child env {}: {error}",
                    session.id
                );
                None
            }
        }
    });
    let queue_paused_at = ctx
        .session_store
        .load_queue_paused_at(&ctx.data_dir, &session.id)
        .map_err(AgentRuntimeError::Other)?;
    let extra_env = workflow_execution_env(session.workflow_node_context.as_ref());
    let mut runtime = backend
        .open_session(SessionSpec {
            session_id: session.id.clone(),
            cwd: session.worktree_path.clone(),
            permission_mode: PermissionMode::parse(&session.permission_mode)
                .map_err(|error| AgentRuntimeError::Other(error.to_string()))?,
            plan_mode: session.plan_mode,
            permission_profile_id: session.permission_profile_id.clone(),
            model: ModelId::parse(&model_id).map_err(AgentRuntimeError::Other)?,
            system_prompt,
            resume: session.agent_session_id.clone(),
            base_branch,
            startup_timeout: startup_timeout_for_session(session),
            startup_max_retries: startup_max_retries_for_session(session),
            stale_timeout: None,
            extra_env,
        })
        .await
        .map_err(AgentRuntimeError::from)?;
    let events = runtime.take_events();
    let runtime: Arc<dyn AgentSessionRuntime> = Arc::from(runtime);
    let runtime_epoch = {
        let mut sessions = ctx.sessions.lock().await;
        let state = sessions.entry(session.id.clone()).or_insert_with(|| {
            RuntimeSessionState::with_queue_pause(backend_id.clone(), queue_paused_at)
        });
        if expected_runtime_epoch.is_some_and(|epoch| {
            !state.owns_runtime_epoch(epoch)
                || state.queue_is_paused()
                || state.interrupt_requested_for_current()
        }) {
            drop(sessions);
            runtime.close().await;
            return Err(AgentRuntimeError::Other(format!(
                "Runtime open was superseded for session {}",
                session.id
            )));
        }
        state.backend_id = backend_id;
        state.runtime = Some(Arc::clone(&runtime));
        expected_runtime_epoch.unwrap_or_else(|| state.bump_runtime_epoch())
    };
    spawn_event_pump_task(ctx, session.id.clone(), runtime_epoch, events);
    Ok(runtime)
}

fn workflow_execution_env(
    context: Option<&crate::usecase::agent_session::session::WorkflowNodeContextDto>,
) -> Vec<(String, String)> {
    context
        .map(|context| {
            vec![
                (
                    "RELEASH_WORKFLOW_EXECUTION_ID".to_string(),
                    context.execution_id.clone(),
                ),
                (
                    "RELEASH_NODE_EXECUTION_ID".to_string(),
                    context.node_execution_id.clone(),
                ),
            ]
        })
        .unwrap_or_default()
}

fn selected_model_for_runtime(
    ctx: &RuntimeContext,
    session: &ChatSession,
) -> Result<ModelId, AgentBackendError> {
    let model_id = match session.selected_model.as_deref() {
        Some(model_id) => model_id.to_string(),
        None => ctx
            .registry
            .default_model_for(
                &required_backend_id(session)
                    .map_err(|error| AgentBackendError::Invalid(error.to_string()))?,
            )
            .map_err(AgentBackendError::Invalid)?,
    };
    ModelId::parse(&model_id).map_err(AgentBackendError::Invalid)
}

fn spawn_event_pump_task(
    ctx: &RuntimeContext,
    session_id: String,
    runtime_epoch: u64,
    mut events: std::pin::Pin<Box<dyn futures_util::Stream<Item = AgentRuntimeEvent> + Send>>,
) {
    let ctx = ctx.clone();
    let spawner = Arc::clone(&ctx.spawner);
    spawner.spawn(Box::pin(async move {
        while let Some(event) = events.next().await {
            let event_received_at = crate::usecase::agent_session::session::now_timestamp();
            let mut persistence_retry =
                crate::domain::agent_session::aggregates::runtime_progress::PersistenceRetry::default();
            loop {
                let applied = {
                    let _session_guard = ctx.session_locks.acquire(&session_id).await;
                    let _runtime_event_guard = ctx.runtime_event_locks.acquire(&session_id).await;
                    apply_runtime_event(
                        &ctx,
                        &session_id,
                        runtime_epoch,
                        event_received_at,
                        event.clone(),
                    )
                    .await
                };
                match applied {
                    Ok(actions) => {
                        run_runtime_event_post_actions(&ctx, &session_id, actions).await;
                        break;
                    }
                    Err(error) => {
                        match persistence_retry.observe_failure() {
                            crate::domain::agent_session::aggregates::runtime_progress::PersistenceRetryObservation::First {
                                ..
                            } => {
                                log::error!(
                                    "canonical runtime event persistence failed for {session_id}; retaining the exact event for same-identity retry: {error}"
                                );
                            }
                            crate::domain::agent_session::aggregates::runtime_progress::PersistenceRetryObservation::Repeated {
                                attempt,
                            } => {
                                log::debug!(
                                    "canonical runtime event persistence retry {attempt} remains pending for {session_id}: {error}"
                                );
                            }
                        }
                        // Release both per-session locks between attempts so Stop, close,
                        // and a winning terminal can make progress. A changed runtime epoch or
                        // durable terminal is observed at the top of `apply_runtime_event` and
                        // safely supersedes this retained event.
                        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                    }
                }
            }
        }
    }));
}
