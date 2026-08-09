impl AgentSessionRuntimeUsecase {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        session_store: Arc<SessionStore>,
        registry: Arc<AgentBackendRegistry>,
        status_center: Arc<AgentStatusCenter>,
        status_notifier: Arc<dyn AgentStatusNotifier>,
        notifier: Arc<dyn AgentSessionEventNotifier>,
        projection_gateway: Arc<dyn AgentRuntimeProjectionGateway>,
        spawner: Arc<dyn AgentTaskSpawner>,
        branch_diff_context: Option<Arc<dyn BranchDiffContextPort>>,
        instruction_source: Arc<dyn InstructionSourcePort>,
        data_dir: PathBuf,
        workspace_query: Arc<dyn crate::usecase::workspace_tree::WorkspaceQueryService>,
    ) -> Self {
        Self {
            ctx: RuntimeContext {
                session_store,
                registry,
                status_center,
                status_notifier,
                notifier,
                projection_gateway,
                spawner,
                branch_diff_context,
                instruction_source,
                data_dir: Arc::new(data_dir),
                workspace_query,
                sessions: Arc::new(Mutex::new(RuntimeSessionMap::new())),
                session_locks: SessionCommandLocks::default(),
                runtime_event_locks: SessionLockMap::default(),
                transitions: SessionTransitionCoordinator::default(),
                shutdown_admission: Arc::new(ShutdownAdmission::default()),
                workflow_turn_complete_notifier: Arc::new(RwLock::new(None)),
                workflow_stall_notifier: Arc::new(RwLock::new(None)),
                accepted_send_obligation_driver: Arc::new(RwLock::new(None)),
                durable_workflow_send_driver: Arc::new(RwLock::new(None)),
                durable_stop_driver: Arc::new(RwLock::new(None)),
                lifecycle_repository: Arc::new(RwLock::new(None)),
            },
        }
    }

    pub(crate) fn report_event_log_recovered(&self, session_id: &str) {
        report_event_log_recovered(
            &self.ctx.status_center,
            &self.ctx.status_notifier,
            &self.ctx.notifier,
            session_id,
        );
    }

    pub fn set_workflow_turn_complete_notifier(
        &self,
        notifier: Arc<dyn WorkflowTurnCompleteNotifier>,
    ) {
        *self
            .ctx
            .workflow_turn_complete_notifier
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(notifier);
    }

    pub fn set_workflow_stall_notifier(&self, notifier: Arc<dyn WorkflowStallNotifier>) {
        *self
            .ctx
            .workflow_stall_notifier
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(notifier);
    }

    pub(crate) fn set_accepted_send_obligation_driver(
        &self,
        driver: Arc<dyn AcceptedSendObligationDriver>,
    ) {
        *self
            .ctx
            .accepted_send_obligation_driver
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(driver);
    }

    pub(crate) fn set_durable_workflow_send_driver(
        &self,
        driver: Arc<dyn DurableWorkflowSendDriver>,
    ) {
        *self
            .ctx
            .durable_workflow_send_driver
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(driver);
    }

    pub(crate) fn set_durable_stop_driver(&self, driver: Arc<dyn DurableStopDriver>) {
        *self
            .ctx
            .durable_stop_driver
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(driver);
    }

    pub(crate) fn set_lifecycle_repository(
        &self,
        repository: Arc<
            dyn crate::domain::agent_session::repository::AgentSessionLifecycleRepository,
        >,
    ) {
        *self
            .ctx
            .lifecycle_repository
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Arc::downgrade(&repository));
    }

    pub fn list_backends(&self) -> BackendListResult {
        self.ctx.registry.list_result()
    }

    #[cfg(test)]
    pub(crate) fn backend_registry(&self) -> &AgentBackendRegistry {
        self.ctx.registry.as_ref()
    }

    #[cfg(test)]
    pub async fn send_message(
        &self,
        req: SendAgentMessageRequest,
    ) -> Result<SendMessageResponse, AgentRuntimeError> {
        self.send_message_with_reserved_session_id(req, None).await
    }

    /// Executes an already accepted send using the session identity fixed by
    /// the durable receipt. `reserved_session_id` is used only for a new
    /// session; regular callers keep using [`Self::send_message`].
    #[cfg(test)]
    pub async fn send_message_with_reserved_session_id(
        &self,
        req: SendAgentMessageRequest,
        reserved_session_id: Option<String>,
    ) -> Result<SendMessageResponse, AgentRuntimeError> {
        let _admission_guard = self.ctx.shutdown_admission.admit()?;
        let mut session_guard = match req.chat_session_id.as_deref() {
            Some(session_id) => {
                Some(acquire_session_control_after_recovery(&self.ctx, session_id).await)
            }
            None => None,
        };
        if let Some(session_id) = req.chat_session_id.as_deref() {
            self.ensure_session_not_closing(session_id).await?;
        }
        let session = self
            .resolve_or_create_session(&req, reserved_session_id.as_deref())
            .await?;
        if session_guard.is_none() {
            session_guard =
                Some(acquire_session_control_after_recovery(&self.ctx, &session.id).await);
        }
        let images = req.images.unwrap_or_default();
        let mentions = req.mentions.unwrap_or_default();
        let session_id = session.id.clone();
        let backend_id = required_backend_id(&session)?;
        self.hydrate_runtime_session_state(&session).await?;
        self.recover_queued_turn_if_idle_without_runtime(&session_id)
            .await;
        let stalled_active_turn = if self.backend_supports_steering(&backend_id) {
            self.stalled_active_turn_target(&session_id).await?
        } else {
            None
        };
        if self.is_turn_busy(&session_id).await {
            if let Some(target) = stalled_active_turn {
                target
                    .runtime
                    .steer(TurnInput {
                        prompt: req.content.clone(),
                        images: images
                            .iter()
                            .cloned()
                            .map(|image| AttachmentPayload {
                                data: image.data,
                                media_type: image.media_type,
                            })
                            .collect(),
                        system_prompt: None,
                        permission_mode: req.permission_mode,
                        plan_mode: req.plan_mode,
                        permission_profile_id: session.permission_profile_id.clone(),
                        editor_context: req.editor_context.clone().map(EditorContext::from),
                    })
                    .await
                    .map_err(AgentRuntimeError::from)?;
                let (human_message, _) = add_human_message_internal(
                    &self.ctx.session_store,
                    &self.ctx.data_dir,
                    &session_id,
                    &req.content,
                    &images,
                    &mentions,
                )?;
                return self.send_response(
                    &session_id,
                    &session.worktree_path,
                    human_message,
                    None,
                    None,
                    self.pending_queue(&session_id).await,
                );
            }
            // Resolve fallible read-model projections before accepting the message. Once the
            // human message is persisted and queued, the command must return an accepted
            // response so the composer cannot retain and resend an already-queued input.
            let response_projection =
                self.prepare_send_response_projection(&session_id, &session.worktree_path)?;
            let session_title = self
                .ctx
                .session_store
                .session_title(&self.ctx.data_dir, &session_id)
                .map_err(AgentRuntimeError::Other)?;
            let (human_message, persisted_meta) = add_human_message_internal(
                &self.ctx.session_store,
                &self.ctx.data_dir,
                &session_id,
                &req.content,
                &images,
                &mentions,
            )?;
            let mut queued = QueuedTurnInput::new(
                req.content,
                req.permission_mode,
                req.plan_mode,
                session.permission_profile_id.clone(),
                images,
                session.worktree_path.clone(),
                mentions,
                req.editor_context,
            );
            queued.existing_human_message_id = Some(human_message.id.clone());
            let queued_view = QueuedAgentTurn::from(&queued);
            let pending_queue = {
                let mut sessions = self.ctx.sessions.lock().await;
                let state = sessions
                    .entry(session_id.clone())
                    .or_insert_with(|| RuntimeSessionState::new(backend_id));
                state.accepted_input_effects.insert(queued.id.clone(), queued);
                pending_queue_view(state)
            };
            return Ok(response_projection.into_accepted_queue_response(
                session_title,
                human_message,
                persisted_meta,
                queued_view,
                pending_queue,
            ));
        }

        let (human_message, _) = add_human_message_internal(
            &self.ctx.session_store,
            &self.ctx.data_dir,
            &session_id,
            &req.content,
            &images,
            &mentions,
        )?;
        let agent_message = add_message_internal(
            &self.ctx.session_store,
            &self.ctx.data_dir,
            &session_id,
            MessageRole::Agent,
            "",
            None,
            None,
        )
        .map_err(AgentRuntimeError::Other)?;

        self.ctx
            .notifier
            .turn_prepared(&session, &human_message, &agent_message);
        let system_prompt = self.build_turn_system_prompt(
            &session,
            None,
            &mentions,
            req.editor_context.as_ref(),
            Vec::new(),
        )?;
        self.start_turn_for_session(
            &session,
            &human_message,
            agent_message.id.clone(),
            TurnStartPayload {
                prompt: req.content,
                images,
                mentions,
                permission_mode: req.permission_mode,
                plan_mode: req.plan_mode,
                permission_profile_id: session.permission_profile_id.clone(),
                editor_context: req.editor_context,
                system_prompt,
                accepted_execution_identity: None,
            },
            None,
            None,
        )
        .await?;

        let response = self.send_response(
            &session_id,
            &session.worktree_path,
            human_message,
            Some(agent_message),
            None,
            self.pending_queue(&session_id).await,
        );
        drop(session_guard);
        response
    }

    /// Consume the identities already committed by durable send acceptance.
    /// This path never performs send admission, creates a second human
    /// message, or appends another `TurnStarted` for an immediately-started
    /// disposition.
    pub(crate) async fn execute_accepted_send(
        &self,
        execution: AcceptedSendExecution<'_>,
    ) -> Result<(), AgentRuntimeError> {
        let AcceptedSendExecution {
            request: req,
            operation_id,
            execution_obligation_id,
            session_id,
            human_message_id,
            assistant_message_id,
            disposition,
            reserved_turn_id,
        } = execution;
        let _admission_guard = self
            .ctx
            .shutdown_admission
            .admit()
            .map_err(|error| fail_accepted_effect_preflight("shutdown-admission", error))?;
        let _session_guard = acquire_session_control_after_recovery(&self.ctx, session_id).await;
        self.ensure_session_not_closing(session_id)
            .await
            .map_err(|error| fail_accepted_effect_preflight("session-closing", error))?;
        let session = self
            .ctx
            .session_store
            .get_session_shell(&self.ctx.data_dir, session_id)
            .map_err(|error| fail_accepted_effect_preflight("session-shell", error))?
            .ok_or_else(|| AgentRuntimeError::Other(format!("Session not found: {session_id}")))?;
        let backend_id = required_backend_id(&session)?;
        self.hydrate_runtime_session_state(&session)
            .await
            .map_err(|error| fail_accepted_effect_preflight("runtime-hydration", error))?;

        let base_system_prompt = req.base_system_prompt;
        let workflow_instructions = req.workflow_instructions;
        let runtime_identity = validate_accepted_effect_runtime_identity(
            &disposition,
            reserved_turn_id,
            assistant_message_id,
        )
        .map_err(|rejection| {
            let message = match rejection {
                AcceptedEffectIdentityRejection::InvalidTurn => {
                    "accepted turn identity is invalid"
                }
                AcceptedEffectIdentityRejection::MissingReservedTurn => {
                    "accepted queued send is missing its reserved turn identity"
                }
                AcceptedEffectIdentityRejection::MissingAssistantMessage => {
                    "accepted send is missing its committed assistant identity"
                }
            };
            AgentRuntimeError::Other(message.into())
        })?;
        let mut accepted_input = QueuedTurnInput::new(
            req.content,
            req.permission_mode,
            req.plan_mode,
            session.permission_profile_id.clone(),
            req.images,
            session.worktree_path.clone(),
            req.mentions,
            req.editor_context,
        );
        accepted_input.existing_human_message_id = Some(human_message_id.to_string());
        accepted_input.existing_agent_message_id = assistant_message_id.map(str::to_string);
        accepted_input.reserved_turn_id = runtime_identity.reserved_turn_id;
        accepted_input.accepted_operation_id = Some(operation_id.to_string());
        accepted_input.execution_obligation_id = Some(execution_obligation_id.to_string());

        match runtime_identity.execution {
            AcceptedEffectExecutionIdentity::Queued { queue_item_id } => {
                accepted_input.id = queue_item_id;
                let reserved_turn_id = runtime_identity
                    .reserved_turn_id
                    .expect("queued runtime identity has a reserved turn");
                let canonical_queue = self
                    .ctx
                    .session_store
                    .canonical_pending_send_queue(session_id)
                    .map_err(|error| fail_accepted_effect_preflight("canonical-queue", error))?;
                let mut sessions = self.ctx.sessions.lock().await;
                let state = sessions
                    .entry(session_id.to_string())
                    .or_insert_with(|| RuntimeSessionState::new(backend_id));
                if state.current_turn_input.as_ref().is_some_and(|current| {
                    accepted_effect_execution_matches(
                        current.accepted_operation_id.as_deref(),
                        current.execution_obligation_id.as_deref(),
                        operation_id,
                        execution_obligation_id,
                    )
                }) {
                    return Ok(());
                }
                if state.accepted_input_effects.values().any(|queued| {
                    accepted_queued_effect_reservation_conflicts(
                        accepted_queued_effect_identity(queued),
                        accepted_queued_effect_identity(&accepted_input),
                    )
                }) {
                    return Err(AgentRuntimeError::Other(format!(
                        "accepted queued turn identity {reserved_turn_id} is already owned"
                    )));
                }
                cache_accepted_input_effect(
                    &mut state.accepted_input_effects,
                    accepted_input,
                    &canonical_queue,
                )
                .map_err(AgentRuntimeError::Other)?;
                Ok(())
            }
            AcceptedEffectExecutionIdentity::StartedTurn {
                turn_id: committed_turn_id,
                assistant_message_id,
            } => {
                let human_message = queued_human_message(&accepted_input);
                let agent_message = self
                    .ctx
                    .session_store
                    .canonical_message_projection(session_id, &assistant_message_id)
                    .map_err(|error| fail_accepted_effect_preflight("assistant-projection", error))?
                    .ok_or_else(|| {
                        AgentRuntimeError::Other(
                            "accepted assistant projection is unavailable".into(),
                        )
                    })?;
                self.ctx
                    .notifier
                    .turn_prepared(&session, &human_message, &agent_message);
                let system_prompt = self
                    .build_turn_system_prompt(
                        &session,
                        base_system_prompt,
                        &accepted_input.mentions,
                        accepted_input.editor_context.as_ref(),
                        workflow_instructions,
                    )
                    .map_err(|error| fail_accepted_effect_preflight("system-prompt", error))?;
                self.start_turn_for_session(
                    &session,
                    &human_message,
                    agent_message.id,
                    TurnStartPayload {
                        prompt: accepted_input.content,
                        images: accepted_input.images,
                        mentions: accepted_input.mentions,
                        permission_mode: accepted_input.permission_mode,
                        plan_mode: accepted_input.plan_mode,
                        permission_profile_id: accepted_input.permission_profile_id,
                        editor_context: accepted_input.editor_context,
                        system_prompt,
                        accepted_execution_identity: Some(AcceptedTurnExecutionIdentity {
                            operation_id: operation_id.to_string(),
                            execution_obligation_id: execution_obligation_id.to_string(),
                        }),
                    },
                    None,
                    Some(committed_turn_id),
                )
                .await
            }
        }
    }

    /// A stored provider id is only resume input. It does not prove the
    /// current process has observed a successful create/resume handshake.
    #[cfg(test)]
    pub(crate) async fn provider_session_is_confirmed(&self, session_id: &str) -> bool {
        let sessions = self.ctx.sessions.lock().await;
        sessions
            .get(session_id)
            .is_some_and(|state| {
                provider_session_is_confirmed(
                    state.runtime.is_some(),
                    state.provider_session_is_established(),
                )
            })
    }

    /// Process-local ownership proof for hiding only the exact accepted turn
    /// currently driven by this runtime. Durable status alone cannot
    /// distinguish a live reservation from one left by a crashed process.
    pub(crate) async fn owns_accepted_turn_execution(
        &self,
        session_id: &str,
        operation_id: &str,
        obligation_id: &str,
    ) -> bool {
        let sessions = self.ctx.sessions.lock().await;
        sessions.get(session_id).is_some_and(|state| {
            state.current_turn_input.as_ref().is_some_and(|input| {
                accepted_effect_is_process_owned(
                    state.has_active_turn_lease(),
                    input.accepted_operation_id.as_deref(),
                    input.execution_obligation_id.as_deref(),
                    operation_id,
                    obligation_id,
                )
            })
        })
    }

    /// Read-only recovery fence for workflow and other aggregate operations.
    /// This deliberately does not open or hydrate a live provider session.
    pub(crate) fn ensure_recovery_operation_allowed(
        &self,
        session_id: &str,
    ) -> Result<(), AgentRuntimeError> {
        ensure_backend_recovery_operation_allowed(&self.ctx, session_id)
    }

    #[cfg(test)]
    pub async fn start_session(
        &self,
        session_id: &str,
        opts: StartSessionOptions,
    ) -> Result<(), AgentRuntimeError> {
        let _admission_guard = self.ctx.shutdown_admission.admit()?;
        let _session_guard = acquire_session_control_after_recovery(&self.ctx, session_id).await;
        ensure_backend_recovery_operation_allowed(&self.ctx, session_id)?;
        self.ensure_session_not_closing(session_id).await?;
        let mut session = self
            .ctx
            .session_store
            .get_session_shell(&self.ctx.data_dir, session_id)
            .map_err(AgentRuntimeError::Other)?
            .ok_or_else(|| AgentRuntimeError::Other(format!("Session not found: {session_id}")))?;
        if should_apply_session_configuration(
            false,
            &session.permission_mode.as_str(),
            &opts.permission_mode.as_str(),
        ) {
            self.ctx
                .session_store
                .update_permission_mode(
                    &self.ctx.data_dir,
                    session_id,
                    opts.permission_mode.as_str(),
                )
                .map_err(AgentRuntimeError::Other)?;
            session.permission_mode = opts.permission_mode.as_str().to_string();
        }
        if should_apply_session_configuration(false, &session.plan_mode, &opts.plan_mode) {
            self.ctx
                .session_store
                .update_plan_mode(&self.ctx.data_dir, session_id, opts.plan_mode)
                .map_err(AgentRuntimeError::Other)?;
            session.plan_mode = opts.plan_mode;
        }
        match self.ensure_runtime(&session, None).await {
            Ok(_) => Ok(()),
            Err(AgentRuntimeError::BackendSessionLost { .. }) => {
                recover_backend_session(
                    &self.ctx,
                    session_id,
                    BackendSessionRecoveryReason::BackendSessionLost,
                )
                .await
            }
            Err(error) => Err(error),
        }
    }

    /// Process-local ownership fact used only for the post-commit mirror.
    /// Canonical permission admission is owned by the Session aggregate.
    pub(crate) async fn permission_request_is_runtime_owned(
        &self,
        session_id: &str,
        request_id: &str,
    ) -> Result<bool, AgentRuntimeError> {
        let sessions = self.ctx.sessions.lock().await;
        let state = sessions.get(session_id).ok_or_else(|| {
            AgentRuntimeError::Other(format!("No active agent runtime for session {session_id}"))
        })?;
        if state.runtime.is_none() {
            return Err(AgentRuntimeError::Other(format!(
                "No active agent runtime for session {session_id}"
            )));
        }
        Ok(runtime_permission_effect_is_owned(
            state.runtime.is_some(),
            state.owns_pending_permission_request(request_id),
        ))
    }

    /// The only production provider handoff for a permission response. The
    /// durable operation has already accepted and claimed the exact payload;
    /// this method deliberately performs no reservation or completion write.
    pub(crate) async fn execute_accepted_permission_response_effect(
        &self,
        session_id: &str,
        turn_id: u64,
        response: PermissionResponse,
    ) -> Result<(), AgentRuntimeError> {
        let _session_guard = self
            .acquire_session_control_after_recovery(session_id)
            .await;
        ensure_backend_recovery_operation_allowed(&self.ctx, session_id)?;
        self.ensure_session_not_closing(session_id).await?;
        let pending = self
            .pending_permission_for_response(session_id, &response)
            .await?;
        if !permission_response_turn_matches(pending.turn_id, turn_id) {
            return Err(AgentRuntimeError::Other(format!(
                "Permission response turn identity changed for session {session_id}"
            )));
        }
        let runtime = {
            let sessions = self.ctx.sessions.lock().await;
            sessions
                .get(session_id)
                .and_then(|state| state.runtime.clone())
        }
        .ok_or_else(|| {
            AgentRuntimeError::Other(format!("No active agent runtime for session {session_id}"))
        })?;
        runtime
            .respond_permission(response)
            .await
            .map_err(AgentRuntimeError::from)
    }

    /// Refresh process-local mirrors only after the operation completion
    /// batch (operation state, obligation, event and projections) is durable.
    pub(crate) async fn apply_permission_response_completion(
        &self,
        session_id: &str,
        turn_id: u64,
        response: &PermissionResponse,
        from_runtime_state: bool,
    ) {
        let (
            patched,
            did_resume_streaming,
            permission_wait_measurement,
            pending_permission_state_revision,
            cleared_stall,
        ) = {
            let mut sessions = self.ctx.sessions.lock().await;
            let Some(state) = sessions.get_mut(session_id) else {
                return;
            };
            let has_cached_request = state.permission_request_cache.is_some();
            let pending_in_state_matches = state
                .permission_request_cache
                .as_ref()
                .is_some_and(|pending| {
                    permission_request_identity_matches(&pending.id, &response.request_id)
                });
            let decision = decide_permission_response_runtime_completion(
                has_cached_request,
                pending_in_state_matches,
                from_runtime_state,
            );
            let patched = decision
                .patch_cached_projection
                .then(|| patch_permission_response_in_state(state, response))
                .flatten();
            let did_resume_streaming = decision.resume_streaming;
            let mut pending_permission_state_revision = None;
            let mut permission_wait_measurement = None;
            let mut cleared_stall = false;
            if did_resume_streaming {
                state.observe_canonical_turn_identity(turn_id);
                let (revision, elapsed) =
                    state.resolve_pending_permission_request(std::time::Instant::now());
                pending_permission_state_revision = Some(revision);
                permission_wait_measurement = elapsed;
                cleared_stall = state.record_progress(std::time::Instant::now());
            }
            (
                patched,
                did_resume_streaming,
                permission_wait_measurement,
                pending_permission_state_revision,
                cleared_stall,
            )
        };
        if cleared_stall {
            if let Err(error) = dispatch_stall_cleared_notifications(&self.ctx, session_id).await {
                log::warn!("workflow stall-cleared notification failed for {session_id}: {error}");
            }
        }
        if let Some(elapsed) = permission_wait_measurement {
            record_agent_turn_duration_detached(
                &self.ctx,
                session_id.to_string(),
                crate::other::telemetry::AgentTurn::PermissionWait,
                elapsed,
            );
        }
        if let Some((message_id, seq, parts, _turn_id)) = patched {
            emit_streaming_delta_or_retry(
                &self.ctx,
                session_id,
                PendingStreamDelta {
                    message_id,
                    seq,
                    snapshot: true,
                    parts,
                    message: None,
                    authoritative: true,
                },
            )
            .await;
        }
        if did_resume_streaming {
            emit_session_state_change(
                &self.ctx.session_store,
                &self.ctx.notifier,
                &self.ctx.status_center,
                &self.ctx.status_notifier,
                &self.ctx.data_dir,
                session_id,
                StateChange {
                    turn_phase: TurnPhase::Streaming,
                    queue_paused: None,
                    pending_permission_request: None,
                    pending_permission_state_revision,
                    exit_code: None,
                    completed_at: None,
                    interrupted: false,
                    session_state: Some(SessionState::Active),
                },
            );
        }
    }

    #[cfg(test)]
    pub async fn respond_permission(
        &self,
        session_id: &str,
        response: PermissionResponse,
    ) -> Result<(), AgentRuntimeError> {
        let _session_guard = self
            .acquire_session_control_after_recovery(session_id)
            .await;
        ensure_backend_recovery_operation_allowed(&self.ctx, session_id)?;
        self.ensure_session_not_closing(session_id).await?;
        let pending = self
            .pending_permission_for_response(session_id, &response)
            .await?;
        let turn_id = pending.turn_id.ok_or_else(|| {
            AgentRuntimeError::Other(format!(
                "Permission response has no durable turn identity for session {session_id}"
            ))
        })?;
        let runtime = {
            let sessions = self.ctx.sessions.lock().await;
            sessions
                .get(session_id)
                .and_then(|state| state.runtime.clone())
        }
        .ok_or_else(|| {
            AgentRuntimeError::Other(format!("No active agent runtime for session {session_id}"))
        })?;
        let projected_message = {
            let sessions = self.ctx.sessions.lock().await;
            sessions.get(session_id).and_then(|state| {
                let mut parts = state.canonical_streaming_parts().to_vec();
                if !crate::domain::agent_session::services::patch_permission_response(
                    &mut parts,
                    &response,
                ) {
                    return None;
                }
                state
                    .streaming_message_id
                    .clone()
                    .or_else(|| state.last_agent_message_id.clone())
                    .map(|message_id| (message_id, state.next_stream_sequence()))
            })
        };
        let obligation_id = self
            .ctx
            .session_store
            .reserve_permission_response(
                &self.ctx.data_dir,
                session_id,
                turn_id,
                &response.request_id,
                response.clone(),
            )
            .map_err(AgentRuntimeError::Other)?;
        self.ctx
            .session_store
            .claim_permission_response_effect(session_id, &obligation_id)
            .map_err(AgentRuntimeError::Other)?;
        runtime
            .respond_permission(response.clone())
            .await
            .map_err(AgentRuntimeError::from)?;
        let resolved_event = permission_resolved_event(turn_id, &response);
        self.ctx
            .session_store
            .complete_permission_response(
                &self.ctx.data_dir,
                session_id,
                &obligation_id,
                resolved_event,
                projected_message
                    .as_ref()
                    .map(|(message_id, _)| message_id.as_str()),
                projected_message.as_ref().map(|(_, seq)| *seq),
            )
            .map_err(AgentRuntimeError::Other)?;
        let (
            patched,
            did_resume_streaming,
            permission_wait_measurement,
            pending_permission_state_revision,
            cleared_stall,
        ) = {
            let mut sessions = self.ctx.sessions.lock().await;
            let Some(state) = sessions.get_mut(session_id) else {
                return Ok(());
            };
            let patched = patch_permission_response_in_state(state, &response);
            let has_cached_request = state.permission_request_cache.is_some();
            let pending_in_state_matches = state
                .permission_request_cache
                .as_ref()
                .is_some_and(|cached| {
                    permission_request_identity_matches(&cached.id, &response.request_id)
                });
            let did_resume_streaming = decide_permission_response_runtime_completion(
                has_cached_request,
                pending_in_state_matches,
                pending.from_runtime_state,
            )
            .resume_streaming;
            let mut pending_permission_state_revision = None;
            let mut permission_wait_measurement = None;
            let mut cleared_stall = false;
            if did_resume_streaming {
                state.observe_canonical_turn_identity(turn_id);
                let (revision, elapsed) =
                    state.resolve_pending_permission_request(std::time::Instant::now());
                pending_permission_state_revision = Some(revision);
                permission_wait_measurement = elapsed;
                cleared_stall = state.record_progress(std::time::Instant::now());
            }
            (
                patched,
                did_resume_streaming,
                permission_wait_measurement,
                pending_permission_state_revision,
                cleared_stall,
            )
        };
        if cleared_stall {
            if let Err(error) = dispatch_stall_cleared_notifications(&self.ctx, session_id).await {
                log::warn!("workflow stall-cleared notification failed for {session_id}: {error}");
            }
        }
        if let Some(elapsed) = permission_wait_measurement {
            record_agent_turn_duration_detached(
                &self.ctx,
                session_id.to_string(),
                crate::other::telemetry::AgentTurn::PermissionWait,
                elapsed,
            );
        }
        if let Some((message_id, seq, parts, _turn_id)) = patched {
            emit_streaming_delta_or_retry(
                &self.ctx,
                session_id,
                PendingStreamDelta {
                    message_id,
                    seq,
                    snapshot: true,
                    parts,
                    message: None,
                    authoritative: true,
                },
            )
            .await;
        }
        if did_resume_streaming {
            emit_session_state_change(
                &self.ctx.session_store,
                &self.ctx.notifier,
                &self.ctx.status_center,
                &self.ctx.status_notifier,
                &self.ctx.data_dir,
                session_id,
                StateChange {
                    turn_phase: TurnPhase::Streaming,
                    queue_paused: None,
                    pending_permission_request: None,
                    pending_permission_state_revision,
                    exit_code: None,
                    completed_at: None,
                    interrupted: false,
                    session_state: Some(SessionState::Active),
                },
            );
        }
        Ok(())
    }

    pub async fn report_permission_request_observed(
        &self,
        session_id: &str,
        request_id: &str,
        visible: bool,
    ) -> Result<(), AgentRuntimeError> {
        let mut sessions = self.ctx.sessions.lock().await;
        let Some(state) = sessions.get_mut(session_id) else {
            return Ok(());
        };
        state.report_permission_request_observed(
            request_id,
            visible,
            std::time::Instant::now(),
        );
        Ok(())
    }

    async fn pending_permission_for_response(
        &self,
        session_id: &str,
        response: &PermissionResponse,
    ) -> Result<PendingPermissionForResponse, AgentRuntimeError> {
        {
            let sessions = self.ctx.sessions.lock().await;
            if let Some((pending, turn_id)) = sessions.get(session_id).and_then(|state| {
                state
                    .permission_request_cache
                    .as_ref()
                    .map(|pending| (pending.clone(), state.active_turn_id()))
            }) {
                if !permission_request_identity_matches(&pending.id, &response.request_id) {
                    return Err(AgentRuntimeError::Other(format!(
                        "Permission request id mismatch: pending={}, response={}",
                        pending.id, response.request_id
                    )));
                }
                return Ok(PendingPermissionForResponse {
                    turn_id,
                    #[cfg(test)]
                    from_runtime_state: true,
                });
            }
        }

        let Some(pending) = self.unresolved_permission_request_from_event_log(session_id) else {
            return Err(AgentRuntimeError::Other(format!(
                "No pending permission request for session {session_id}"
            )));
        };
        if !permission_request_identity_matches(&pending.request.id, &response.request_id) {
            return Err(AgentRuntimeError::Other(format!(
                "Permission request id mismatch: pending={}, response={}",
                pending.request.id, response.request_id
            )));
        }
        Ok(PendingPermissionForResponse {
            turn_id: Some(pending.turn_id),
            #[cfg(test)]
            from_runtime_state: false,
        })
    }

    #[cfg(test)]
    async fn recover_queued_turn_if_idle_without_runtime(&self, session_id: &str) {
        let should_drain = {
            let sessions = self.ctx.sessions.lock().await;
            sessions.get(session_id).is_some_and(|state| {
                !state.has_active_turn_lease() && !state.accepted_input_effects.is_empty()
            })
        };
        if should_drain {
            start_next_queued_turn(&self.ctx, session_id).await;
        }
    }

    pub async fn set_permission_mode(
        &self,
        session_id: &str,
        mode: PermissionMode,
    ) -> Result<(), AgentRuntimeError> {
        let _admission_guard = self.ctx.shutdown_admission.admit()?;
        let _session_guard = acquire_session_control_after_recovery(&self.ctx, session_id).await;
        ensure_backend_recovery_operation_allowed(&self.ctx, session_id)?;
        self.ctx
            .session_store
            .update_permission_mode_from_user(&self.ctx.data_dir, session_id, mode.as_str())
            .map_err(AgentRuntimeError::Other)?;
        self.ctx
            .notifier
            .permission_mode_changed(session_id, mode.as_str());
        Ok(())
    }

    pub async fn set_plan_mode(
        &self,
        session_id: &str,
        plan_mode: bool,
    ) -> Result<(), AgentRuntimeError> {
        let _admission_guard = self.ctx.shutdown_admission.admit()?;
        let _session_guard = acquire_session_control_after_recovery(&self.ctx, session_id).await;
        ensure_backend_recovery_operation_allowed(&self.ctx, session_id)?;
        self.ctx
            .session_store
            .update_plan_mode_from_user(&self.ctx.data_dir, session_id, plan_mode)
            .map_err(AgentRuntimeError::Other)?;
        Ok(())
    }

    pub async fn set_model(
        &self,
        session_id: &str,
        entry_id: &str,
    ) -> Result<(), AgentRuntimeError> {
        let _admission_guard = self.ctx.shutdown_admission.admit()?;
        let _session_guard = acquire_session_control_after_recovery(&self.ctx, session_id).await;
        ensure_backend_recovery_operation_allowed(&self.ctx, session_id)?;
        let entry = self
            .ctx
            .registry
            .resolve_model_entry(entry_id)
            .map_err(AgentRuntimeError::Other)?;
        let (session, _page, _) = self
            .ctx
            .session_store
            .get_session_with_latest_page(&self.ctx.data_dir, session_id, 1)
            .map_err(AgentRuntimeError::Other)?
            .ok_or_else(|| AgentRuntimeError::Other(format!("Session not found: {session_id}")))?;
        let backend_changes =
            backend_selection_changes(session.backend_id.as_deref(), &entry.backend);
        if backend_changes {
            let lifecycle_repository = self.ctx.lifecycle_repository();
            if lifecycle_repository.is_none() {
                #[cfg(not(test))]
                return Err(AgentRuntimeError::Other(
                    "agent-session lifecycle repository is not configured".to_string(),
                ));
            }
            let selection_is_unlocked = if let Some(repository) = lifecycle_repository {
                repository
                    .restore_session(session_id)
                    .await
                    .map_err(|error| {
                        AgentRuntimeError::Other(format!(
                            "failed to restore backend-switch aggregate: {error:?}"
                        ))
                    })?
                    .admit_backend_selection_change()
                    .is_ok()
            } else {
                #[cfg(test)]
                {
                    let sessions = self.ctx.sessions.lock().await;
                    let runtime = sessions.get(session_id);
                    backend_selection_change_is_admitted(BackendSelectionChangeFacts {
                        has_messages: !session.messages.is_empty(),
                        has_provider_session: session.agent_session_id.is_some(),
                        turn_phase: runtime
                            .map(RuntimeSessionState::projected_turn_phase)
                            .unwrap_or(TurnPhase::Idle),
                        has_pending_permission: runtime
                            .is_some_and(|state| state.permission_request_cache.is_some()),
                        has_accepted_effects: runtime
                            .is_some_and(|state| !state.accepted_input_effects.is_empty()),
                        has_backend_recovery: runtime
                            .is_some_and(|state| state.backend_recovery.is_some()),
                    })
                }
                #[cfg(not(test))]
                false
            };
            if !selection_is_unlocked {
                return Err(AgentRuntimeError::BackendSelectionLocked);
            }
        }
        self.ctx
            .session_store
            .update_backend_selection_from_user(
                &self.ctx.data_dir,
                session_id,
                entry.backend.clone(),
                Some(entry.model_id.clone()),
            )
            .map_err(AgentRuntimeError::Other)?;
        if backend_changes {
            self.close_session_runtime_locked(session_id).await;
        }
        if let Ok(available_models) = self.ctx.registry.available_models(&entry.backend) {
            self.ctx
                .notifier
                .models_updated(session_id, available_models, entry.model_id.clone());
        }
        Ok(())
    }

    #[cfg(test)]
    pub async fn set_session_backend(
        &self,
        session_id: &str,
        backend_id: &str,
    ) -> Result<GetSessionResponse, AgentRuntimeError> {
        let selected_model = self
            .ctx
            .registry
            .default_model_for(backend_id)
            .map_err(AgentRuntimeError::Other)?;
        self.run_session_close(session_id, || async {
            ensure_backend_recovery_operation_allowed(&self.ctx, session_id)?;
            self.ctx
                .session_store
                .update_backend_selection(
                    &self.ctx.data_dir,
                    session_id,
                    backend_id.to_string(),
                    Some(selected_model),
                )
                .map_err(AgentRuntimeError::Other)
        })
        .await?;
        self.get_session(session_id)
            .await?
            .ok_or_else(|| AgentRuntimeError::Other(format!("Session not found: {session_id}")))
    }

    /// Waits for an in-flight backend recovery and then closes the live runtime.
    ///
    /// This is the normal teardown entry point. It may also reconcile the durable
    /// event log before returning session control, including persisting an
    /// interrupted recovery failure and publishing its user-facing error part.
    pub async fn close_session(&self, session_id: &str) -> Result<(), AgentRuntimeError> {
        self.run_session_close(session_id, || async { Ok(()) })
            .await?;
        Ok(())
    }

    async fn run_session_close<T, F, Fut>(
        &self,
        session_id: &str,
        after_finish: F,
    ) -> Result<T, AgentRuntimeError>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<T, AgentRuntimeError>>,
    {
        let session_guard = acquire_session_control_after_recovery(&self.ctx, session_id).await;
        let should_drain = self.begin_session_close_locked(session_id).await?;
        drop(session_guard);
        if should_drain {
            self.drain_closing_turn(session_id).await;
        }
        let session_guard = acquire_session_runtime_lock(&self.ctx.session_locks, session_id).await;
        let workflow_notification = self.finalize_session_close_locked(session_id).await?;
        let output = match after_finish().await {
            Ok(output) => output,
            Err(error) => {
                if let Some(state) = self.ctx.sessions.lock().await.get_mut(session_id) {
                    state.cancel_closing();
                }
                drop(session_guard);
                if let Some(workflow_notification) = workflow_notification {
                    dispatch_workflow_turn_complete_notification(
                        &self.ctx.workflow_turn_complete_notifier,
                        workflow_notification,
                    )
                    .await;
                }
                return Err(error);
            }
        };
        self.close_session_runtime_locked(session_id).await;
        drop(session_guard);
        if let Some(workflow_notification) = workflow_notification {
            dispatch_workflow_turn_complete_notification(
                &self.ctx.workflow_turn_complete_notifier,
                workflow_notification,
            )
            .await;
        }
        Ok(output)
    }

    #[cfg(test)]
    pub async fn close_all(&self) -> Result<(), AgentRuntimeError> {
        self.ctx.shutdown_admission.begin_shutdown();
        self.ctx.shutdown_admission.wait_for_idle().await;
        let session_ids = {
            let sessions = self.ctx.sessions.lock().await;
            sessions.keys().cloned().collect::<Vec<_>>()
        };
        let results = futures_util::future::join_all(
            session_ids
                .iter()
                .map(|session_id| self.close_session(session_id)),
        )
        .await;
        let errors = session_ids
            .iter()
            .zip(results)
            .filter_map(|(session_id, result)| {
                result.err().map(|error| format!("{session_id}: {error}"))
            })
            .collect::<Vec<_>>();
        if errors.is_empty() {
            Ok(())
        } else {
            {
                let mut sessions = self.ctx.sessions.lock().await;
                for state in sessions.values_mut() {
                    state.cancel_closing();
                }
            }
            self.ctx.shutdown_admission.cancel_shutdown();
            Err(AgentRuntimeError::Other(format!(
                "Failed to close agent sessions: {}",
                errors.join("; ")
            )))
        }
    }

    pub(crate) fn application_shutdown_target_session_ids(
        &self,
    ) -> Result<Vec<String>, AgentRuntimeError> {
        self.ctx
            .session_store
            .application_shutdown_target_session_ids(&self.ctx.data_dir)
            .map_err(AgentRuntimeError::Other)
    }

    async fn begin_session_close_locked(
        &self,
        session_id: &str,
    ) -> Result<bool, AgentRuntimeError> {
        let should_finalize = {
            let mut sessions = self.ctx.sessions.lock().await;
            let Some(state) = sessions.get_mut(session_id) else {
                return Ok(false);
            };
            state.begin_closing();
            state.has_active_turn_lease()
        };
        if should_finalize {
            flush_streaming_update(&self.ctx, session_id, true)
                .await
                .map_err(AgentRuntimeError::Other)?;
        }
        Ok(should_finalize)
    }

    async fn finalize_session_close_locked(
        &self,
        session_id: &str,
    ) -> Result<Option<WorkflowTurnCompleteNotification>, AgentRuntimeError> {
        let should_finalize = {
            let sessions = self.ctx.sessions.lock().await;
            sessions
                .get(session_id)
                .is_some_and(|state| state.has_active_turn_lease())
        };
        let workflow_notification = if should_finalize {
            complete_turn(
                &self.ctx,
                session_id,
                None,
                TurnResult::Interrupted {
                    reason: DomainInterruptReason::SessionClosed,
                    error: None,
                },
            )
            .await
            .map_err(AgentRuntimeError::Other)?
        } else {
            None
        };
        Ok(workflow_notification)
    }

    async fn close_session_runtime_locked(&self, session_id: &str) {
        let runtime = {
            let sessions = self.ctx.sessions.lock().await;
            sessions
                .get(session_id)
                .and_then(|state| state.runtime.clone())
        };
        if let Some(runtime) = runtime {
            runtime.close().await;
        }
        self.ctx.sessions.lock().await.remove(session_id);
    }

    /// Closes the live runtime immediately without waiting for backend recovery.
    ///
    /// This is reserved for lifecycle teardown paths where waiting for a provider
    /// establishment event could deadlock shutdown or node cleanup.
    pub(crate) async fn force_close_session(
        &self,
        session_id: &str,
    ) -> Result<(), AgentRuntimeError> {
        self.close_session_runtime_locked(session_id).await;
        Ok(())
    }

    async fn drain_closing_turn(&self, session_id: &str) {
        let deadline = tokio::time::Instant::now() + CLOSE_DRAIN_TIMEOUT;
        loop {
            let still_active = {
                let sessions = self.ctx.sessions.lock().await;
                sessions
                    .get(session_id)
                    .is_some_and(|state| state.has_active_turn_lease())
            };
            if !still_active {
                return;
            }

            let now = tokio::time::Instant::now();
            if now >= deadline {
                return;
            }
            tokio::time::sleep(CLOSE_DRAIN_POLL_INTERVAL.min(deadline - now)).await;
        }
    }

    pub async fn find_permission_request(
        &self,
        session_id: &str,
        request_id: &str,
    ) -> Result<Option<PermissionRequestMsg>, AgentRuntimeError> {
        {
            let sessions = self.ctx.sessions.lock().await;
            if let Some(state) = sessions.get(session_id) {
                if let Some(request) = state
                    .permission_request_cache
                    .as_ref()
                    .filter(|request| request.id == request_id)
                    .cloned()
                {
                    return Ok(Some(request));
                }
                if let Some(request) =
                    permission_request_from_parts(
                        self.ctx.projection_gateway.as_ref(),
                        state.persisted_streaming_parts(),
                        request_id,
                    )
                {
                    return Ok(Some(request));
                }
            }
        }

        let mut cursor = None;
        while let Some(page) = self
            .ctx
            .session_store
            .get_session_page(
                &self.ctx.data_dir,
                session_id,
                cursor.clone(),
                INITIAL_SESSION_PAGE_LIMIT,
            )
            .map_err(AgentRuntimeError::Other)?
        {
            if let Some(request) = page
                .messages
                .iter()
                .rev()
                .filter_map(|message| message.parts.as_deref())
                .find_map(|parts| {
                    permission_request_from_parts(
                        self.ctx.projection_gateway.as_ref(),
                        parts,
                        request_id,
                    )
                })
            {
                return Ok(Some(request));
            }
            if !page.has_more {
                break;
            }
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        Ok(self
            .unresolved_permission_request_from_event_log(session_id)
            .filter(|pending| pending.request.id == request_id)
            .map(|pending| {
                self.ctx
                    .projection_gateway
                    .permission_request(&pending.request)
            }))
    }

    pub async fn cancel_queued_turn(
        &self,
        _session_id: &str,
        _queued_turn_id: Option<&str>,
    ) -> Result<CancelQueuedTurnResponse, AgentRuntimeError> {
        Err(AgentRuntimeError::Other(
            "Queued turn cancellation is unavailable until it has an atomic durable queue operation"
                .to_string(),
        ))
    }

    pub async fn get_session(
        &self,
        session_id: &str,
    ) -> Result<Option<GetSessionResponse>, AgentRuntimeError> {
        self.get_session_with_message_limit(session_id, INITIAL_SESSION_PAGE_LIMIT, false)
            .await
    }

    pub async fn get_display_session_window(
        &self,
        session_id: &str,
        visible_message_count: Option<usize>,
    ) -> Result<Option<GetSessionResponse>, AgentRuntimeError> {
        let message_limit = visible_message_count
            .unwrap_or(INITIAL_SESSION_PAGE_LIMIT)
            .clamp(INITIAL_SESSION_PAGE_LIMIT, RETAINED_MESSAGE_CAP);
        self.get_session_with_message_limit(session_id, message_limit, true)
            .await
    }

    async fn get_session_with_message_limit(
        &self,
        session_id: &str,
        message_limit: usize,
        overlay_live_streaming: bool,
    ) -> Result<Option<GetSessionResponse>, AgentRuntimeError> {
        let _session_guard = acquire_session_control_after_recovery(&self.ctx, session_id).await;
        let Some((mut session, page, last_turn_interruption)) = self
            .ctx
            .session_store
            .get_session_with_latest_page(&self.ctx.data_dir, session_id, message_limit)
            .map_err(AgentRuntimeError::Other)?
        else {
            return Ok(None);
        };
        let backend_id = required_backend_id(&session)?;
        let durable_queue_paused_at = self
            .ctx
            .session_store
            .load_queue_paused_at(&self.ctx.data_dir, session_id)
            .map_err(AgentRuntimeError::Other)?;
        let (
            mut turn_phase,
            pending_queue,
            queue_paused,
            latest_token_usage,
            pending_permission_request,
            pending_permission_state_revision,
            active_turn_id,
            streaming_message,
        ) = {
            let mut sessions = self.ctx.sessions.lock().await;
            let state = sessions.entry(session_id.to_string()).or_insert_with(|| {
                RuntimeSessionState::with_queue_pause(backend_id, durable_queue_paused_at)
            });
            (
                state.projected_turn_phase(),
                pending_queue_view(state),
                state.queue_is_paused(),
                state.latest_token_usage,
                (state.runtime.is_some() && state.permission_request_cache.is_some())
                    .then(|| state.permission_request_cache.clone())
                    .flatten(),
                state.pending_permission_state_revision(),
                (state.has_active_turn_lease())
                    .then_some(state.active_turn_id())
                    .flatten(),
                overlay_live_streaming
                    .then(|| {
                        state.streaming_message_id.as_ref().map(|message_id| {
                            (
                                message_id.clone(),
                                state.persisted_streaming_parts().to_vec(),
                                state.stream_sequence(),
                            )
                        })
                    })
                    .flatten(),
            )
        };
        if let Some((message_id, parts, streaming_final_seq)) = streaming_message {
            if let Some(message) = session
                .messages
                .iter_mut()
                .find(|message| message.id == message_id)
            {
                message.parts = Some(parts);
                message.streaming_final_seq = message.streaming_final_seq.max(streaming_final_seq);
            }
        }
        if pending_permission_request.is_some() {
            turn_phase = TurnPhase::WaitingPermission;
        }
        let available_models = self.available_models_for_session(&session)?;
        let total_count = page.total_count;
        let can_change_backend = backend_selection_is_presented_as_changeable(
            !session.messages.is_empty(),
            session.agent_session_id.is_some(),
            turn_phase,
        );
        let session_meta = self
            .ctx
            .session_store
            .get_session_meta(&self.ctx.data_dir, session_id)
            .map_err(AgentRuntimeError::Other)?
            .ok_or_else(|| AgentRuntimeError::Other(format!("Session not found: {session_id}")))?;
        let response = GetSessionResponse {
            session,
            session_revision: session_meta.state_revision,
            active_turn_id,
            turn_phase,
            available_models,
            can_change_backend,
            pending_queue_count: pending_queue.len(),
            pending_queue,
            queue_paused,
            pending_permission_request,
            pending_permission_state_revision,
            initial_page: Some(InitialSessionPage {
                next_cursor: page.next_cursor,
                has_more: page.has_more,
                total_count,
            }),
            latest_token_usage: latest_token_usage.or(page.latest_token_usage),
            last_turn_interruption,
        };
        // This method still owns the per-session runtime lock acquired above. Publish the
        // bounded window before releasing it so every later runtime/state event is ordered
        // after this snapshot instead of being overwritten by a delayed command response.
        if overlay_live_streaming && !self.ctx.notifier.display_window_updated(&response) {
            return Err(AgentRuntimeError::Other(
                "failed to publish agent session display window".to_string(),
            ));
        }
        Ok(Some(response))
    }

    fn unresolved_permission_request_from_event_log(
        &self,
        session_id: &str,
    ) -> Option<UnresolvedPermissionRequest> {
        let events = match self
            .ctx
            .session_store
            .load_session_events(&self.ctx.data_dir, session_id)
        {
            Ok(events) => events,
            Err(error) => {
                log::warn!(
                    "failed to load session events for unresolved permission lookup {session_id}: {error}"
                );
                return None;
            }
        };
        latest_unresolved_permission_request(&events)
    }

    pub async fn init_sessions(
        &self,
        worktree_path: &str,
        open_tabs: &OpenTabRegistry,
    ) -> Result<InitSessionsResponse, AgentRuntimeError> {
        let sessions = self.list_sessions(worktree_path).await?;
        for session in &sessions {
            if session.is_workflow_node_session() && !session.state.is_closed() {
                open_tabs.add(&session.id);
            }
        }
        let active_candidate = sessions
            .iter()
            .find(|session| !session.is_workflow_node_session())
            .map(|session| session.id.clone());
        let active_mode = active_candidate.as_deref().and_then(|session_id| {
            sessions
                .iter()
                .find(|session| session.id == session_id)
                .map(|session| (session.permission_mode.clone(), session.plan_mode))
        });
        let active_session = match active_candidate.as_deref() {
            Some(session_id) => self.get_session(session_id).await?,
            None => None,
        };
        let (permission_mode, plan_mode) = active_mode
            .or_else(|| {
                active_session.as_ref().map(|session| {
                    (
                        session.session.permission_mode.clone(),
                        session.session.plan_mode,
                    )
                })
            })
            .unwrap_or_else(|| (PermissionMode::Edit.as_str().to_string(), false));
        Ok(InitSessionsResponse {
            sessions,
            active_session,
            permission_mode,
            plan_mode,
        })
    }

    pub async fn has_live_runtime(&self, session_id: &str) -> bool {
        self.live_runtime(session_id).await.is_some()
    }

    async fn ensure_session_not_closing(&self, session_id: &str) -> Result<(), AgentRuntimeError> {
        let sessions = self.ctx.sessions.lock().await;
        if sessions
            .get(session_id)
            .is_some_and(|state| !state.accepts_work())
        {
            return Err(AgentRuntimeError::Other(format!(
                "Agent session is closing: {session_id}"
            )));
        }
        Ok(())
    }

    /// Acquires the per-session runtime lock.
    ///
    /// While the returned guard is held, callers must not acquire another session runtime lock,
    /// including the same session recursively. Backend I/O awaits such as process startup and
    /// stdin writes must be limited to the smallest range required for per-session ordering.
    /// UI and event notifications, including session state-change emits, must run after the guard
    /// is dropped.
    #[cfg(test)]
    pub async fn acquire_session_lock(&self, session_id: &str) -> SessionCommandLockGuard {
        self.ctx.session_locks.acquire(session_id).await
    }

    /// Waits for backend recovery and acquires exclusive session control.
    ///
    /// Besides waiting, this projects the complete durable event log. If recovery
    /// was interrupted, it persists a Failed marker, moves the session to Error,
    /// and publishes the user-facing recovery Error part before returning.
    pub async fn acquire_session_control_after_recovery(
        &self,
        session_id: &str,
    ) -> SessionRuntimeLockGuard {
        acquire_session_control_after_recovery(&self.ctx, session_id).await
    }

    pub async fn list_sessions(
        &self,
        worktree_path: &str,
    ) -> Result<Vec<SessionSummary>, AgentRuntimeError> {
        self.workspace_session_summaries(
            worktree_path,
            crate::domain::workspace_tree::WorkspaceSessionListKind::Active,
        )
        .await
    }

    pub async fn list_closed_sessions(
        &self,
        worktree_path: &str,
    ) -> Result<Vec<SessionSummary>, AgentRuntimeError> {
        self.workspace_session_summaries(
            worktree_path,
            crate::domain::workspace_tree::WorkspaceSessionListKind::Closed,
        )
        .await
    }

    async fn workspace_session_summaries(
        &self,
        worktree_path: &str,
        list: crate::domain::workspace_tree::WorkspaceSessionListKind,
    ) -> Result<Vec<SessionSummary>, AgentRuntimeError> {
        let workspace_identity =
            crate::domain::workspace_tree::WorkspaceIdentity::new(worktree_path);
        let workspace_query = Arc::clone(&self.ctx.workspace_query);
        tokio::task::spawn_blocking(move || {
            workspace_query.session_summaries(&workspace_identity, list)
        })
        .await
        .map_err(|error| {
            AgentRuntimeError::Other(format!(
                "Workspace Session query worker failed to join: {error}"
            ))
        })?
        .map_err(AgentRuntimeError::WorkspaceQuery)
    }

    #[cfg(test)]
    pub(crate) fn session_runtime_lock_is_held_for_test(&self, session_id: &str) -> bool {
        self.ctx.session_locks.is_held_for_test(session_id)
    }

    pub(crate) async fn start_workflow_turn_locked(
        &self,
        request: DurableWorkflowTurnRequest,
    ) -> Result<(), AgentRuntimeError> {
        let driver = self
            .ctx
            .durable_workflow_send_driver
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        if let Some(driver) = driver {
            return driver
                .send(request)
                .await
                .map_err(AgentRuntimeError::WorkflowTurnSend);
        }

        #[cfg(test)]
        return self
            .start_turn_locked(
                &request.session_id,
                request.permission_mode,
                request.content,
                request.base_system_prompt,
                request.workflow_instructions,
            )
            .await;

        #[cfg(not(test))]
        Err(AgentRuntimeError::WorkflowTurnSend(
            DurableWorkflowSendError::DriverUnavailable,
        ))
    }

    #[cfg(test)]
    pub async fn start_turn_locked(
        &self,
        session_id: &str,
        permission_mode: PermissionMode,
        prompt: String,
        base_system_prompt: Option<String>,
        workflow_instructions: Vec<String>,
    ) -> Result<(), AgentRuntimeError> {
        let _admission_guard = self.ctx.shutdown_admission.admit()?;
        ensure_backend_recovery_operation_allowed(&self.ctx, session_id)?;
        self.ensure_session_not_closing(session_id).await?;
        let mut session = self
            .ctx
            .session_store
            .get_session_shell(&self.ctx.data_dir, session_id)
            .map_err(AgentRuntimeError::Other)?
            .ok_or_else(|| AgentRuntimeError::Other(format!("Session not found: {session_id}")))?;
        let queue_transition_guard = self.ctx.transitions.acquire(session_id).await;
        self.hydrate_runtime_session_state(&session).await?;
        let queue_paused = {
            let sessions = self.ctx.sessions.lock().await;
            sessions
                .get(session_id)
                .is_some_and(|state| state.queue_is_paused())
        };
        if queue_paused {
            return Err(AgentRuntimeError::Other(format!(
                "Agent queue is paused for session {session_id}; resume it before starting a workflow turn"
            )));
        }
        if session.permission_mode != permission_mode.as_str() {
            self.ctx
                .session_store
                .update_permission_mode(&self.ctx.data_dir, session_id, permission_mode.as_str())
                .map_err(AgentRuntimeError::Other)?;
            session.permission_mode = permission_mode.as_str().to_string();
        }

        let human_message = add_message_internal(
            &self.ctx.session_store,
            &self.ctx.data_dir,
            session_id,
            MessageRole::Human,
            &prompt,
            None,
            None,
        )
        .map_err(AgentRuntimeError::Other)?;
        let agent_message = add_message_internal(
            &self.ctx.session_store,
            &self.ctx.data_dir,
            session_id,
            MessageRole::Agent,
            "",
            None,
            None,
        )
        .map_err(AgentRuntimeError::Other)?;
        self.ctx
            .notifier
            .turn_prepared(&session, &human_message, &agent_message);

        let system_prompt = self.build_turn_system_prompt(
            &session,
            base_system_prompt,
            &[],
            None,
            workflow_instructions,
        )?;
        self.start_turn_for_session(
            &session,
            &human_message,
            agent_message.id,
            TurnStartPayload {
                prompt,
                images: Vec::new(),
                mentions: Vec::new(),
                permission_mode,
                plan_mode: session.plan_mode,
                permission_profile_id: session.permission_profile_id.clone(),
                editor_context: None,
                system_prompt,
                accepted_execution_identity: None,
            },
            Some(queue_transition_guard),
            None,
        )
        .await
    }

    pub async fn turn_phase(&self, session_id: &str) -> Option<TurnPhase> {
        let sessions = self.ctx.sessions.lock().await;
        sessions
            .get(session_id)
            .map(|state| state.projected_turn_phase())
    }

    pub async fn streaming_parts(&self, session_id: &str) -> Vec<MessagePart> {
        let sessions = self.ctx.sessions.lock().await;
        sessions
            .get(session_id)
            .map(|state| state.persisted_streaming_parts().to_vec())
            .unwrap_or_default()
    }

    pub async fn build_agent_task_list_report(
        &self,
        session_id: &str,
    ) -> Result<crate::usecase::agent_session::session::AgentTaskListReport, AgentRuntimeError>
    {
        let mut parts = self
            .ctx
            .session_store
            .load_full_session_for_restore(&self.ctx.data_dir, session_id)
            .map_err(AgentRuntimeError::Other)?
            .map(|session| {
                session
                    .messages
                    .into_iter()
                    .filter_map(|message| message.parts)
                    .flatten()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        parts.extend(self.streaming_parts(session_id).await);
        Ok(crate::usecase::agent_session::session::build_agent_task_list_report_from_parts(&parts))
    }

    #[cfg(test)]
    pub(crate) async fn insert_runtime_state_for_test(
        &self,
        session_id: &str,
        phase: TurnPhase,
        queued: bool,
    ) {
        let mut sessions = self.ctx.sessions.lock().await;
        let state = sessions
            .entry(session_id.to_string())
            .or_insert_with(|| RuntimeSessionState::new("claude".to_string()));
        state.runtime = Some(Arc::new(TestNoopAgentRuntime));
        state.install_turn_lease_for_test(phase);
        if queued {
            let queued = QueuedTurnInput::new(
                "queued".to_string(),
                PermissionMode::Edit,
                false,
                None,
                Vec::new(),
                "/repo".to_string(),
                Vec::new(),
                None,
            );
            state.accepted_input_effects.insert(queued.id.clone(), queued);
        }
    }

    #[cfg(test)]
    pub(crate) async fn insert_failing_runtime_state_for_test(&self, session_id: &str) {
        let mut sessions = self.ctx.sessions.lock().await;
        let state = sessions
            .entry(session_id.to_string())
            .or_insert_with(|| RuntimeSessionState::new("claude".to_string()));
        state.runtime = Some(Arc::new(TestFailingAgentRuntime));
        state.release_turn_lease();
    }

    pub(crate) async fn drain_accepted_queue_if_idle(
        &self,
        session_id: &str,
    ) -> Result<AcceptedQueueDrainOutcome, AgentRuntimeError> {
        let _session_guard = self.ctx.session_locks.acquire(session_id).await;
        let canonical_queue = match self
            .ctx
            .session_store
            .canonical_pending_send_queue(session_id)
        {
            Ok(queue) => queue,
            Err(error) => {
                #[cfg(not(test))]
                return Err(AgentRuntimeError::Other(error));
                #[cfg(test)]
                {
                    let _ = error;
                    Vec::new()
                }
            }
        };
        let (front_requires_durable_idle, runtime_ready, accepted_identity, queue_item_id) = {
            let sessions = self.ctx.sessions.lock().await;
            let Some(state) = sessions.get(session_id) else {
                return Ok(AcceptedQueueDrainOutcome::NoWork);
            };
            let Some(front) =
                next_cached_input_effect(&state.accepted_input_effects, &canonical_queue)
            else {
                return Ok(AcceptedQueueDrainOutcome::NoWork);
            };
            if !state.accepts_work() {
                return Ok(AcceptedQueueDrainOutcome::NoWork);
            }
            (
                queued_turn_has_accepted_identity(front),
                state.admits_queue_drain(),
                front
                    .accepted_operation_id
                    .as_ref()
                    .zip(front.execution_obligation_id.as_ref())
                    .map(|(operation_id, obligation_id)| {
                        (operation_id.clone(), obligation_id.clone())
                    }),
                front.id.clone(),
            )
        };
        if !runtime_ready {
            let Some((operation_id, obligation_id)) = accepted_identity else {
                return Ok(AcceptedQueueDrainOutcome::Blocked);
            };
            return Ok(
                match self
                    .accepted_queue_redrive_readiness(session_id, &operation_id, &obligation_id)
                    .await
                {
                    AcceptedQueueRedriveReadiness::Blocked => AcceptedQueueDrainOutcome::Blocked,
                    // The canonical state already unblocked, but its local
                    // mirror has not caught up. Keep one bounded redriver alive
                    // across that projection-to-memory handoff.
                    AcceptedQueueRedriveReadiness::Ready => AcceptedQueueDrainOutcome::Attempted,
                    AcceptedQueueRedriveReadiness::Missing => AcceptedQueueDrainOutcome::NoWork,
                },
            );
        }
        match self
            .ctx
            .session_store
            .load_queue_paused_at(&self.ctx.data_dir, session_id)
        {
            Ok(Some(_)) => return Ok(AcceptedQueueDrainOutcome::Blocked),
            Ok(None) => {}
            Err(error) => return Err(AgentRuntimeError::Other(error)),
        }
        if let Err(failure) = self
            .ctx
            .session_store
            .ensure_no_unresolved_recovery(session_id)
            .await
        {
            if failure.kind
                == crate::domain::local_event::SessionOperationFailureKind::StorageUnavailable
            {
                return Err(AgentRuntimeError::Other(failure.to_string()));
            }
            return Ok(AcceptedQueueDrainOutcome::Blocked);
        }
        if front_requires_durable_idle {
            let readiness = self
                .ctx
                .session_store
                .accepted_queue_start_readiness(&self.ctx.data_dir, session_id)
                .map_err(AgentRuntimeError::Other)?;
            match readiness {
                Some(true) => {}
                Some(false) => return Ok(AcceptedQueueDrainOutcome::Blocked),
                None => {
                    return Err(AgentRuntimeError::Other(format!(
                        "Session not found: {session_id}"
                    )));
                }
            }
        }
        start_next_queued_turn(&self.ctx, session_id).await;
        let queue_item_remains = {
            let sessions = self.ctx.sessions.lock().await;
            sessions.get(session_id).is_some_and(|state| {
                queued_effect_remains_unstarted(
                    state.has_active_turn_lease(),
                    state.accepted_input_effects.contains_key(&queue_item_id),
                )
            })
        };
        if !queue_item_remains {
            return Ok(AcceptedQueueDrainOutcome::Attempted);
        }
        let Some((operation_id, obligation_id)) = accepted_identity else {
            return Ok(AcceptedQueueDrainOutcome::Blocked);
        };
        Ok(
            match self
                .accepted_queue_redrive_readiness(session_id, &operation_id, &obligation_id)
                .await
            {
                AcceptedQueueRedriveReadiness::Ready => AcceptedQueueDrainOutcome::Attempted,
                AcceptedQueueRedriveReadiness::Blocked => AcceptedQueueDrainOutcome::Blocked,
                AcceptedQueueRedriveReadiness::Missing => AcceptedQueueDrainOutcome::NoWork,
            },
        )
    }

    pub(crate) async fn accepted_queue_redrive_readiness(
        &self,
        session_id: &str,
        operation_id: &str,
        obligation_id: &str,
    ) -> AcceptedQueueRedriveReadiness {
        let canonical_queue = self
            .ctx
            .session_store
            .canonical_pending_send_queue(session_id).unwrap_or_default();
        let requires_durable_idle = {
            let sessions = self.ctx.sessions.lock().await;
            let Some(state) = sessions.get(session_id) else {
                return AcceptedQueueRedriveReadiness::Missing;
            };
            let Some(effect) = state.accepted_input_effects.values().find(|queued| {
                accepted_effect_execution_matches(
                    queued.accepted_operation_id.as_deref(),
                    queued.execution_obligation_id.as_deref(),
                    operation_id,
                    obligation_id,
                )
            }) else {
                return AcceptedQueueRedriveReadiness::Missing;
            };
            let is_canonical_head =
                next_cached_input_effect(&state.accepted_input_effects, &canonical_queue)
                    .is_some_and(|head| queue_item_identity_matches(&head.id, &effect.id));
            if !accepted_effect_delivery_is_admitted(
                state.is_closing(),
                state.backend_recovery.is_some(),
                is_canonical_head,
            ) {
                return AcceptedQueueRedriveReadiness::Blocked;
            }
            queued_turn_has_accepted_identity(effect)
        };
        if !requires_durable_idle {
            return AcceptedQueueRedriveReadiness::Ready;
        }
        match self
            .ctx
            .session_store
            .load_queue_paused_at(&self.ctx.data_dir, session_id)
        {
            Ok(Some(_)) => return AcceptedQueueRedriveReadiness::Blocked,
            Ok(None) => {}
            Err(_) => return AcceptedQueueRedriveReadiness::Ready,
        }
        match self
            .ctx
            .session_store
            .accepted_queue_start_readiness(&self.ctx.data_dir, session_id)
        {
            Ok(Some(false)) => return AcceptedQueueRedriveReadiness::Blocked,
            // A missing or temporarily unreadable projection must enter the
            // bounded redriver. It will either recover the exact local item or
            // retire/reconcile it; treating the read error as owned forever
            // would strand the accepted obligation without another signal.
            Ok(None) | Err(_) => return AcceptedQueueRedriveReadiness::Ready,
            Ok(Some(true)) => {}
        }
        if let Err(failure) = self
            .ctx
            .session_store
            .ensure_no_unresolved_recovery(session_id)
            .await
        {
            return if failure.kind
                == crate::domain::local_event::SessionOperationFailureKind::StorageUnavailable
            {
                AcceptedQueueRedriveReadiness::Ready
            } else {
                AcceptedQueueRedriveReadiness::Blocked
            };
        }
        AcceptedQueueRedriveReadiness::Ready
    }

    #[cfg(test)]
    pub(crate) async fn drain_next_queued_turn_for_test(&self, session_id: &str) {
        let _session_guard = self.ctx.session_locks.acquire(session_id).await;
        start_next_queued_turn(&self.ctx, session_id).await;
    }

    #[cfg(test)]
    pub(crate) async fn prepare_queued_runtime_reopen_for_test(&self, session_id: &str) {
        let mut sessions = self.ctx.sessions.lock().await;
        let state = sessions
            .get_mut(session_id)
            .expect("queued runtime state must exist");
        assert!(!state.accepted_input_effects.is_empty());
        state.runtime = None;
        state.release_turn_lease();
    }

    #[cfg(test)]
    pub(crate) async fn stream_emit_failure_state_for_test(
        &self,
        session_id: &str,
    ) -> Option<(u32, bool)> {
        let sessions = self.ctx.sessions.lock().await;
        sessions.get(session_id).map(|state| {
            (
                state.stream_emit_failure_count(),
                state.stream_emit_is_suppressed(),
            )
        })
    }

    pub async fn skill_catalog(
        &self,
        backend_id: Option<&str>,
        cwd: &Path,
        query: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Vec<crate::domain::agent_session::value_objects::SkillEntry>, AgentRuntimeError>
    {
        let backend = self
            .ctx
            .registry
            .backend_for_optional_id(backend_id)
            .map_err(AgentRuntimeError::Other)?;
        backend
            .skill_catalog(cwd, query, limit)
            .await
            .map_err(AgentRuntimeError::from)
    }

    pub async fn mentionable_files(
        &self,
        backend_id: Option<&str>,
        root: &Path,
        query: &str,
        limit: usize,
    ) -> Result<Option<Vec<String>>, AgentRuntimeError> {
        let backend = self
            .ctx
            .registry
            .backend_for_optional_id(backend_id)
            .map_err(AgentRuntimeError::Other)?;
        backend
            .fuzzy_file_search(root, query, limit)
            .await
            .map_err(AgentRuntimeError::from)
    }

    #[cfg(test)]
    async fn resolve_or_create_session(
        &self,
        req: &SendAgentMessageRequest,
        reserved_session_id: Option<&str>,
    ) -> Result<ChatSession, AgentRuntimeError> {
        if let Some(session_id) = req.chat_session_id.as_deref() {
            ensure_backend_recovery_operation_allowed(&self.ctx, session_id)?;
            let mut session = self
                .ctx
                .session_store
                .get_session_shell(&self.ctx.data_dir, session_id)
                .map_err(AgentRuntimeError::Other)?
                .ok_or_else(|| {
                    AgentRuntimeError::Other(format!("Session not found: {session_id}"))
                })?;
            let backend_recovery_in_progress = {
                let sessions = self.ctx.sessions.lock().await;
                sessions
                    .get(session_id)
                    .is_some_and(|state| state.backend_recovery.is_some())
            };
            if should_apply_session_configuration(
                backend_recovery_in_progress,
                &session.permission_mode.as_str(),
                &req.permission_mode.as_str(),
            ) {
                self.ctx
                    .session_store
                    .update_permission_mode(
                        &self.ctx.data_dir,
                        session_id,
                        req.permission_mode.as_str(),
                    )
                    .map_err(AgentRuntimeError::Other)?;
                session.permission_mode = req.permission_mode.as_str().to_string();
            }
            if should_apply_session_configuration(
                backend_recovery_in_progress,
                &session.plan_mode,
                &req.plan_mode,
            ) {
                self.ctx
                    .session_store
                    .update_plan_mode(&self.ctx.data_dir, session_id, req.plan_mode)
                    .map_err(AgentRuntimeError::Other)?;
                session.plan_mode = req.plan_mode;
            }
            return Ok(session);
        }

        let resolved_model = match req.model_id.as_deref() {
            Some(model_id) => Some(
                self.ctx
                    .registry
                    .resolve_model_entry(model_id)
                    .map_err(AgentRuntimeError::Other)?,
            ),
            None => None,
        };
        let requested_backend = resolved_model
            .as_ref()
            .map(|model| model.backend.clone())
            .or(req.backend_id.clone());
        let backend_id = self
            .ctx
            .registry
            .resolve_backend_id(requested_backend)
            .map_err(AgentRuntimeError::Other)?;
        if let Some(session_id) = reserved_session_id {
            let selected_model = match resolved_model {
                Some(model) => model.model_id,
                None => self
                    .ctx
                    .registry
                    .default_model_for(&backend_id)
                    .map_err(AgentRuntimeError::Other)?,
            };
            crate::usecase::agent_session::session::create_session_with_resolved_options_and_id(
                &self.ctx.session_store,
                &self.ctx.data_dir,
                session_id.to_string(),
                &req.worktree_path,
                backend_id,
                req.permission_mode,
                selected_model,
                req.plan_mode,
            )
            .map_err(AgentRuntimeError::Other)
        } else {
            create_session_with_model_and_plan_mode(
                &self.ctx.session_store,
                &self.ctx.registry,
                &self.ctx.data_dir,
                &req.worktree_path,
                backend_id,
                req.permission_mode,
                resolved_model.map(|model| model.model_id),
                req.plan_mode,
            )
            .map_err(AgentRuntimeError::Other)
        }
    }

    async fn start_turn_for_session(
        &self,
        session: &ChatSession,
        human_message: &ChatMessage,
        agent_message_id: String,
        mut payload: TurnStartPayload,
        queue_transition_guard: Option<SessionLockGuard>,
        committed_turn_id: Option<u64>,
    ) -> Result<(), AgentRuntimeError> {
        let accepted_execution = payload.accepted_execution_identity.is_some();
        let accepted_running_identity = payload.accepted_execution_identity.clone();
        let had_runtime = self.live_runtime(&session.id).await.is_some();
        let restore_policy =
            context_restore_policy_for_turn(&self.ctx, &session.id, &agent_message_id, had_runtime)
                .map_err(|error| {
                    classify_turn_preclaim_error(
                        accepted_execution,
                        "context-restore",
                        AgentRuntimeError::Other(error),
                    )
                })?;
        let context_was_reinjected =
            matches!(&restore_policy.plan, ContextRestorePlan::Reinject { .. });
        let clear_context_carry_after_start =
            !had_runtime && matches!(&restore_policy.plan, ContextRestorePlan::NoContext);
        let recovery_restore_required = restore_policy.recovery_restore_required;
        let expected_provider_session_generation =
            restore_policy.expected_provider_session_generation;
        let restore_plan = restore_policy.plan;
        let original_prompt = payload.prompt.clone();
        payload.prompt = apply_restore_prompt_prefix(payload.prompt, &restore_plan);
        let selected_model = had_runtime
            .then(|| selected_model_for_runtime(&self.ctx, session))
            .transpose()
            .map_err(|error| {
                classify_turn_preclaim_error(
                    accepted_execution,
                    "selected-model",
                    AgentRuntimeError::from(error),
                )
            })?;
        let turn_id = match committed_turn_id {
            Some(turn_id) => turn_id,
            None => next_turn_id(&self.ctx.session_store, &self.ctx.data_dir, &session.id)
                .map_err(|error| {
                    classify_turn_preclaim_error(
                        accepted_execution,
                        "turn-identity",
                        AgentRuntimeError::Other(error),
                    )
                })?,
        };
        let backend_id = required_backend_id(session)
            .map_err(|error| classify_turn_preclaim_error(accepted_execution, "backend", error))?;
        let prompt_message = self
            .ctx
            .session_store
            .load_previous_human_message_before_agent(
                &self.ctx.data_dir,
                &session.id,
                &agent_message_id,
            )
            .map_err(|error| {
                classify_turn_preclaim_error(
                    accepted_execution,
                    "prompt-message",
                    AgentRuntimeError::Other(error),
                )
            })?
            .unwrap_or_else(|| human_message.clone());
        let queue_transition_guard = match queue_transition_guard {
            Some(guard) => guard,
            None => self.ctx.transitions.acquire(&session.id).await,
        };
        let queue_paused_at = self
            .ctx
            .session_store
            .load_queue_paused_at(&self.ctx.data_dir, &session.id)
            .map_err(|error| {
                classify_turn_preclaim_error(
                    accepted_execution,
                    "queue-pause",
                    AgentRuntimeError::Other(error),
                )
            })?;
        let local_queue_paused = self
            .ctx
            .sessions
            .lock()
            .await
            .get(&session.id)
            .is_some_and(|state| state.queue_is_paused());
        let queue_is_blocked =
            crate::domain::agent_session::aggregates::runtime_queue::RuntimeQueuePause::blocked_by_durable_or_local(
                queue_paused_at.is_some(),
                local_queue_paused,
            );
        if queue_is_blocked && payload.accepted_execution_identity.is_some() {
            return Err(fail_accepted_effect_preflight(
                "queue-blocked-before-turn-claim",
                format!("the accepted send queue is paused for {}", session.id),
            ));
        }
        let accepted_claim = if let Some(identity) = payload.accepted_execution_identity.as_ref() {
            let driver = self
                .ctx
                .accepted_send_obligation_driver
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone();
            match driver {
                Some(driver) => Some(
                    driver
                        .claim_immediate_turn_execution(
                            &identity.operation_id,
                            &identity.execution_obligation_id,
                        )
                        .await
                        .map_err(|()| AgentRuntimeError::AcceptedEffectAdmissionDeferred)?,
                ),
                None => {
                    #[cfg(test)]
                    {
                        Some(AcceptedSendExecutionClaim::new(|| {}))
                    }
                    #[cfg(not(test))]
                    {
                        return Err(fail_accepted_effect_preflight(
                            "turn-execution-driver",
                            "the accepted send obligation driver is unavailable",
                        ));
                    }
                }
            }
        } else {
            None
        };
        let mut current_turn_input = QueuedTurnInput::new(
            original_prompt,
            payload.permission_mode,
            payload.plan_mode,
            payload.permission_profile_id.clone(),
            payload.images.clone(),
            session.worktree_path.clone(),
            payload.mentions.clone(),
            payload.editor_context.clone(),
        );
        current_turn_input.existing_human_message_id = Some(human_message.id.clone());
        current_turn_input.existing_agent_message_id = Some(agent_message_id.clone());
        if let Some(identity) = payload.accepted_execution_identity.take() {
            current_turn_input.accepted_operation_id = Some(identity.operation_id);
            current_turn_input.execution_obligation_id = Some(identity.execution_obligation_id);
        }
        let generation = {
            let mut sessions = self.ctx.sessions.lock().await;
            let state = sessions.entry(session.id.clone()).or_insert_with(|| {
                RuntimeSessionState::with_queue_pause(backend_id.clone(), queue_paused_at)
            });
            if state.queue_is_paused() {
                return Err(AgentRuntimeError::Other(format!(
                    "Agent queue is paused for session {}; resume it before starting a turn",
                    session.id
                )));
            }
            let generation = state.register_turn_start_intent(turn_id, agent_message_id.clone());
            state.current_turn_input = Some(current_turn_input.clone());
            generation
        };
        drop(queue_transition_guard);
        let _accepted_claim = accepted_claim;
        if committed_turn_id.is_none() {
            if let Err(error) = self
                .ctx
                .session_store
                .append_turn_started_and_project_state(
                &self.ctx.data_dir,
                &session.id,
                AgentSessionEvent::TurnStarted {
                    turn_id,
                    message_id: prompt_message.id.clone(),
                    assistant_message_id: Some(agent_message_id.clone()),
                    prompt:
                        crate::usecase::agent_session::event_log::prompt_input_from_human_message(
                            &prompt_message,
                        ),
                    at: prompt_message.timestamp,
                },
            ) {
                let rollback_guard = self.ctx.transitions.acquire(&session.id).await;
                let mut sessions = self.ctx.sessions.lock().await;
                if let Some(state) = sessions.get_mut(&session.id) {
                    if state.should_rollback_start(generation) {
                        state.rollback_started_turn();
                    }
                }
                drop(sessions);
                drop(rollback_guard);
                return Err(AgentRuntimeError::Other(error));
            }
        }
        let commit_guard = self.ctx.transitions.acquire(&session.id).await;
        let (start_committed, interrupt_was_accepted) = {
            let mut sessions = self.ctx.sessions.lock().await;
            match sessions.get_mut(&session.id) {
                Some(state) => match state.decide_start_commit(generation, turn_id) {
                    RuntimeTurnStartCommit::Commit => {
                        state.commit_turn_start(agent_message_id.clone());
                        state.current_turn_input = Some(current_turn_input);
                        (true, false)
                    }
                    RuntimeTurnStartCommit::Interrupted => (false, true),
                    RuntimeTurnStartCommit::Paused | RuntimeTurnStartCommit::Superseded => {
                        (false, false)
                    }
                },
                _ => (false, false),
            }
        };
        drop(commit_guard);
        if interrupt_was_accepted {
            let (notification, _) = complete_turn_with_acceptance(
                &self.ctx,
                &session.id,
                Some(generation),
                TurnResult::Interrupted {
                    reason: DomainInterruptReason::Abort,
                    error: None,
                },
            )
            .await
            .map_err(AgentRuntimeError::Other)?;
            if let Some(notification) = notification {
                dispatch_workflow_turn_complete_notification(
                    &self.ctx.workflow_turn_complete_notifier,
                    notification,
                )
                .await;
            }
            return Ok(());
        }
        if !start_committed {
            return if accepted_execution {
                Err(AgentRuntimeError::Other(format!(
                    "accepted turn lost its live start ownership for session {}",
                    session.id
                )))
            } else {
                Ok(())
            };
        }
        let runtime_result = self
            .ensure_runtime_for_turn(session, payload.system_prompt.clone(), generation)
            .await;
        let runtime_result = match runtime_result {
            Err(AgentRuntimeError::BackendSessionLost { .. }) => {
                recover_backend_session(
                    &self.ctx,
                    &session.id,
                    BackendSessionRecoveryReason::BackendSessionLost,
                )
                .await?;
                if accepted_execution {
                    let retained = {
                        let sessions = self.ctx.sessions.lock().await;
                        sessions.get(&session.id).is_some_and(|state| {
                            state.has_active_turn_lease()
                                && state.current_turn_input.as_ref().is_some_and(|input| {
                                    input.accepted_operation_id.is_some()
                                        && input.execution_obligation_id.is_some()
                                })
                        })
                    };
                    if !retained {
                        return Err(AgentRuntimeError::Other(format!(
                            "accepted backend recovery failed for session {}",
                            session.id
                        )));
                    }
                }
                return Ok(());
            }
            result => result,
        };
        let runtime = {
            let _runtime_event_guard = self.ctx.runtime_event_locks.acquire(&session.id).await;
            let runtime = match runtime_result {
                Ok(runtime) => runtime,
                Err(error) => {
                    let should_report_failure = {
                        let sessions = self.ctx.sessions.lock().await;
                        sessions
                            .get(&session.id)
                            .is_some_and(|state| state.owns_generation(generation))
                    };
                    if !should_report_failure {
                        return if accepted_execution {
                            Err(AgentRuntimeError::Other(format!(
                                "accepted turn lost its runtime-open outcome for session {}",
                                session.id
                            )))
                        } else {
                            Ok(())
                        };
                    }
                    let message = error.to_string();
                    let (notification, interrupt_was_accepted) = complete_turn_with_acceptance(
                        &self.ctx,
                        &session.id,
                        Some(generation),
                        TurnResult::Interrupted {
                            reason: DomainInterruptReason::Crash,
                            error: Some(message),
                        },
                    )
                    .await
                    .map_err(AgentRuntimeError::Other)?;
                    if let Some(notification) = notification {
                        dispatch_workflow_turn_complete_notification(
                            &self.ctx.workflow_turn_complete_notifier,
                            notification,
                        )
                        .await;
                    }
                    return if interrupt_was_accepted {
                        Ok(())
                    } else {
                        Err(error)
                    };
                }
            };
            if !turn_owns_runtime(&self.ctx, &session.id, generation, &runtime).await {
                detach_runtime_if_current(&self.ctx, &session.id, &runtime).await;
                return if accepted_execution {
                    Err(AgentRuntimeError::Other(format!(
                        "accepted turn lost provider ownership before input for session {}",
                        session.id
                    )))
                } else {
                    Ok(())
                };
            }
            runtime
        };
        let start_result = async {
            if let Some(model) = selected_model {
                runtime.set_model(&model).await?;
            }
            runtime
                .start_turn(TurnInput {
                    prompt: payload.prompt,
                    images: payload
                        .images
                        .into_iter()
                        .map(|image| AttachmentPayload {
                            data: image.data,
                            media_type: image.media_type,
                        })
                        .collect(),
                    system_prompt: payload.system_prompt,
                    permission_mode: payload.permission_mode,
                    plan_mode: payload.plan_mode,
                    permission_profile_id: payload.permission_profile_id,
                    editor_context: payload.editor_context.map(EditorContext::from),
                })
                .await
        }
        .await;
        let _runtime_event_guard = self.ctx.runtime_event_locks.acquire(&session.id).await;
        match start_result {
            Ok(()) => {
                if !turn_owns_runtime(&self.ctx, &session.id, generation, &runtime).await {
                    return Ok(());
                }
                self.spawn_stale_watchdog(
                    session.id.clone(),
                    generation,
                    stale_timeout_for_session(session),
                );
                let runtime_epoch = {
                    let sessions = self.ctx.sessions.lock().await;
                    sessions
                        .get(&session.id)
                        .filter(|state| state.matches_generation(generation))
                        .map(RuntimeSessionState::runtime_epoch)
                };
                if let Some(identity) = accepted_running_identity {
                    mark_accepted_turn_running_or_retry(
                        &self.ctx,
                        &session.id,
                        generation,
                        identity.operation_id,
                        identity.execution_obligation_id,
                        turn_id,
                    );
                }
                drop(_runtime_event_guard);
                complete_context_restore_after_start_or_retry(
                    &self.ctx,
                    session.id.clone(),
                    runtime_epoch.unwrap_or_default(),
                    ContextRestoreCompletionRequest::after_started_turn(
                        expected_provider_session_generation,
                        turn_id,
                        context_was_reinjected,
                        clear_context_carry_after_start,
                        recovery_restore_required,
                    ),
                );
                emit_session_state_change_from_session(
                    session,
                    &self.ctx.notifier,
                    &self.ctx.status_center,
                    &self.ctx.status_notifier,
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
            Err(error) => {
                if !turn_runtime_is_current(&self.ctx, &session.id, generation, &runtime).await {
                    return if accepted_execution {
                        Err(AgentRuntimeError::Other(format!(
                            "accepted turn lost its provider failure outcome for session {}",
                            session.id
                        )))
                    } else {
                        Ok(())
                    };
                }
                let message = error.to_string();
                let (notification, interrupt_was_accepted) = complete_turn_with_acceptance(
                    &self.ctx,
                    &session.id,
                    Some(generation),
                    TurnResult::Interrupted {
                        reason: DomainInterruptReason::Crash,
                        error: Some(message),
                    },
                )
                .await
                .map_err(AgentRuntimeError::Other)?;
                if let Some(notification) = notification {
                    dispatch_workflow_turn_complete_notification(
                        &self.ctx.workflow_turn_complete_notifier,
                        notification,
                    )
                    .await;
                }
                if interrupt_was_accepted {
                    Ok(())
                } else {
                    Err(AgentRuntimeError::from(error))
                }
            }
        }
    }

    #[cfg(test)]
    async fn ensure_runtime(
        &self,
        session: &ChatSession,
        system_prompt: Option<String>,
    ) -> Result<Arc<dyn AgentSessionRuntime>, AgentRuntimeError> {
        if let Some(runtime) = self.live_runtime(&session.id).await {
            return Ok(runtime);
        }
        open_runtime_for_session(&self.ctx, session, system_prompt, None).await
    }

    async fn ensure_runtime_for_turn(
        &self,
        session: &ChatSession,
        system_prompt: Option<String>,
        generation: u64,
    ) -> Result<Arc<dyn AgentSessionRuntime>, AgentRuntimeError> {
        let runtime_open_epoch = {
            let mut sessions = self.ctx.sessions.lock().await;
            let state = sessions.get_mut(&session.id).ok_or_else(|| {
                AgentRuntimeError::Other(format!(
                    "Runtime state disappeared before opening session {}",
                    session.id
                ))
            })?;
            if let Some(runtime) = state.runtime.clone() {
                return Ok(runtime);
            }
            if !state.owns_generation(generation) {
                return Err(AgentRuntimeError::Other(format!(
                    "Turn no longer owns runtime open for session {}",
                    session.id
                )));
            }
            state.bump_runtime_epoch()
        };
        open_runtime_for_session(&self.ctx, session, system_prompt, Some(runtime_open_epoch)).await
    }

    fn spawn_stale_watchdog(
        &self,
        session_id: String,
        generation: u64,
        timeout: std::time::Duration,
    ) {
        spawn_stale_watchdog_task(&self.ctx, session_id, generation, timeout);
    }

    async fn live_runtime(&self, session_id: &str) -> Option<Arc<dyn AgentSessionRuntime>> {
        let sessions = self.ctx.sessions.lock().await;
        sessions
            .get(session_id)
            .and_then(|state| state.runtime.clone())
    }

    pub(crate) fn default_model_for_backend(&self, backend_id: &str) -> Result<String, String> {
        self.ctx.registry.default_model_for(backend_id)
    }

    #[cfg(test)]
    async fn stalled_active_turn_target(
        &self,
        session_id: &str,
    ) -> Result<Option<StalledActiveTurnTarget>, AgentRuntimeError> {
        let sessions = self.ctx.sessions.lock().await;
        let Some(state) = sessions.get(session_id) else {
            return Ok(None);
        };
        if !state.has_active_turn_lease() || !state.stall_observation_is_active() {
            return Ok(None);
        }
        let runtime = state.runtime.clone().ok_or_else(|| {
            AgentRuntimeError::Other(format!(
                "No active agent runtime for stalled session {session_id}"
            ))
        })?;
        Ok(Some(StalledActiveTurnTarget { runtime }))
    }

    #[cfg(test)]
    fn backend_supports_steering(&self, backend_id: &str) -> bool {
        self.ctx
            .registry
            .get(backend_id)
            .is_some_and(|backend| backend.capabilities().steering)
    }

    async fn hydrate_runtime_session_state(
        &self,
        session: &ChatSession,
    ) -> Result<(), AgentRuntimeError> {
        let backend_id = required_backend_id(session)?;
        let queue_paused_at = self
            .ctx
            .session_store
            .load_queue_paused_at(&self.ctx.data_dir, &session.id)
            .map_err(AgentRuntimeError::Other)?;
        let mut sessions = self.ctx.sessions.lock().await;
        sessions
            .entry(session.id.clone())
            .or_insert_with(|| RuntimeSessionState::with_queue_pause(backend_id, queue_paused_at));
        Ok(())
    }

    #[cfg(test)]
    async fn is_turn_busy(&self, session_id: &str) -> bool {
        let sessions = self.ctx.sessions.lock().await;
        sessions
            .get(session_id)
            .map(|state| {
                state.has_active_turn_lease()
                    || state.queue_is_paused()
                    || !state.accepted_input_effects.is_empty()
            })
            .unwrap_or(false)
    }

    #[cfg(test)]
    async fn pending_queue(&self, session_id: &str) -> Vec<QueuedAgentTurn> {
        let sessions = self.ctx.sessions.lock().await;
        sessions
            .get(session_id)
            .map(pending_queue_view)
            .unwrap_or_default()
    }

    #[cfg(test)]
    fn send_response(
        &self,
        session_id: &str,
        worktree_path: &str,
        human_message: ChatMessage,
        agent_message: Option<ChatMessage>,
        queued_turn: Option<QueuedAgentTurn>,
        pending_queue: Vec<QueuedAgentTurn>,
    ) -> Result<SendMessageResponse, AgentRuntimeError> {
        let projection = self.prepare_send_response_projection(session_id, worktree_path)?;
        Ok(SendMessageResponse {
            session: projection.session,
            human_message,
            agent_message,
            queued_turn,
            pending_queue_count: pending_queue.len(),
            pending_queue,
            sessions: projection.sessions,
        })
    }

    #[cfg(test)]
    fn prepare_send_response_projection(
        &self,
        session_id: &str,
        worktree_path: &str,
    ) -> Result<SendResponseProjection, AgentRuntimeError> {
        let session = self
            .ctx
            .session_store
            .get_session_shell(&self.ctx.data_dir, session_id)
            .map_err(AgentRuntimeError::Other)?
            .ok_or_else(|| AgentRuntimeError::Other(format!("Session not found: {session_id}")))?;
        let sessions = self
            .ctx
            .session_store
            .list_sessions(&self.ctx.data_dir, worktree_path)
            .map_err(AgentRuntimeError::Other)?;
        Ok(SendResponseProjection { session, sessions })
    }

    fn available_models_for_session(
        &self,
        session: &ChatSession,
    ) -> Result<Vec<ModelInfo>, AgentRuntimeError> {
        let backend_id = required_backend_id(session)?;
        self.ctx
            .registry
            .available_models(&backend_id)
            .map_err(AgentRuntimeError::Other)
    }

    fn build_turn_system_prompt(
        &self,
        session: &ChatSession,
        base_system_prompt: Option<String>,
        mentions: &[crate::domain::code::MentionReference],
        editor_context: Option<&AgentEditorContext>,
        workflow_instructions: Vec<String>,
    ) -> Result<Option<String>, AgentRuntimeError> {
        let backend_id = required_backend_id(session)?;
        let built = build_session_system_prompt(SessionSystemPromptBuildRequest {
            session_store: &self.ctx.session_store,
            data_dir: &self.ctx.data_dir,
            session,
            branch_diff_context: self.ctx.branch_diff_context.as_deref(),
            instruction_source: self.ctx.instruction_source.as_ref(),
            backend_id: &backend_id,
            model_id: session.selected_model.as_deref(),
            mentions,
            editor_context: editor_context.and_then(system_context_editor_input),
            workflow_instructions,
        })
        .map_err(AgentRuntimeError::Other)?;
        let prompt = compose_system_prompt(base_system_prompt, &built.system_context);
        persist_session_system_prompt_build(
            &self.ctx.session_store,
            &self.ctx.data_dir,
            &session.id,
            &built,
        )
        .map_err(AgentRuntimeError::Other)?;
        Ok(prompt)
    }
}
