impl SessionStore {
    #[cfg(test)]
    pub fn append_session_event_and_project_state(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        event: AgentSessionEvent,
    ) -> Result<SessionState, String> {
        self.append_session_event_and_project_read_model(app_data_dir, session_id, event)
            .map(|projected| projected.status.session_state)
    }

    #[cfg(test)]
    pub fn append_session_event_and_project(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        event: AgentSessionEvent,
    ) -> Result<SessionState, String> {
        #[cfg(test)]
        if let Some(hook) = self.append_event_hook.read().clone() {
            hook(session_id, &event)?;
        }
        self.commit_agent_events(app_data_dir, session_id, std::slice::from_ref(&event))?;
        if !self.canonical_authority_active() {
            #[cfg(test)]
            {
                self.test_storage().append_session_events(
                    app_data_dir,
                    session_id,
                    std::slice::from_ref(&event),
                )?;
                let events = self
                    .test_storage()
                    .load_session_events(app_data_dir, session_id)?;
                if self.test_storage().take_event_log_recovered(session_id) {
                    self.notify_event_log_recovered(session_id);
                }
                return self.project_session_events(app_data_dir, session_id, &events);
            }
            #[cfg(not(test))]
            return Err("agent-session SQLite event authority is not configured".to_string());
        }
        let events = self
            .read_session_projection(session_id)?
            .map(|projection| projection.reducer_events)
            .unwrap_or_default();
        #[cfg(test)]
        if let Some(hook) = self.appended_event_hook.read().clone() {
            hook(session_id, &event);
        }
        self.project_session_events(app_data_dir, session_id, &events)
    }

    #[cfg(test)]
    pub(crate) fn append_session_event_and_project_read_model(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        event: AgentSessionEvent,
    ) -> Result<SessionReadModel, String> {
        #[cfg(test)]
        if let Some(hook) = self.append_event_hook.read().clone() {
            hook(session_id, &event)?;
        }
        self.commit_agent_events(app_data_dir, session_id, std::slice::from_ref(&event))?;
        if !self.canonical_authority_active() {
            #[cfg(test)]
            {
                self.test_storage().append_session_events(
                    app_data_dir,
                    session_id,
                    std::slice::from_ref(&event),
                )?;
                let events = self
                    .test_storage()
                    .load_session_events(app_data_dir, session_id)?;
                if self.test_storage().take_event_log_recovered(session_id) {
                    self.notify_event_log_recovered(session_id);
                }
                let projected = TurnEventLog::from_events(events.clone()).project();
                self.project_session_events(app_data_dir, session_id, &events)?;
                return Ok(projected);
            }
            #[cfg(not(test))]
            return Err("agent-session SQLite event authority is not configured".to_string());
        }
        let events = self
            .read_session_projection(session_id)?
            .map(|projection| projection.reducer_events)
            .ok_or_else(|| format!("Session projection not found: {session_id}"))?;
        let projected = TurnEventLog::from_events(events.clone()).project();
        Ok(projected)
    }

    #[cfg(test)]
    pub(crate) fn append_error_episode_and_materialize(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        input: ErrorEpisodeInput,
    ) -> Result<(SessionReadModel, ChatMessage), String> {
        self.append_error_episode_with_queue_policy_and_materialize(
            app_data_dir,
            session_id,
            input,
            false,
        )
    }

    pub(crate) fn append_error_episode_and_pause_queue(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        input: ErrorEpisodeInput,
    ) -> Result<(SessionReadModel, ChatMessage), String> {
        self.append_error_episode_with_queue_policy_and_materialize(
            app_data_dir,
            session_id,
            input,
            true,
        )
    }

    fn append_error_episode_with_queue_policy_and_materialize(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        input: ErrorEpisodeInput,
        pause_queue: bool,
    ) -> Result<(SessionReadModel, ChatMessage), String> {
        let message_id = input.message_id;
        let event = AgentSessionEvent::SessionErrored {
            message_id: message_id.clone(),
            reason: input.reason,
            at: input.at,
        };
        let queue_was_paused = pause_queue
            && self
                .load_queue_paused_at(app_data_dir, session_id)?
                .is_some();
        #[cfg(test)]
        if let Some(hook) = self.append_event_hook.read().clone() {
            hook(session_id, &event)?;
        }
        let mut events = vec![event];
        if pause_queue && !queue_was_paused {
            events.push(AgentSessionEvent::QueuePaused { at: input.at });
        }
        let (projected, message, _) = self.commit_projection_and_notify_with_queue_guard(
            app_data_dir,
            session_id,
            &events,
            pause_queue.then_some(queue_was_paused),
            |projected, projected_meta| {
                let message = projected
                    .message_for_id(&message_id)
                    .cloned()
                    .ok_or_else(|| {
                        format!("Error projection omitted message {message_id} for {session_id}")
                    })?;
                #[cfg(test)]
                if let Some(hook) = self.append_message_hook.read().clone() {
                    hook(session_id, &message)?;
                }
                Ok((
                    AgentSessionProjectionCommit {
                        meta: projected_meta,
                        message: AgentSessionProjectedMessage::Append(message.clone()),
                    },
                    message,
                ))
            },
        )?;
        Ok((projected, message))
    }

    #[allow(clippy::too_many_arguments)] // Terminal identity, projection patch, and result must enter one commit together.
    pub(crate) fn append_terminal_events_and_materialize(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        events: &[AgentSessionEvent],
        message_id: &str,
        streaming_final_seq: u64,
        completed_at: f64,
        turn_result: &crate::domain::agent_session::entities::TurnResult,
    ) -> Result<(SessionReadModel, Vec<MessagePart>), String> {
        #[cfg(test)]
        if let Some(hook) = self.append_event_hook.read().clone() {
            for event in events {
                hook(session_id, event)?;
            }
        }
        if self.canonical_authority_active() {
            let codec = self
                .event_authority
                .read()
                .as_ref()
                .map(|authority| authority.projection_codec.clone())
                .ok_or_else(|| "agent-session projection codec is unavailable".to_string())?;
            let previous = self
                .read_session_projection(session_id)?
                .ok_or_else(|| format!("Session projection not found: {session_id}"))?;
            let candidate_turn_id = SessionAggregate::terminal_turn_id(events);
            let durable_winner = candidate_turn_id
                .map(|turn_id| self.canonical_terminal(session_id, turn_id))
                .transpose()?
                .flatten();
            if !SessionAggregate::terminal_commit_is_current(
                &previous.reducer_events,
                events,
                durable_winner.is_some(),
            ) {
                let projected =
                    TurnEventLog::from_events(previous.reducer_events.clone()).project();
                let persisted_parts = self
                    .canonical_message_projection(session_id, message_id)?
                    .and_then(|message| message.parts)
                    .unwrap_or_else(|| projected.agent_parts_for_message(message_id));
                return Ok((projected, persisted_parts));
            }
            let events = complete_terminal_projection_events(&previous.reducer_events, events);
            if !events.is_empty() {
                let encoded_events = codec.encode_events_for_identity(&events)?;
                let terminal_mutation = runtime_terminal_record_mutation(
                    session_id,
                    &events,
                    message_id,
                    streaming_final_seq,
                    completed_at,
                    turn_result,
                    &encoded_events,
                )?;
                let mut additional_mutations = vec![terminal_mutation];
                let terminal_record = match &additional_mutations[0] {
                    crate::domain::local_event::LocalStateMutation::TerminalRecord(record) => {
                        record.clone()
                    }
                    _ => unreachable!("runtime terminal builder always returns a terminal row"),
                };
                if let Some(workflow_context) = previous.meta.workflow_node_context.as_ref() {
                    let mut candidate_events = previous.reducer_events.clone();
                    candidate_events.extend(events.iter().cloned());
                    let workflow_input = TurnEventLog::from_events(candidate_events)
                        .project()
                        .workflow_turn_complete
                        .ok_or_else(|| {
                            "workflow-owned terminal omitted its turn-completion input".to_string()
                        })?;
                    if candidate_turn_id != Some(workflow_input.turn_id) {
                        return Err(
                            "workflow-owned terminal projected a different turn identity"
                                .to_string(),
                        );
                    }
                    // The workflow turn-complete usecase intentionally treats
                    // a clean interruption as a no-op. Do not create an
                    // impossible completion obligation that would otherwise
                    // block orphan recovery forever waiting for a workflow
                    // commit which is not meant to exist.
                    if SessionAggregate::requires_workflow_turn_completion(
                        workflow_input.interrupted,
                        workflow_input.exit_code,
                        workflow_input.failure_signal.is_some(),
                    ) {
                        additional_mutations.push(workflow_turn_completion_pending_mutation(
                            codec.as_ref(),
                            session_id,
                            workflow_context,
                            &terminal_record,
                            message_id,
                            &workflow_input,
                        )?);
                    }
                }
                let participant_provider =
                    self.runtime_terminal_participant_provider.read().clone();
                if participant_provider.is_none() {
                    #[cfg(not(test))]
                    return Err(
                        "runtime terminal participant provider is not configured".to_string()
                    );
                }
                let terminal_participant =
                    participant_provider.map(|provider| (provider, terminal_record));
                if let Err(error) = self.commit_agent_events_with_additional_mutations(
                    session_id,
                    &events,
                    additional_mutations,
                    Some(TerminalMessageProjectionPatch {
                        message_id: message_id.to_string(),
                        streaming_final_seq,
                        timestamp: Some(completed_at),
                        parts: None,
                    }),
                    None,
                    terminal_participant,
                    crate::domain::local_event::CommitOperationKind::Projection,
                ) {
                    let converged = candidate_turn_id
                        .map(|turn_id| self.canonical_terminal(session_id, turn_id))
                        .transpose()?
                        .flatten()
                        .is_some();
                    if !converged {
                        return Err(error);
                    }
                }
            }
            let canonical = self
                .read_session_projection(session_id)?
                .ok_or_else(|| format!("Session projection not found: {session_id}"))?;
            let projected = TurnEventLog::from_events(canonical.reducer_events.clone()).project();
            self.projected_meta_for_commit(
                session_id,
                &canonical.meta,
                &canonical.reducer_events,
                &projected,
            )?;
            projected
                .message_for_id(message_id)
                .filter(|message| message.role == MessageRole::Agent)
                .ok_or_else(|| {
                    format!("Turn projection omitted message {message_id} for {session_id}")
                })?;
            let persisted_parts = projected.agent_parts_for_message(message_id);
            #[cfg(test)]
            if let Some(hook) = self.persist_parts_hook.read().clone() {
                hook(session_id, message_id, &persisted_parts)?;
            }
            self.notify_projected_commit(
                session_id,
                Some(PreviousSessionProjection {
                    state: previous.meta.state,
                    error_reason: previous.meta.error_reason,
                    worktree_path: previous.meta.worktree_path,
                    state_revision: canonical.meta.state_revision,
                }),
                &projected,
            );
            return Ok((projected, persisted_parts));
        }
        let (projected, (), persisted_parts) = self.commit_projection_and_notify(
            app_data_dir,
            session_id,
            events,
            |projected, projected_meta| {
                projected
                    .message_for_id(message_id)
                    .filter(|message| message.role == MessageRole::Agent)
                    .ok_or_else(|| {
                        format!("Turn projection omitted message {message_id} for {session_id}")
                    })?;
                let parts = projected.agent_parts_for_message(message_id);
                #[cfg(test)]
                if let Some(hook) = self.persist_parts_hook.read().clone() {
                    hook(session_id, message_id, &parts)?;
                }
                Ok((
                    AgentSessionProjectionCommit {
                        meta: projected_meta,
                        message: AgentSessionProjectedMessage::PersistParts {
                            message_id: message_id.to_string(),
                            parts,
                            streaming_final_seq,
                            completed_at,
                        },
                    },
                    (),
                ))
            },
        )?;
        Ok((projected, persisted_parts))
    }

    fn commit_projection_and_notify<Output>(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        events: &[AgentSessionEvent],
        build_commit: impl FnMut(
            &SessionReadModel,
            SessionMeta,
        ) -> Result<
            (
                AgentSessionProjectionCommit<SessionMeta, ChatMessage, MessagePart>,
                Output,
            ),
            String,
        >,
    ) -> Result<(SessionReadModel, Output, Vec<MessagePart>), String> {
        self.commit_projection_and_notify_with_queue_guard(
            app_data_dir,
            session_id,
            events,
            None,
            build_commit,
        )
    }

    fn commit_projection_and_notify_with_queue_guard<Output>(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        events: &[AgentSessionEvent],
        expected_queue_paused: Option<bool>,
        mut build_commit: impl FnMut(
            &SessionReadModel,
            SessionMeta,
        ) -> Result<
            (
                AgentSessionProjectionCommit<SessionMeta, ChatMessage, MessagePart>,
                Output,
            ),
            String,
        >,
    ) -> Result<(SessionReadModel, Output, Vec<MessagePart>), String> {
        self.ensure_canonical_mutation_admission()?;
        if !self.canonical_authority_active() {
            #[cfg(test)]
            {
                let mut projected_result = None;
                let mut previous_projection = None;
                let persisted_parts = {
                    let mut prepare = |all_events: &[AgentSessionEvent], meta: &SessionMeta| {
                        let projected = TurnEventLog::from_events(all_events.to_vec()).project();
                        let mut projected_meta = self
                            .projected_meta_for_commit(session_id, meta, all_events, &projected)?;
                        projected_meta.state_revision =
                            next_sqlite_counter(meta.state_revision, "session state revision")?;
                        previous_projection = Some(PreviousSessionProjection {
                            state: meta.state,
                            error_reason: meta.error_reason.clone(),
                            worktree_path: meta.worktree_path.clone(),
                            state_revision: projected_meta.state_revision,
                        });
                        let (commit, output) = build_commit(&projected, projected_meta)?;
                        projected_result = Some((projected, output));
                        Ok(commit)
                    };
                    self.test_storage().commit_session_projection(
                        app_data_dir,
                        session_id,
                        events,
                        &mut prepare,
                    )?
                };
                if self.test_storage().take_event_log_recovered(session_id) {
                    self.notify_event_log_recovered(session_id);
                }
                let (projected, output) = projected_result
                    .expect("commit_session_projection must invoke prepare before returning Ok");
                self.notify_projected_commit(session_id, previous_projection, &projected);
                return Ok((projected, output, persisted_parts));
            }
            #[cfg(not(test))]
            return Err("agent-session SQLite event authority is not configured".to_string());
        }

        let previous_canonical = self
            .read_session_projection(session_id)?
            .ok_or_else(|| format!("Session projection not found: {session_id}"))?;
        let previous_projection = Some(PreviousSessionProjection {
            state: previous_canonical.meta.state,
            error_reason: previous_canonical.meta.error_reason.clone(),
            worktree_path: previous_canonical.meta.worktree_path.clone(),
            state_revision: previous_canonical.meta.state_revision,
        });
        match expected_queue_paused {
            Some(expected_queue_paused) => self.commit_agent_events_with_queue_pause_guard(
                app_data_dir,
                session_id,
                events,
                expected_queue_paused,
            )?,
            None => self.commit_agent_events(app_data_dir, session_id, events)?,
        }
        let canonical = self
            .read_session_projection(session_id)?
            .ok_or_else(|| format!("Session projection not found: {session_id}"))?;
        let projected = TurnEventLog::from_events(canonical.reducer_events.clone()).project();
        let mut projected_meta = self.projected_meta_for_commit(
            session_id,
            &canonical.meta,
            &canonical.reducer_events,
            &projected,
        )?;
        projected_meta.state_revision = canonical.meta.state_revision;
        let (derived_commit, output) = build_commit(&projected, projected_meta)?;
        let persisted_parts = match &derived_commit.message {
            AgentSessionProjectedMessage::PersistParts { parts, .. } => parts.clone(),
            AgentSessionProjectedMessage::Append(_) => Vec::new(),
        };
        self.notify_projected_commit(session_id, previous_projection, &projected);
        Ok((projected, output, persisted_parts))
    }

    fn projected_meta_for_commit(
        &self,
        _session_id: &str,
        meta: &SessionMeta,
        events: &[AgentSessionEvent],
        projected: &SessionReadModel,
    ) -> Result<SessionMeta, String> {
        #[cfg(test)]
        if let Some(hook) = self.projection_hook.read().clone() {
            hook(
                _session_id,
                &projected.status.session_state,
                projected.error_reason.as_deref(),
            )?;
        }
        let mut projected_meta = meta.clone();
        projected_meta.state = projected.status.session_state;
        projected_meta.error_reason =
            error_reason_for_state(&projected_meta.state, &projected.error_reason);
        projected_meta.last_turn_interruption = latest_turn_interruption(events);
        projected_meta.last_turn_id = events.iter().rev().find_map(|event| match event {
            AgentSessionEvent::TurnStarted { turn_id, .. } => Some(*turn_id),
            _ => None,
        });
        #[cfg(test)]
        if let Some(hook) = self.event_projection_hook.read().clone() {
            hook(_session_id, projected_meta.last_turn_id)?;
        }
        Ok(projected_meta)
    }

    fn notify_projected_commit(
        &self,
        session_id: &str,
        previous_projection: Option<PreviousSessionProjection>,
        projected: &SessionReadModel,
    ) {
        #[cfg(test)]
        if let Some(hook) = self.projected_read_model_hook.read().clone() {
            hook(session_id, projected);
        }
        let previous = previous_projection
            .expect("commit_session_projection must invoke prepare before returning Ok");
        let projected_reason =
            error_reason_for_state(&projected.status.session_state, &projected.error_reason);
        if previous.state != projected.status.session_state
            || previous.error_reason != projected_reason
        {
            self.notify_state_change(
                session_id,
                &previous.worktree_path,
                &projected.status.session_state,
                previous.state_revision,
            );
        }
    }

    pub fn append_turn_started_and_project_state(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        event: AgentSessionEvent,
    ) -> Result<(), String> {
        let _turn_id = match &event {
            AgentSessionEvent::TurnStarted { turn_id, .. } => *turn_id,
            _ => return Err("Turn start projection requires a TurnStarted event".to_string()),
        };
        self.append_session_event_without_projection(app_data_dir, session_id, event.clone())?;
        #[cfg(test)]
        if !self.canonical_authority_active() {
            if let Err(projection_error) = self.set_event_projection(
                app_data_dir,
                session_id,
                SessionState::Active,
                None,
                None,
                Some(_turn_id),
            ) {
                let recovery =
                    self.load_session_events(app_data_dir, session_id)
                        .and_then(|events| {
                            self.project_session_events(app_data_dir, session_id, &events)
                                .map(|_| ())
                        });
                return match recovery {
                    Ok(()) => Err(projection_error),
                    Err(recovery_error) => Err(format!(
                        "{projection_error}; failed to recover committed turn projection: {recovery_error}"
                    )),
                };
            }
        }
        #[cfg(test)]
        if let Some(hook) = self.appended_event_hook.read().clone() {
            hook(session_id, &event);
        }
        Ok(())
    }

    /// Commit the dequeue boundary for a durably accepted queued send. The
    /// transaction verifies and removes only the exact canonical queue front;
    /// a recovery worker restoring a later item cannot advance it first.
    #[cfg(test)]
    pub(crate) fn append_accepted_queued_turn_started_and_project_state(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        queue_item_id: &str,
        event: AgentSessionEvent,
    ) -> Result<(), String> {
        if !matches!(&event, AgentSessionEvent::TurnStarted { .. }) {
            return Err("Accepted queue projection requires a TurnStarted event".to_string());
        }
        if self.canonical_authority_active() {
            return self.commit_agent_events_with_kind_and_queue_front(
                app_data_dir,
                session_id,
                std::slice::from_ref(&event),
                crate::domain::local_event::CommitOperationKind::Projection,
                Some(ExpectedAcceptedQueueFront {
                    queue_item_id: queue_item_id.to_string(),
                }),
                Vec::new(),
                None,
            );
        }
        #[cfg(test)]
        {
            self.append_turn_started_and_project_state(app_data_dir, session_id, event)
        }
        #[cfg(not(test))]
        {
            Err("agent-session SQLite event authority is not configured".to_string())
        }
    }

    /// Atomically claim a queued send and materialize its canonical
    /// `TurnStarted` boundary. Lifecycle/recovery winners leave every supplied
    /// operation participant untouched and retain the exact queue item.
    pub(crate) fn commit_accepted_queued_turn_start_with_participants(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        queue_item_id: &str,
        event: AgentSessionEvent,
        additional_mutations: Vec<crate::domain::local_event::LocalStateMutation>,
    ) -> Result<AcceptedQueuedTurnStartCommitOutcome, String> {
        if !matches!(&event, AgentSessionEvent::TurnStarted { .. }) {
            return Err("Accepted queue projection requires a TurnStarted event".to_string());
        }
        if !self.canonical_authority_active() {
            return Err("agent-session SQLite event authority is not configured".to_string());
        }
        let result = self.commit_agent_events_with_kind_and_queue_front(
            app_data_dir,
            session_id,
            std::slice::from_ref(&event),
            crate::domain::local_event::CommitOperationKind::Send,
            Some(ExpectedAcceptedQueueFront {
                queue_item_id: queue_item_id.to_string(),
            }),
            additional_mutations,
            None,
        );
        match result {
            Ok(()) => {
                #[cfg(test)]
                if let Some(hook) = self.appended_event_hook.read().clone() {
                    hook(session_id, &event);
                }
                Ok(AcceptedQueuedTurnStartCommitOutcome::Committed)
            }
            Err(error) if error.starts_with(ACCEPTED_QUEUE_START_BLOCKED) => {
                Ok(AcceptedQueuedTurnStartCommitOutcome::Blocked)
            }
            Err(error) => Err(error),
        }
    }

    #[cfg(test)]
    pub fn project_session_events(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        events: &[AgentSessionEvent],
    ) -> Result<SessionState, String> {
        let last_turn_interruption = latest_turn_interruption(events);
        let last_turn_id = events.iter().rev().find_map(|event| match event {
            AgentSessionEvent::TurnStarted { turn_id, .. } => Some(*turn_id),
            _ => None,
        });
        let projected = TurnEventLog::from_events(events.to_vec()).project();
        let projected_state = projected.status.session_state;
        self.set_event_projection(
            app_data_dir,
            session_id,
            projected_state,
            projected.error_reason,
            last_turn_interruption,
            last_turn_id,
        )?;
        Ok(projected_state)
    }

    pub(crate) fn next_turn_id(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<u64, NextTurnIdError> {
        if self.canonical_authority_active() {
            return self
                .send_acceptance_allocation(session_id)
                .map(|allocation| allocation.next_turn_id);
        }

        let meta = self.require_meta(app_data_dir, session_id)?;
        #[cfg(test)]
        let last_turn_id = match meta.last_turn_id {
            Some(turn_id) => turn_id,
            None => self
                .test_storage()
                .load_session_events(app_data_dir, session_id)?
                .iter()
                .rev()
                .find_map(|event| match event {
                    AgentSessionEvent::TurnStarted { turn_id, .. } => Some(*turn_id),
                    _ => None,
                })
                .unwrap_or(0),
        };
        #[cfg(not(test))]
        let last_turn_id = meta.last_turn_id.unwrap_or(0);
        allocate_next_turn_identity(last_turn_id, std::iter::empty()).map_err(Into::into)
    }

    /// Allocate from one canonical projection snapshot and return the exact
    /// revision that must guard the later acceptance mutation. Queue identity
    /// is strictly increasing in canonical order; accepting new work on a
    /// malformed queue would otherwise make turn reuse permanent.
    pub(crate) fn send_acceptance_allocation(
        &self,
        session_id: &str,
    ) -> Result<SendAcceptanceAllocation, NextTurnIdError> {
        let (projection, revision) = self
            .read_session_projection_with_revision(session_id)?
            .ok_or_else(|| format!("Session projection not found: {session_id}"))?;
        let last_turn_id = projection.meta.last_turn_id.unwrap_or_else(|| {
            projection
                .reducer_events
                .iter()
                .rev()
                .find_map(|event| match event {
                    AgentSessionEvent::TurnStarted { turn_id, .. } => Some(*turn_id),
                    _ => None,
                })
                .unwrap_or(0)
        });
        let next_turn_id = allocate_next_turn_identity(
            last_turn_id,
            projection
                .pending_send_queue
                .iter()
                .map(|pending| ReservedTurnIdentity {
                    queue_item_id: &pending.queue_item_id,
                    turn_id: &pending.reserved_turn_id,
                }),
        )
        .map_err(NextTurnIdError::from)?;
        Ok(SendAcceptanceAllocation {
            next_turn_id,
            has_active_turn: reducer_has_active_turn(&projection.reducer_events),
            has_pending_queue: !projection.pending_send_queue.is_empty(),
            session_projection_guard: crate::domain::local_event::RevisionGuard::Expected(revision),
        })
    }

}
