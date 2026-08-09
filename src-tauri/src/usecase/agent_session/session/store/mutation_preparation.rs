impl SessionStore {
    /// Prepare the bounded canonical session/message projection mutations for
    /// an arbitrary agent-event slice without committing them. Operation
    /// admission paths use this to include their domain events and read-model
    /// changes in the same compare-and-swap batch.
    pub(crate) fn prepare_event_projection_mutations(
        &self,
        session_id: &str,
        events: &[AgentSessionEvent],
    ) -> Result<Vec<crate::domain::local_event::LocalStateMutation>, String> {
        let authority = self
            .event_authority
            .read()
            .clone()
            .ok_or_else(|| "agent-session SQLite authority is unavailable".to_string())?;
        let current_projection = self.read_session_projection(session_id)?;
        let fallback_meta = current_projection
            .as_ref()
            .map(|projection| projection.meta.clone());
        let session_id = session_id.to_string();
        let events = complete_terminal_projection_events(
            current_projection
                .as_ref()
                .map(|projection| projection.reducer_events.as_slice())
                .unwrap_or_default(),
            events,
        );
        std::thread::scope(|scope| {
            scope
                .spawn(move || {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|error| {
                            format!("failed to create event projection runtime: {error}")
                        })?
                        .block_on(prepare_canonical_event_projection_mutations(
                            &authority,
                            &session_id,
                            &events,
                            fallback_meta,
                            None,
                            None,
                        ))
                })
                .join()
                .map_err(|_| "event projection worker panicked".to_string())?
        })
    }

    /// Prepare an event projection only when the mutation was derived from
    /// the caller's exact public session revision. The returned projection
    /// retains its own SQLite revision guard, so a change after preparation
    /// still conflicts at commit.
    pub(crate) fn prepare_event_projection_mutations_if_current_revision(
        &self,
        session_id: &str,
        expected_state_revision: u64,
        events: &[AgentSessionEvent],
    ) -> Result<Option<Vec<crate::domain::local_event::LocalStateMutation>>, String> {
        let mutations = self.prepare_event_projection_mutations(session_id, events)?;
        let authority = self
            .event_authority
            .read()
            .clone()
            .ok_or_else(|| "agent-session SQLite authority is unavailable".to_string())?;
        let projection = mutations
            .iter()
            .find_map(|mutation| match mutation {
                crate::domain::local_event::LocalStateMutation::SessionProjection(projection) => {
                    Some(projection)
                }
                _ => None,
            })
            .ok_or_else(|| "agent event batch omitted its session projection".to_string())?;
        let projected = authority.projection_codec.decode(&projection.projection)?;
        let expected_projected_revision =
            next_sqlite_counter(expected_state_revision, "guarded session state revision")?;
        if projected.meta.state_revision != expected_projected_revision {
            return Ok(None);
        }
        Ok(Some(mutations))
    }

    /// Prepare the owner-side closure for a backend recovery whose provider
    /// effect survived a process crash but whose completion batch did not.
    ///
    /// The durable provider identity/generation is the only success evidence.
    /// This method does not commit: RecoveryActionUsecase appends the returned
    /// events, projection and publication obligation beside its action/source
    /// obligation CAS in one LocalAtomicBatch.
    pub(crate) fn prepare_backend_recovery_readback_completion(
        &self,
        session_id: &str,
        recovery_id: &str,
    ) -> Result<Option<BackendRecoveryReadbackParticipants>, String> {
        let source_obligation_id = backend_recovery_obligation_id(session_id, recovery_id);
        let source = self
            .canonical_obligation(&source_obligation_id)?
            .ok_or_else(|| "backend recovery readback has no durable reservation".to_string())?;
        let reservation =
            crate::domain::agent_session::services::backend_recovery_reservation(
                &source.record,
                session_id,
                recovery_id,
            )
            .ok_or_else(|| {
                "backend recovery readback reservation identity is inconsistent".to_string()
            })?;
        let old_provider_session_generation = reservation.old_provider_session_generation;
        let reserved_at_bits = reservation.reserved_at_bits;
        let current_projection = self
            .read_session_projection(session_id)?
            .ok_or_else(|| "backend recovery owner projection is unavailable".to_string())?;
        let publication_snapshot = current_projection
            .meta
            .recovery_publication_snapshot
            .as_ref();
        let completion = match decide_backend_recovery_durable_completion(
            BackendRecoveryDurableCompletionFacts {
            session_id,
            recovery_id,
            old_provider_session_generation,
            reserved_at_bits,
            projected_provider_session_generation: current_projection
                .meta
                .provider_session_generation,
            context_reinjection_generation: current_projection.meta.context_reinjection_generation,
            backend_session_id: current_projection.meta.agent_session_id.as_deref(),
            publication_recovery_id: publication_snapshot.map(|snapshot| snapshot.recovery_id.as_str()),
            publication_session_id: publication_snapshot.map(|snapshot| snapshot.summary.id.as_str()),
            completed_at_bits: current_projection.meta.updated_at.to_bits(),
            },
        ) {
            Ok(BackendRecoveryDurableCompletionDecision::NotReady) => return Ok(None),
            Ok(BackendRecoveryDurableCompletionDecision::Complete(completion)) => completion,
            Err(
                BackendRecoveryDurableCompletionRejection::ProviderSessionGenerationCapacityExceeded,
            ) => {
                return Err("provider session generation capacity is exhausted".to_string());
            }
            Err(BackendRecoveryDurableCompletionRejection::InvalidCompletionTimestamp) => {
                return Err(
                    "backend recovery durable completion timestamp is invalid".to_string(),
                );
            }
        };
        let provider_session_generation = completion.provider_session_generation;
        let backend_session_id = completion.backend_session_id;
        let at = f64::from_bits(completion.completed_at_bits);
        let pending_recovery_message = PendingRecoveryMessage::Notice {
            recovery_id: recovery_id.to_string(),
            message_id: completion.publication_message_id,
        };
        let authority = self
            .event_authority
            .read()
            .clone()
            .ok_or_else(|| "agent-session SQLite authority is unavailable".to_string())?;
        let publication_message = authority
            .projection_codec
            .recovery_publication_message_record(&pending_recovery_message);
        let source_completion = backend_recovery_obligation_mutation(
            source_obligation_id.clone(),
            crate::domain::local_event::ObligationRecord::BackendSessionRecovery {
                session_id: session_id.to_string(),
                recovery_id: recovery_id.to_string(),
                detail:
                    crate::domain::local_event::BackendSessionRecoveryObligationRecord::Completed {
                        old_provider_session_generation,
                        provider_session_generation,
                        backend_session_id: backend_session_id.clone(),
                        completed_at_bits: at.to_bits(),
                    },
                state: crate::domain::local_event::ObligationStateRecord::Completed,
            },
            None,
            Some(&source),
        )?;
        let publication_obligation_id = recovery_publication_obligation_id(
            session_id,
            recovery_id,
            &publication_message.message_id,
        );
        if self
            .canonical_obligation(&publication_obligation_id)?
            .is_some()
        {
            return Err("backend recovery publication identity was already reserved".to_string());
        }
        let publication = recovery_publication_obligation_mutation(
            publication_obligation_id.clone(),
            crate::domain::local_event::ObligationRecord::RecoveryPublication {
                session_id: session_id.to_string(),
                recovery_id: recovery_id.to_string(),
                message_id: publication_message.message_id.clone(),
                source_obligation_id,
                detail: crate::domain::local_event::RecoveryPublicationObligationRecord::Pending {
                    pending_message: publication_message,
                },
                state: crate::domain::local_event::ObligationStateRecord::Pending,
            },
            Some(crate::domain::local_event::PendingIndexEntry {
                ordered_key: format!(
                    "{:020}:{publication_obligation_id}",
                    (at * 1000.0).round() as i64
                ),
                owner: session_id.to_string(),
                partition: crate::domain::local_event::PendingPartition::Owner,
                shutdown_plan: None,
            }),
            None,
        )?;
        let events = vec![
            AgentSessionEvent::SessionConfigurationReactivated {
                recovery_id: recovery_id.to_string(),
                provider_session_generation,
                consumed_observation_id: None,
                at,
            },
            AgentSessionEvent::SessionGoalReactivated {
                recovery_id: recovery_id.to_string(),
                outcome: GoalReactivationOutcome::NoCurrentGoal,
                provider_session_generation,
                restoring_turn_id: None,
                consumed_observation_id: None,
                at,
            },
            AgentSessionEvent::BackendSessionRecoveryCompleted {
                recovery_id: recovery_id.to_string(),
                provider_session_generation,
                at,
            },
        ];
        let fallback_meta = Some(current_projection.meta);
        let session_id = session_id.to_string();
        std::thread::scope(|scope| {
            scope
                .spawn(move || {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|error| {
                            format!("failed to create backend readback runtime: {error}")
                        })?
                        .block_on(async move {
                            let codec = authority.projection_codec.as_ref();
                            let stream_id =
                                crate::domain::local_event::StreamId::agent_session(&session_id)
                                    .map_err(|_| {
                                        "backend recovery stream identity is invalid".to_string()
                                    })?;
                            let head = authority
                                .repository
                                .load_stream(crate::domain::local_event::LoadStreamRequest {
                                    stream_id: stream_id.clone(),
                                    after: None,
                                    limit: 1,
                                })
                                .await
                                .map_err(|error| {
                                    format!("backend recovery stream head read failed: {error}")
                                })?
                                .head;
                            let mut mutations = prepare_canonical_event_projection_mutations(
                                &authority,
                                &session_id,
                                &events,
                                fallback_meta,
                                None,
                                None,
                            )
                            .await?;
                            patch_event_projection_meta(
                                codec,
                                &mut mutations,
                                &EventProjectionMetaPatch::ReadbackCompleted {
                                    old_provider_session_generation,
                                    provider_session_generation,
                                    backend_session_id,
                                    pending_recovery_message,
                                    at,
                                },
                            )?;
                            mutations.push(publication);
                            let occurred_at_ms = (at * 1000.0).round() as i64;
                            let uncommitted_events = events
                                .into_iter()
                                .map(|event| crate::domain::local_event::UncommittedDomainEvent {
                                    stream_id: stream_id.clone(),
                                    event:
                                        crate::domain::local_event::LocalDomainEvent::AgentSession(
                                            event,
                                        ),
                                    occurred_at_ms,
                                })
                                .collect::<Vec<_>>();
                            let encoded_events = authority
                                .repository
                                .canonical_event_batch_identity_v1(&uncommitted_events)?;
                            let mut mutation_identities = Vec::with_capacity(mutations.len());
                            for mutation in &mutations {
                                let encoded = authority
                                    .repository
                                    .canonical_mutation_identity_v1(mutation)?;
                                mutation_identities.push(encoded);
                            }
                            mutation_identities.sort();
                            let participant_digest =
                                crate::domain::local_event::backend_recovery_readback_participant_identity(
                                    stream_id.as_str(),
                                    head.value(),
                                    &encoded_events,
                                    mutation_identities.iter().map(Vec::as_slice),
                                );
                            // RecoveryActionUsecase validates and merges this
                            // exact source closure into its single wrapped
                            // source mutation; it is deliberately excluded
                            // from the owner-batch digest after normalization.
                            mutations.push(source_completion);
                            Ok(Some(BackendRecoveryReadbackParticipants {
                                expected_heads: vec![
                                    crate::domain::local_event::ExpectedStreamHead {
                                        stream_id: stream_id.clone(),
                                        expected: head,
                                    },
                                ],
                                events: uncommitted_events,
                                canonical_events: encoded_events,
                                mutations,
                                participant_digest,
                            }))
                        })
                })
                .join()
                .map_err(|_| "backend recovery readback worker panicked".to_string())?
        })
    }

    /// Prepare the canonical lifecycle projection as a participant of the
    /// lifecycle acceptance transaction. Runtime teardown happens only after
    /// this mutation commits; it must never perform a second canonical state
    /// write for close, archive, or backend selection.
    pub(crate) fn prepare_lifecycle_acceptance_mutations(
        &self,
        session_id: &str,
        expected_revision: u64,
        events: &[AgentSessionEvent],
        lifecycle_state: SessionState,
        backend_selection: Option<(&str, &str)>,
    ) -> Result<Option<Vec<crate::domain::local_event::LocalStateMutation>>, String> {
        let Some(mut mutations) = self.prepare_event_projection_mutations_if_current_revision(
            session_id,
            expected_revision,
            events,
        )? else {
            return Ok(None);
        };
        let authority = self
            .event_authority
            .read()
            .clone()
            .ok_or_else(|| "agent-session SQLite authority is unavailable".to_string())?;
        let codec = authority.projection_codec.as_ref();
        let projection = mutations
            .iter_mut()
            .find_map(|mutation| match mutation {
                crate::domain::local_event::LocalStateMutation::SessionProjection(projection) => {
                    Some(projection)
                }
                _ => None,
            })
            .ok_or_else(|| "lifecycle batch omitted its session projection".to_string())?;
        let mut decoded = codec.decode(&projection.projection)?;
        decoded.meta.state = lifecycle_state;
        if !decoded.meta.state.retains_error_reason() {
            decoded.meta.error_reason = None;
        }
        if let Some((backend_id, selected_model)) = backend_selection {
            decoded.meta.backend_id = backend_id.to_string();
            decoded.meta.selected_model = Some(selected_model.to_string());
        }
        decoded.meta.updated_at = now_timestamp();
        projection.projection = codec.encode(&decoded)?;
        Ok(Some(mutations))
    }

    pub(crate) fn prepare_send_acceptance_mutations(
        &self,
        input: SendAcceptanceProjectionInput<'_>,
    ) -> Result<Vec<crate::domain::local_event::LocalStateMutation>, String> {
        let SendAcceptanceProjectionInput {
            session_id,
            initial_session,
            session_projection_guard,
            human_message_id,
            prompt,
            disposition,
            reserved_turn_id,
            input_ref,
            events,
        } = input;
        let authority = self
            .event_authority
            .read()
            .clone()
            .ok_or_else(|| "agent-session SQLite authority is unavailable".to_string())?;
        let codec = authority.projection_codec.clone();
        let session_id = session_id.to_string();
        let human_message_id = human_message_id.to_string();
        let prompt = prompt.clone();
        let disposition = disposition.clone();
        let reserved_turn_id = reserved_turn_id.map(str::to_string);
        let input_ref = input_ref.to_string();
        let initial_meta = initial_session.map(SessionMeta::from_session);
        let events = events.to_vec();
        std::thread::scope(|scope| {
            scope
                .spawn(move || {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|error| format!("failed to create send projection runtime: {error}"))?
                        .block_on(async move {
                            // Projection, revision, and recovery inputs come from one SQLite
                            // reader snapshot; the projection mutation below retains that revision
                            // as its commit fence.
                            let stored = match authority
                                .repository
                                .query(crate::domain::local_event::LocalEventQuery::AgentSessionLifecycleSnapshot {
                                    session_id: session_id.clone(),
                                })
                                .await
                                .map_err(|error| format!("send projection read failed: {error}"))?
                            {
                                crate::domain::local_event::LocalEventQueryResult::AgentSessionLifecycleSnapshot(stored) => stored,
                                _ => return Err("send lifecycle snapshot query returned the wrong shape".to_string()),
                            };
                            let (
                                mut meta,
                                title,
                                reducer_events,
                                mut pending_send_queue,
                                mut session_aggregate,
                                expected,
                                revision,
                            ) = match stored {
                                Some(snapshot) => {
                                    let pending_obligations = snapshot.pending_obligations;
                                    let stored = snapshot.session;
                                    if initial_meta.is_some() {
                                        return Err("new send target already exists".to_string());
                                    }
                                    if session_projection_guard
                                        != crate::domain::local_event::RevisionGuard::Expected(
                                            stored.revision,
                                        )
                                    {
                                        return Err(
                                            "send allocation projection changed before acceptance"
                                                .to_string(),
                                        );
                                    }
                                    let decoded = codec.decode(&stored.projection)?;
                                    let session_aggregate = codec
                                        .restore_session_aggregate(&decoded, &pending_obligations)?;
                                    (
                                        decoded.meta,
                                        decoded.title,
                                        decoded.reducer_events,
                                        decoded.pending_send_queue,
                                        session_aggregate,
                                        session_projection_guard,
                                        stored.revision.next().ok_or_else(|| "send projection revision exhausted".to_string())?,
                                    )
                                }
                                None => {
                                    if session_projection_guard
                                        != crate::domain::local_event::RevisionGuard::Absent
                                    {
                                        return Err(
                                            "send allocation projection changed before acceptance"
                                                .to_string(),
                                        );
                                    }
                                    let meta = initial_meta.ok_or_else(|| {
                                        "send target projection is missing".to_string()
                                    })?;
                                    let session_aggregate =
                                        SessionAggregate::new(meta.id.clone()).map_err(|error| {
                                            format!("invalid initial Session aggregate: {error:?}")
                                        })?;
                                    (
                                        meta,
                                        None,
                                        Vec::new(),
                                        Vec::new(),
                                        session_aggregate,
                                        session_projection_guard,
                                        crate::domain::local_event::Revision::new(0).expect("zero revision"),
                                    )
                                }
                            };
                            let reducer_events = bounded_reducer_events(reducer_events, &events);
                            let projected = TurnEventLog::from_events(reducer_events.clone()).project();
                            let started_turn_id = events.iter().find_map(|event| match event {
                                AgentSessionEvent::TurnStarted { turn_id, .. } => Some(*turn_id),
                                _ => None,
                            });
                            if let Some(turn_id) = started_turn_id {
                                if meta
                                    .last_turn_id
                                    .is_some_and(|last_turn_id| {
                                        !turn_identity_advances(last_turn_id, turn_id)
                                    })
                                {
                                    return Err(
                                        "started send turn identity does not advance".to_string()
                                    );
                                }
                                match session_aggregate.start_turn(Turn::start(turn_id)) {
                                    TransitionOutcome::Applied => {}
                                    TransitionOutcome::Rejected(
                                        TransitionRejection::QueueNotEmpty,
                                    ) => {
                                        return Err(
                                            "started send cannot bypass the canonical queue front"
                                                .to_string(),
                                        );
                                    }
                                    TransitionOutcome::Rejected(
                                        TransitionRejection::UnresolvedRecovery,
                                    ) => {
                                        return Err(
                                            "started send cannot bypass unresolved recovery"
                                                .to_string(),
                                        );
                                    }
                                    _ => {
                                        return Err(
                                            "started send target is not quiescent".to_string()
                                        );
                                    }
                                }
                                meta.state = session_aggregate.state();
                                meta.error_reason = error_reason_for_state(
                                    &meta.state,
                                    &projected.error_reason,
                                );
                                meta.last_turn_id = Some(turn_id);
                            }
                            let started_turn = started_turn_id.is_some();
                            meta.state_revision = next_sqlite_counter(
                                meta.state_revision,
                                "session state revision",
                            )?;

                            let mut messages = projected
                                .messages
                                .into_iter()
                                .filter(|message| {
                                    message.id == human_message_id
                                        || (started_turn && message.role == MessageRole::Agent)
                                })
                                .collect::<Vec<_>>();
                            if !messages.iter().any(|message| message.id == human_message_id) {
                                messages.push(ChatMessage {
                                    id: human_message_id.clone(),
                                    role: MessageRole::Human,
                                    content: prompt.content.clone(),
                                    thinking: None,
                                    activities: None,
                                    parts: (!prompt.parts.is_empty()).then(|| prompt.parts.clone()),
                                    streaming_final_seq: 0,
                                    timestamp: now_timestamp(),
                                    mentions: (!prompt.mentions.is_empty()).then(|| {
                                        prompt
                                            .mentions
                                            .clone()
                                            .into_iter()
                                            .map(super::MessageMention::from_domain)
                                            .collect()
                                    }),
                                });
                            }

                            let content_blobs = authority
                                .projection_codec
                                .externalize_message_content(&mut messages)?;

                            let mut mutations = prepare_canonical_content_blob_mutations(
                                &authority.repository,
                                &session_id,
                                content_blobs,
                            )
                            .await?;
                            let mut inserted = Vec::new();
                            for message in messages {
                                let encoded = codec.encode_message(&message)?;
                                let stored = match authority
                                    .repository
                                    .query(crate::domain::local_event::LocalEventQuery::MessageProjectionByIdentity {
                                        session_id: session_id.clone(),
                                        message_id: message.id.clone(),
                                    })
                                    .await
                                    .map_err(|error| format!("send message projection read failed: {error}"))?
                                {
                                    crate::domain::local_event::LocalEventQueryResult::MessageProjectionByIdentity(stored) => stored,
                                    _ => return Err("send message query returned the wrong shape".to_string()),
                                };
                                let (message_expected, message_revision) = match stored {
                                    Some(stored) if stored.projection == encoded => continue,
                                    Some(stored) => (
                                        crate::domain::local_event::RevisionGuard::Expected(stored.revision),
                                        stored.revision.next().ok_or_else(|| "send message revision exhausted".to_string())?,
                                    ),
                                    None => {
                                        inserted.push(message.clone());
                                        (
                                            crate::domain::local_event::RevisionGuard::Absent,
                                            crate::domain::local_event::Revision::new(0).expect("zero revision"),
                                        )
                                    }
                                };
                                mutations.push(crate::domain::local_event::LocalStateMutation::MessageProjection(
                                    crate::domain::local_event::MessageProjectionMutation {
                                        session_id: session_id.clone(),
                                        message_id: message.id,
                                        projection: encoded,
                                        expected: message_expected,
                                        revision: message_revision,
                                    },
                                ));
                            }
                            meta.message_count = add_sqlite_count(
                                meta.message_count,
                                inserted.len(),
                                "session message count",
                            )?;
                            if meta.first_message_preview.is_empty() {
                                meta.first_message_preview = super::first_message_preview(&inserted);
                            }
                            meta.updated_at = now_timestamp();
                            if let crate::domain::agent_session::events::SendDisposition::Queued {
                                queue_item_id,
                            } = &disposition
                            {
                                let reserved_turn_id = reserved_turn_id.clone().ok_or_else(|| {
                                    "queued send is missing its reserved turn identity".to_string()
                                })?;
                                if !pending_send_queue
                                    .iter()
                                    .any(|entry| entry.queue_item_id == *queue_item_id)
                                {
                                    pending_send_queue.push(CanonicalQueuedSend {
                                        queue_item_id: queue_item_id.clone(),
                                        human_message_id: human_message_id.clone(),
                                        reserved_turn_id,
                                        input_ref: input_ref.clone(),
                                    });
                                }
                            }
                            let projection = codec.encode(&CanonicalAgentSessionProjection {
                                meta,
                                title,
                                messages: Vec::new(),
                                reducer_events,
                                queue_paused_at: projected.queue_paused_at,
                                latest_token_usage: None,
                                pending_send_queue,
                            })?;
                            mutations.insert(0, crate::domain::local_event::LocalStateMutation::SessionProjection(
                                crate::domain::local_event::SessionProjectionMutation {
                                    session_id,
                                    projection,
                                    expected,
                                    revision,
                                },
                            ));
                            Ok(mutations)
                        })
                })
                .join()
                .map_err(|_| "send projection worker panicked".to_string())?
        })
    }

    fn commit_meta_projection_snapshot(&self, meta: SessionMeta) -> Result<(), String> {
        self.commit_meta_projection_snapshot_with_kind(
            meta,
            crate::domain::local_event::CommitOperationKind::Projection,
        )
    }

    fn commit_user_meta_projection_snapshot(&self, meta: SessionMeta) -> Result<(), String> {
        self.commit_meta_projection_snapshot_with_kind(
            meta,
            crate::domain::local_event::CommitOperationKind::UserMutation,
        )
    }

    fn commit_meta_projection_snapshot_with_kind(
        &self,
        meta: SessionMeta,
        operation_kind: crate::domain::local_event::CommitOperationKind,
    ) -> Result<(), String> {
        let current = self.read_session_projection(&meta.id)?;
        let queue_paused_at = current
            .as_ref()
            .and_then(|projection| projection.queue_paused_at);
        let title = current
            .as_ref()
            .and_then(|projection| projection.title.clone());
        let reducer_events = current
            .as_ref()
            .map(|projection| projection.reducer_events.clone())
            .unwrap_or_default();
        let pending_send_queue = current
            .map(|projection| projection.pending_send_queue)
            .unwrap_or_default();
        self.commit_session_projection_snapshot_with_kind(
            CanonicalAgentSessionProjection {
                meta,
                title,
                messages: Vec::new(),
                reducer_events,
                queue_paused_at,
                latest_token_usage: None,
                pending_send_queue,
            },
            operation_kind,
        )
    }

    #[cfg(test)]
    fn remove_read_session_projection(&self, session_id: &str) -> Result<(), String> {
        let Some(authority) = self.event_authority.read().clone() else {
            #[cfg(test)]
            return Ok(());
            #[cfg(not(test))]
            return Err("agent-session SQLite event authority is not configured".to_string());
        };
        let session_id = session_id.to_string();
        std::thread::scope(|scope| {
            scope
                .spawn(move || {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|error| {
                            format!("failed to create agent rollback runtime: {error}")
                        })?
                        .block_on(async move {
                            let result = authority
                                .repository
                                .query(crate::domain::local_event::LocalEventQuery::SessionProjectionByIdentity {
                                    session_id: session_id.clone(),
                                })
                                .await
                                .map_err(|error| {
                                    format!("agent rollback projection read failed: {error}")
                                })?;
                            let crate::domain::local_event::LocalEventQueryResult::SessionProjectionByIdentity(
                                stored,
                            ) = result
                            else {
                                return Err("agent rollback projection query returned the wrong shape".to_string());
                            };
                            let Some(stored) = stored else {
                                return Ok(());
                            };
                            let binding =
                                crate::domain::local_event::session_projection_rollback_identity(
                                    &session_id,
                                    stored.revision.value(),
                                );
                            let binding_hash = binding.digest;
                            let identity = binding.identity;
                            let commit_identity =
                                crate::domain::local_event::CommitIdentity::parse(&identity)
                                    .map_err(|_| {
                                        "agent rollback commit identity is invalid".to_string()
                                    })?;
                            let batch = crate::domain::local_event::LocalAtomicBatch {
                                commit_id: commit_identity.clone(),
                                idempotency: crate::domain::local_event::IdempotencyBinding {
                                    installation_id: authority.installation_id.clone(),
                                    operation_kind: crate::domain::local_event::CommitOperationKind::Projection,
                                    idempotency_key: hex_lower(binding_hash),
                                    payload_hash: binding_hash,
                                },
                                expected_heads: Vec::new(),
                                events: Vec::new(),
                                state_mutations: vec![
                                    crate::domain::local_event::LocalStateMutation::SessionProjectionRemoval(
                                        crate::domain::local_event::SessionProjectionRemovalMutation {
                                            session_id,
                                            expected: crate::domain::local_event::RevisionGuard::Expected(
                                                stored.revision,
                                            ),
                                        },
                                    ),
                                ],
                            };
                            match authority.repository.commit_batch(batch).await {
                                Ok(_) => Ok(()),
                                Err(crate::domain::local_event::CommitBatchError::OutcomeUnknown {
                                    ..
                                }) => match authority
                                    .repository
                                    .resolve_commit(commit_identity)
                                    .await
                                    .map_err(|error| {
                                        format!("agent rollback readback failed: {error}")
                                    })?
                                {
                                    crate::domain::local_event::CommitResolution::Committed(_) => {
                                        Ok(())
                                    }
                                    crate::domain::local_event::CommitResolution::NotCommitted => {
                                        Err("agent rollback was not committed".to_string())
                                    }
                                },
                                Err(error) => {
                                    Err(format!("agent rollback commit failed: {error}"))
                                }
                            }
                        })
                })
                .join()
                .map_err(|_| "agent rollback worker panicked".to_string())?
        })
    }

    #[cfg(test)]
    pub(crate) fn set_save_hook_for_test(&self, hook: SessionSaveHook) {
        *self.save_hook.write() = Some(hook);
    }

    #[cfg(test)]
    pub(crate) fn set_append_message_hook_for_test(&self, hook: SessionAppendMessageHook) {
        *self.append_message_hook.write() = Some(hook);
    }

    #[cfg(test)]
    pub(crate) fn set_persist_parts_hook_for_test(&self, hook: SessionPersistPartsHook) {
        *self.persist_parts_hook.write() = Some(hook);
    }

    #[cfg(test)]
    pub(crate) fn set_append_event_hook_for_test(&self, hook: SessionAppendEventHook) {
        *self.append_event_hook.write() = Some(hook);
    }

    #[cfg(test)]
    pub(crate) fn set_state_hook_for_test(&self, hook: SessionSetStateHook) {
        *self.set_state_hook.write() = Some(hook);
    }

    #[cfg(test)]
    pub(crate) fn set_projection_hook_for_test(&self, hook: SessionProjectionHook) {
        *self.projection_hook.write() = Some(hook);
    }

    #[cfg(test)]
    pub(crate) fn set_appended_event_hook_for_test(&self, hook: SessionAppendedEventHook) {
        *self.appended_event_hook.write() = Some(hook);
    }

    #[cfg(test)]
    pub(crate) fn set_event_projection_hook_for_test(&self, hook: SessionEventProjectionHook) {
        *self.event_projection_hook.write() = Some(hook);
    }

    #[cfg(test)]
    pub(crate) fn set_atomic_event_commit_hook_for_test(&self, hook: SessionAtomicEventCommitHook) {
        *self.atomic_event_commit_hook.write() = Some(hook);
    }

    #[cfg(test)]
    pub(crate) fn set_backend_established_hook_for_test(
        &self,
        hook: SessionBackendEstablishedHook,
    ) {
        *self.backend_established_hook.write() = Some(hook);
    }

    #[cfg(test)]
    pub(crate) fn set_projected_read_model_hook_for_test(
        &self,
        hook: SessionProjectedReadModelHook,
    ) {
        *self.projected_read_model_hook.write() = Some(hook);
    }

}
