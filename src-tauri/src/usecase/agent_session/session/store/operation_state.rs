impl SessionStore {
    /// Report whether a durably accepted queue front may claim the next turn.
    ///
    /// Older builds persisted a newly-created session as `Active` before any
    /// `TurnStarted` event existed. The bounded reducer is the canonical turn
    /// owner, so that legacy label must not strand the accepted first send.
    pub(crate) fn accepted_queue_start_readiness(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<bool>, String> {
        if self.canonical_authority_active() {
            return self.read_session_projection(session_id).map(|projection| {
                projection.map(|projection| {
                    reducer_allows_queue_start(&projection.meta.state, &projection.reducer_events)
                })
            });
        }
        self.get_session_meta(app_data_dir, session_id)
            .map(|meta| meta.map(|meta| meta.state.permits_legacy_queue_start()))
    }

    /// Read the canonical owner of an immediately accepted provider turn.
    ///
    /// This is a preflight optimization for the runtime adapter. The SQLite
    /// writer repeats the same check in the claim transaction, which closes
    /// the read/claim race.
    pub(crate) fn canonical_active_turn_matches(
        &self,
        session_id: &str,
        turn_id: u64,
    ) -> Result<bool, String> {
        Ok(self
            .read_session_projection(session_id)?
            .is_some_and(|projection| {
                SessionAggregate::canonical_active_turn_matches(
                    &projection.reducer_events,
                    projection.meta.last_turn_id,
                    turn_id,
                )
            }))
    }

    pub fn append_session_event_without_projection(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        event: AgentSessionEvent,
    ) -> Result<(), String> {
        #[cfg(test)]
        if let Some(hook) = self.append_event_hook.read().clone() {
            hook(session_id, &event)?;
        }
        self.commit_agent_events(app_data_dir, session_id, std::slice::from_ref(&event))?;
        if !self.canonical_authority_active() {
            #[cfg(test)]
            {
                self.test_storage()
                    .append_session_event_without_projection(app_data_dir, session_id, &event)?;
                if self.test_storage().take_event_log_recovered(session_id) {
                    self.notify_event_log_recovered(session_id);
                }
                return Ok(());
            }
            #[cfg(not(test))]
            return Err("agent-session SQLite event authority is not configured".to_string());
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn reserve_permission_response(
        &self,
        _app_data_dir: &Path,
        session_id: &str,
        turn_id: u64,
        request_id: &str,
        exact_response: crate::domain::agent_session::entities::PermissionResponse,
    ) -> Result<String, String> {
        let obligation_id = format!("permission-response:{session_id}:{turn_id}:{request_id}");
        let turn_id_text = turn_id.to_string();
        let at = now_timestamp();
        if exact_response.request_id != request_id {
            return Err("permission response request identity is inconsistent".to_string());
        }
        let record = crate::domain::local_event::ObligationRecord::PermissionResponse {
            operation_id: obligation_id.clone(),
            effect_identity: obligation_id.clone(),
            session_id: session_id.to_string(),
            turn_id: turn_id_text.clone(),
            response: exact_response,
            owner_access: true,
            from_runtime_state: true,
            state: crate::domain::local_event::ObligationStateRecord::Pending,
        };

        let matches_pending = |stored: &crate::domain::local_event::ObligationRecord| {
            matches!(
                stored,
                crate::domain::local_event::ObligationRecord::PermissionResponse {
                    operation_id,
                    effect_identity,
                    session_id: stored_session_id,
                    turn_id: stored_turn_id,
                    response,
                    owner_access: true,
                    from_runtime_state: true,
                    state: crate::domain::local_event::ObligationStateRecord::Pending,
                } if operation_id == &obligation_id
                    && effect_identity == &obligation_id
                    && stored_session_id == session_id
                    && stored_turn_id == &turn_id_text
                    && response == match &record {
                        crate::domain::local_event::ObligationRecord::PermissionResponse { response, .. } => response,
                        _ => unreachable!(),
                    }
            )
        };

        if !self.canonical_authority_active() {
            #[cfg(test)]
            {
                let mut reservations = self.permission_response_reservations.write();
                if let Some(current) = reservations.get(&obligation_id) {
                    if matches_pending(current) {
                        return Ok(obligation_id);
                    }
                    return Err(format!(
                        "permission response {request_id} already has a claimed effect and requires reconciliation"
                    ));
                }
                reservations.insert(obligation_id.clone(), record);
                return Ok(obligation_id);
            }
            #[cfg(not(test))]
            return Err(
                "permission responses require the canonical local-event authority".to_string(),
            );
        }
        if let Some(current) = self.canonical_obligation(&obligation_id)? {
            if matches_pending(&current.record) {
                return Ok(obligation_id);
            }
            return Err(format!(
                "permission response {request_id} already has a claimed effect and requires reconciliation"
            ));
        }
        let obligation = backend_recovery_obligation_mutation(
            obligation_id.clone(),
            record.clone(),
            Some(crate::domain::local_event::PendingIndexEntry {
                ordered_key: format!(
                    "permission-response:{:020}:{obligation_id}",
                    (at * 1000.0).round() as i64
                ),
                owner: session_id.to_string(),
                partition: crate::domain::local_event::PendingPartition::Owner,
                shutdown_plan: None,
            }),
            None,
        )?;
        let event = AgentSessionEvent::ObligationRecorded {
            obligation_id: obligation_id.clone(),
            kind: crate::domain::agent_session::events::ObligationKind::PermissionResponse,
            state: crate::domain::agent_session::events::ObligationState::Pending,
            at,
        };
        self.commit_agent_events_with_additional_mutations(
            session_id,
            std::slice::from_ref(&event),
            vec![obligation],
            None,
            None,
            None,
            crate::domain::local_event::CommitOperationKind::Recovery,
        )?;
        let fresh = self.canonical_obligation(&obligation_id)?.ok_or_else(|| {
            "permission response reservation was not readable after commit".to_string()
        })?;
        if fresh.record != record
            || fresh.pending.as_ref().map(|pending| pending.owner.as_str()) != Some(session_id)
        {
            return Err(
                "permission response reservation fresh-read did not match the accepted response"
                    .to_string(),
            );
        }
        Ok(obligation_id)
    }

    #[cfg(test)]
    pub(crate) fn claim_permission_response_effect(
        &self,
        session_id: &str,
        obligation_id: &str,
    ) -> Result<(), String> {
        if !self.canonical_authority_active() {
            #[cfg(test)]
            {
                let mut reservations = self.permission_response_reservations.write();
                let stored = reservations.get_mut(obligation_id).ok_or_else(|| {
                    "permission response claim has no durable reservation".to_string()
                })?;
                match stored {
                    crate::domain::local_event::ObligationRecord::PermissionResponse {
                        session_id: stored_session_id,
                        state,
                        ..
                    } if stored_session_id == session_id
                        && *state == crate::domain::local_event::ObligationStateRecord::Pending =>
                    {
                        *state = crate::domain::local_event::ObligationStateRecord::EffectReserved;
                    }
                    _ => {
                        return Err(
							"permission response effect was already claimed and requires reconciliation"
								.to_string(),
						);
                    }
                }
                return Ok(());
            }
            #[cfg(not(test))]
            return Err(
                "permission responses require the canonical local-event authority".to_string(),
            );
        }

        let current = self
            .canonical_obligation(obligation_id)?
            .ok_or_else(|| "permission response claim has no durable reservation".to_string())?;
        let mut record = current.record.clone();
        match &mut record {
            crate::domain::local_event::ObligationRecord::PermissionResponse {
                session_id: stored_session_id,
                state,
                ..
            } if stored_session_id == session_id
                && *state == crate::domain::local_event::ObligationStateRecord::Pending =>
            {
                *state = crate::domain::local_event::ObligationStateRecord::EffectReserved;
            }
            _ => {
                return Err(
                    "permission response effect was already claimed and requires reconciliation"
                        .to_string(),
                );
            }
        }
        let at = now_timestamp();
        let pending =
            current
                .pending
                .as_ref()
                .map(|pending| crate::domain::local_event::PendingIndexEntry {
                    ordered_key: pending.ordered_key.clone(),
                    owner: pending.owner.clone(),
                    partition: pending.partition,
                    shutdown_plan: pending.shutdown_plan.clone(),
                });
        let obligation = backend_recovery_obligation_mutation(
            obligation_id.to_string(),
            record,
            pending,
            Some(&current),
        )?;
        let event = AgentSessionEvent::ObligationRecorded {
            obligation_id: obligation_id.to_string(),
            kind: crate::domain::agent_session::events::ObligationKind::PermissionResponse,
            state: crate::domain::agent_session::events::ObligationState::EffectReserved,
            at,
        };
        self.commit_agent_events_with_additional_mutations(
            session_id,
            std::slice::from_ref(&event),
            vec![obligation],
            None,
            None,
            None,
            crate::domain::local_event::CommitOperationKind::Recovery,
        )?;
        let fresh = self
            .canonical_obligation(obligation_id)?
            .ok_or_else(|| "permission response claim was not readable after commit".to_string())?;
        if !matches!(
            fresh.record,
            crate::domain::local_event::ObligationRecord::PermissionResponse {
                state: crate::domain::local_event::ObligationStateRecord::EffectReserved,
                ..
            }
        ) {
            return Err(
                "permission response claim outcome could not be verified; provider effect was not started"
                    .to_string(),
            );
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn load_permission_response_obligation(
        &self,
        obligation_id: &str,
    ) -> Result<Option<crate::domain::local_event::ObligationStateRecord>, String> {
        if self.canonical_authority_active() {
            return self.canonical_obligation(obligation_id).map(|obligation| {
                obligation.and_then(|obligation| match obligation.record {
                    crate::domain::local_event::ObligationRecord::PermissionResponse {
                        state,
                        ..
                    } => Some(state),
                    _ => None,
                })
            });
        }
        Ok(self
            .permission_response_reservations
            .read()
            .get(obligation_id)
            .and_then(|record| match record {
                crate::domain::local_event::ObligationRecord::PermissionResponse {
                    state, ..
                } => Some(*state),
                _ => None,
            }))
    }

    #[cfg(test)]
    pub(crate) fn complete_permission_response(
        &self,
        _app_data_dir: &Path,
        session_id: &str,
        obligation_id: &str,
        resolved_event: AgentSessionEvent,
        message_id: Option<&str>,
        streaming_final_seq: Option<u64>,
    ) -> Result<(), String> {
        if !self.canonical_authority_active() {
            #[cfg(test)]
            {
                let state = self
                    .permission_response_reservations
                    .read()
                    .get(obligation_id)
                    .and_then(|record| match record {
                        crate::domain::local_event::ObligationRecord::PermissionResponse {
                            state,
                            ..
                        } => Some(*state),
                        _ => None,
                    });
                if state != Some(crate::domain::local_event::ObligationStateRecord::EffectReserved)
                {
                    return Err("permission response completion has no claimed effect".to_string());
                }
                if let (Some(message_id), Some(streaming_final_seq)) =
                    (message_id, streaming_final_seq)
                {
                    let mut events = self.load_session_events(_app_data_dir, session_id)?;
                    events.push(resolved_event.clone());
                    let projected = TurnEventLog::from_events(events).project();
                    self.persist_message_parts(
                        _app_data_dir,
                        session_id,
                        message_id,
                        &projected.agent_parts_for_message(message_id),
                        streaming_final_seq,
                        None,
                    )?;
                }
                self.append_session_event_and_project_state(
                    _app_data_dir,
                    session_id,
                    resolved_event,
                )?;
                self.permission_response_reservations
                    .write()
                    .remove(obligation_id);
                return Ok(());
            }
            #[cfg(not(test))]
            return Err(
                "permission responses require the canonical local-event authority".to_string(),
            );
        }
        let current = self.canonical_obligation(obligation_id)?.ok_or_else(|| {
            "permission response completion has no durable reservation".to_string()
        })?;
        let mut record = current.record.clone();
        match &mut record {
            crate::domain::local_event::ObligationRecord::PermissionResponse {
                state: crate::domain::local_event::ObligationStateRecord::Completed,
                ..
            } => return Ok(()),
            crate::domain::local_event::ObligationRecord::PermissionResponse { state, .. }
                if *state == crate::domain::local_event::ObligationStateRecord::EffectReserved =>
            {
                *state = crate::domain::local_event::ObligationStateRecord::Completed;
            }
            _ => {
                return Err("permission response reservation requires reconciliation".to_string());
            }
        }
        let at = now_timestamp();
        let obligation = backend_recovery_obligation_mutation(
            obligation_id.to_string(),
            record,
            None,
            Some(&current),
        )?;
        let events = vec![
            resolved_event,
            AgentSessionEvent::ObligationRecorded {
                obligation_id: obligation_id.to_string(),
                kind: crate::domain::agent_session::events::ObligationKind::PermissionResponse,
                state: crate::domain::agent_session::events::ObligationState::Completed,
                at,
            },
        ];
        let message_patch = match (message_id, streaming_final_seq) {
            (Some(message_id), Some(streaming_final_seq)) => Some(TerminalMessageProjectionPatch {
                message_id: message_id.to_string(),
                streaming_final_seq,
                timestamp: None,
                parts: None,
            }),
            (None, None) => None,
            _ => {
                return Err("permission response message projection is incomplete".to_string());
            }
        };
        self.commit_agent_events_with_additional_mutations(
            session_id,
            &events,
            vec![obligation],
            message_patch,
            None,
            None,
            crate::domain::local_event::CommitOperationKind::Recovery,
        )
    }

    pub fn begin_backend_session_recovery(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        recovery_id: &str,
        reason: BackendSessionRecoveryReason,
    ) -> Result<BackendSessionRecoveryStartOutcome, String> {
        self.ensure_canonical_mutation_admission()?;
        let current_meta = self.require_meta(app_data_dir, session_id)?;
        let old_provider_session_generation = current_meta.provider_session_generation;
        let projection_codec = self
            .event_authority
            .read()
            .as_ref()
            .map(|authority| authority.projection_codec.clone());
        let publication_decision = decide_recovery_publication(current_meta.state);
        let publication_snapshot = match projection_codec {
            Some(codec) => codec.recovery_publication_snapshot(
                recovery_id,
                &current_meta,
                publication_decision,
            ),
            #[cfg(test)]
            None => {
                test_recovery_publication_snapshot(
                    recovery_id,
                    &current_meta,
                    publication_decision,
                )
            }
            #[cfg(not(test))]
            None => unreachable!("production mutation admission requires a projection codec"),
        };
        let at = now_timestamp();
        let event = AgentSessionEvent::BackendSessionRecoveryStarted {
            recovery_id: recovery_id.to_string(),
            old_provider_session_generation,
            reason,
            at,
        };
        #[cfg(test)]
        if let Some(hook) = self.append_event_hook.read().clone() {
            hook(session_id, &event)?;
        }
        if !self.canonical_authority_active() {
            #[cfg(test)]
            {
                let mut updated = None;
                self.test_storage()
                    .update_session_meta_and_append_session_events(
                app_data_dir,
                session_id,
                &mut |meta| {
                    let actual_generation = meta.provider_session_generation;
                    let mut state = test_domain_backend_recovery_meta(meta, false);
                    state
                        .start(
                            old_provider_session_generation,
                            publication_snapshot.summary.state,
                            publication_snapshot.summary.error_reason.clone(),
                        )
                        .map_err(|rejection| match rejection {
                            BackendRecoveryProjectionRejection::StaleProviderGeneration => format!(
                                "Backend session generation changed while starting recovery: expected {old_provider_session_generation}, actual {actual_generation}"
                            ),
                            _ => "backend recovery start decision is inconsistent".to_string(),
                        })?;
                    test_apply_domain_backend_recovery_meta(meta, state);
                    meta.recovery_publication_snapshot = Some(publication_snapshot.clone());
                    meta.updated_at = at;
                    updated = Some(meta.clone());
                    Ok(())
                },
                std::slice::from_ref(&event),
            )?;
                return updated
                    .map(Box::new)
                    .map(BackendSessionRecoveryStartOutcome::Started)
                    .ok_or_else(|| format!("Session not found: {session_id}"));
            }
            #[cfg(not(test))]
            unreachable!("production mutation admission rejects a missing SQLite authority");
        }

        let obligation_id = backend_recovery_obligation_id(session_id, recovery_id);
        let current = self.canonical_obligation(&obligation_id)?;
        match admit_backend_recovery_start(
            current.as_ref().map(|current| &current.record),
            session_id,
            recovery_id,
        ) {
            Ok(BackendRecoveryReservationDecision::Apply) => {}
            Ok(BackendRecoveryReservationDecision::AlreadyApplied) => {
                return self
                    .get_session_meta(app_data_dir, session_id)?
                    .map(Box::new)
                    .map(BackendSessionRecoveryStartOutcome::Started)
                    .ok_or_else(|| format!("Session not found: {session_id}"));
            }
            Err(BackendRecoveryReservationRejection::AlreadyResolved) => {
                return Err("backend recovery identity was already resolved".to_string());
            }
            Err(_) => {
                return Err("backend recovery obligation identity is inconsistent".to_string());
            }
        }
        let obligation_record =
            crate::domain::local_event::ObligationRecord::BackendSessionRecovery {
            session_id: session_id.to_string(),
            recovery_id: recovery_id.to_string(),
                detail:
                    crate::domain::local_event::BackendSessionRecoveryObligationRecord::EffectReserved {
                        old_provider_session_generation,
                        reason,
                        reserved_at_bits: at.to_bits(),
                    },
                state: crate::domain::local_event::ObligationStateRecord::EffectReserved,
            };
        let obligation = backend_recovery_obligation_mutation(
            obligation_id.clone(),
            obligation_record,
            Some(crate::domain::local_event::PendingIndexEntry {
                ordered_key: format!("{:020}:{obligation_id}", (at * 1000.0).round() as i64),
                owner: session_id.to_string(),
                partition: crate::domain::local_event::PendingPartition::Owner,
                shutdown_plan: None,
            }),
            None,
        )?;
        match self.commit_agent_events_with_additional_mutations(
            session_id,
            std::slice::from_ref(&event),
            vec![obligation],
            None,
            Some(EventProjectionMetaPatch::Started {
                expected_generation: old_provider_session_generation,
                publication_snapshot: Box::new(publication_snapshot),
                at,
            }),
            None,
            crate::domain::local_event::CommitOperationKind::Recovery,
        ) {
            Ok(()) => {}
            Err(error) if error == BACKEND_RECOVERY_START_SUPPRESSED_BY_QUEUE_PAUSE => {
                return Ok(BackendSessionRecoveryStartOutcome::SuppressedByQueuePause);
            }
            Err(error) => return Err(error),
        }
        self.get_session_meta(app_data_dir, session_id)?
            .map(Box::new)
            .map(BackendSessionRecoveryStartOutcome::Started)
            .ok_or_else(|| format!("Session not found: {session_id}"))
    }

    pub fn complete_backend_session_recovery(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        recovery_id: &str,
        old_provider_session_generation: u64,
        backend_session_id: String,
    ) -> Result<SessionMeta, String> {
        self.ensure_canonical_mutation_admission()?;
        let provider_session_generation = next_sqlite_counter(
            old_provider_session_generation,
            "provider session generation",
        )?;
        let at = now_timestamp();
        let pending_recovery_message = PendingRecoveryMessage::Notice {
            recovery_id: recovery_id.to_string(),
            message_id: uuid::Uuid::new_v4().to_string(),
        };
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
        #[cfg(test)]
        if let Some(hook) = self.append_event_hook.read().clone() {
            for event in &events {
                hook(session_id, event)?;
            }
        }
        if !self.canonical_authority_active() {
            #[cfg(test)]
            {
                let mut updated = None;
                self.test_storage()
                    .update_session_meta_and_append_session_events(
                app_data_dir,
                session_id,
                &mut |meta| {
                    let actual_generation = meta.provider_session_generation;
                    let mut state = test_domain_backend_recovery_meta(meta, false);
                    state
                        .complete(
                            old_provider_session_generation,
                            provider_session_generation,
                            backend_session_id.clone(),
                            backend_recovery_provider_observation_id(recovery_id),
                        )
                        .map_err(|rejection| match rejection {
                            BackendRecoveryProjectionRejection::StaleProviderGeneration => format!(
                                "Backend session generation changed while completing recovery: expected {old_provider_session_generation}, actual {actual_generation}"
                            ),
                            _ => "backend recovery completion decision is inconsistent".to_string(),
                        })?;
                    test_apply_domain_backend_recovery_meta(meta, state);
                    meta.pending_recovery_message = Some(pending_recovery_message.clone());
                    meta.recovery_publication_snapshot = None;
                    meta.updated_at = at;
                    updated = Some(meta.clone());
                    Ok(())
                },
                &events,
            )?;
                return updated.ok_or_else(|| format!("Session not found: {session_id}"));
            }
            #[cfg(not(test))]
            unreachable!("production mutation admission rejects a missing SQLite authority");
        }

        let projection_codec = self
            .event_authority
            .read()
            .as_ref()
            .expect("canonical mutation admission requires SQLite authority")
            .projection_codec
            .clone();
        let publication_message =
            projection_codec.recovery_publication_message_record(&pending_recovery_message);
        let obligation_id = backend_recovery_obligation_id(session_id, recovery_id);
        let current = self
            .canonical_obligation(&obligation_id)?;
        match admit_backend_recovery_completion(
            current.as_ref().map(|current| &current.record),
            session_id,
            recovery_id,
            old_provider_session_generation,
        ) {
            Ok(BackendRecoveryReservationDecision::AlreadyApplied) => {
                return self
                    .get_session_meta(app_data_dir, session_id)?
                    .ok_or_else(|| format!("Session not found: {session_id}"));
            }
            Ok(BackendRecoveryReservationDecision::Apply) => {}
            Err(BackendRecoveryReservationRejection::Missing) => {
                return Err(
                    "backend recovery completion has no durable reservation".to_string(),
                );
            }
            Err(BackendRecoveryReservationRejection::NotPending) => {
                return Err("backend recovery reservation is not pending".to_string());
            }
            Err(_) => {
                return Err("backend recovery obligation identity is inconsistent".to_string());
            }
        }
        let current = current.expect("domain admission requires the durable reservation");
        let obligation_record =
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
            };
        let obligation = backend_recovery_obligation_mutation(
            obligation_id.clone(),
            obligation_record,
            None,
            Some(&current),
        )?;
        let publication_obligation_id = recovery_publication_obligation_id(
            session_id,
            &publication_message.recovery_id,
            &publication_message.message_id,
        );
        let publication = recovery_publication_obligation_mutation(
            publication_obligation_id.clone(),
            crate::domain::local_event::ObligationRecord::RecoveryPublication {
                session_id: session_id.to_string(),
                recovery_id: publication_message.recovery_id.clone(),
                message_id: publication_message.message_id.clone(),
                source_obligation_id: obligation_id,
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
        self.commit_agent_events_with_additional_mutations(
            session_id,
            &events,
            vec![obligation, publication],
            None,
            Some(EventProjectionMetaPatch::Completed {
                expected_generation: old_provider_session_generation,
                provider_session_generation,
                backend_session_id,
                pending_recovery_message,
                at,
            }),
            None,
            crate::domain::local_event::CommitOperationKind::Recovery,
        )?;
        self.get_session_meta(app_data_dir, session_id)?
            .ok_or_else(|| format!("Session not found: {session_id}"))
    }

    pub fn fail_backend_session_recovery(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        recovery_id: &str,
        error: &str,
    ) -> Result<SessionMeta, String> {
        self.ensure_canonical_mutation_admission()?;
        let at = now_timestamp();
        let fallback_message_id =
            || backend_recovery_failure_message_id(session_id, recovery_id);
        let message_id = if self.canonical_authority_active() {
            let current_projection = self
                .read_session_projection(session_id)?
                .ok_or_else(|| format!("Session projection not found: {session_id}"))?;
            TurnEventLog::from_events(current_projection.reducer_events)
                .project()
                .messages
                .into_iter()
                .rev()
                .find(|message| message.role == super::MessageRole::Agent)
                .map(|message| message.id)
                .unwrap_or_else(fallback_message_id)
        } else {
            #[cfg(test)]
            {
                self.load_full_session_for_restore(app_data_dir, session_id)?
                    .and_then(|session| {
                        session
                            .messages
                            .into_iter()
                            .rev()
                            .find(|message| message.role == super::MessageRole::Agent)
                            .map(|message| message.id)
                    })
                    .unwrap_or_else(fallback_message_id)
            }
            #[cfg(not(test))]
            unreachable!("production mutation admission rejects a missing SQLite authority");
        };
        let pending_recovery_message = PendingRecoveryMessage::Error {
            recovery_id: recovery_id.to_string(),
            message_id,
            error: error.to_string(),
        };
        let event = AgentSessionEvent::BackendSessionRecoveryFailed {
            recovery_id: recovery_id.to_string(),
            error: error.to_string(),
            at,
        };
        #[cfg(test)]
        if let Some(hook) = self.append_event_hook.read().clone() {
            hook(session_id, &event)?;
        }
        if !self.canonical_authority_active() {
            #[cfg(test)]
            {
                let mut updated = None;
                self.test_storage()
                    .update_session_meta_and_append_session_events(
                        app_data_dir,
                        session_id,
                        &mut |meta| {
                            let mut state = test_domain_backend_recovery_meta(meta, false);
                            state.fail(Some(error.to_string()));
                            test_apply_domain_backend_recovery_meta(meta, state);
                            meta.pending_recovery_message = Some(pending_recovery_message.clone());
                            meta.recovery_publication_snapshot = None;
                            meta.updated_at = at;
                            updated = Some(meta.clone());
                            Ok(())
                        },
                        std::slice::from_ref(&event),
                    )?;
                return updated.ok_or_else(|| format!("Session not found: {session_id}"));
            }
            #[cfg(not(test))]
            unreachable!("production mutation admission rejects a missing SQLite authority");
        }

        let projection_codec = self
            .event_authority
            .read()
            .as_ref()
            .expect("canonical mutation admission requires SQLite authority")
            .projection_codec
            .clone();
        let publication_message =
            projection_codec.recovery_publication_message_record(&pending_recovery_message);
        let obligation_id = backend_recovery_obligation_id(session_id, recovery_id);
        let current = self
            .canonical_obligation(&obligation_id)?;
        match admit_backend_recovery_failure(
            current.as_ref().map(|current| &current.record),
            session_id,
            recovery_id,
        ) {
            Ok(BackendRecoveryReservationDecision::AlreadyApplied) => {
                return self
                    .get_session_meta(app_data_dir, session_id)?
                    .ok_or_else(|| format!("Session not found: {session_id}"));
            }
            Ok(BackendRecoveryReservationDecision::Apply) => {}
            Err(BackendRecoveryReservationRejection::Missing) => {
                return Err("backend recovery failure has no durable reservation".to_string());
            }
            Err(BackendRecoveryReservationRejection::NotPending) => {
                return Err("backend recovery reservation is not pending".to_string());
            }
            Err(_) => {
                return Err("backend recovery obligation identity is inconsistent".to_string());
            }
        }
        let current = current.expect("domain admission requires the durable reservation");
        let error_digest = backend_recovery_error_digest(error);
        let obligation_record =
            crate::domain::local_event::ObligationRecord::BackendSessionRecovery {
                session_id: session_id.to_string(),
                recovery_id: recovery_id.to_string(),
                detail:
                    crate::domain::local_event::BackendSessionRecoveryObligationRecord::Failed {
                        error_sha256: error_digest,
                        failed_at_bits: at.to_bits(),
                    },
                state: crate::domain::local_event::ObligationStateRecord::Failed,
            };
        let obligation = backend_recovery_obligation_mutation(
            obligation_id.clone(),
            obligation_record,
            None,
            Some(&current),
        )?;
        let publication_obligation_id = recovery_publication_obligation_id(
            session_id,
            &publication_message.recovery_id,
            &publication_message.message_id,
        );
        let publication = recovery_publication_obligation_mutation(
            publication_obligation_id.clone(),
            crate::domain::local_event::ObligationRecord::RecoveryPublication {
                session_id: session_id.to_string(),
                recovery_id: publication_message.recovery_id.clone(),
                message_id: publication_message.message_id.clone(),
                source_obligation_id: obligation_id,
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
        self.commit_agent_events_with_additional_mutations(
            session_id,
            std::slice::from_ref(&event),
            vec![obligation, publication],
            None,
            Some(EventProjectionMetaPatch::Failed {
                pending_recovery_message,
                at,
            }),
            None,
            crate::domain::local_event::CommitOperationKind::Recovery,
        )?;
        self.get_session_meta(app_data_dir, session_id)?
            .ok_or_else(|| format!("Session not found: {session_id}"))
    }

    #[cfg(test)]
    pub(crate) fn clear_pending_recovery_message(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        delivered: &PendingRecoveryMessage,
    ) -> Result<(), String> {
        self.update_meta_only(app_data_dir, session_id, |meta| {
            if meta.pending_recovery_message.as_ref() == Some(delivered) {
                meta.pending_recovery_message = None;
            }
            Ok(())
        })?;
        Ok(())
    }

    pub(crate) fn publish_pending_recovery_message(
        &self,
        _app_data_dir: &Path,
        session_id: &str,
        pending: &PendingRecoveryMessage,
        message: ChatMessage,
    ) -> Result<bool, String> {
        self.ensure_canonical_mutation_admission()?;
        if !self.canonical_authority_active() {
            #[cfg(test)]
            {
                let exists = self
                    .load_full_session_for_restore(_app_data_dir, session_id)?
                    .is_some_and(|session| {
                        session
                            .messages
                            .iter()
                            .any(|stored| stored.id == message.id)
                    });
                if exists {
                    self.persist_message_parts(
                        _app_data_dir,
                        session_id,
                        &message.id,
                        message.parts.as_deref().unwrap_or_default(),
                        message.streaming_final_seq,
                        Some(message.timestamp),
                    )?;
                } else {
                    self.append_message(_app_data_dir, session_id, &message)?;
                }
                self.clear_pending_recovery_message(_app_data_dir, session_id, pending)?;
                return Ok(!exists);
            }
            #[cfg(not(test))]
            unreachable!("production mutation admission rejects a missing SQLite authority");
        }

        let mut projection = self
            .read_session_projection(session_id)?
            .ok_or_else(|| format!("Session projection not found: {session_id}"))?;
        let existing = self.canonical_message_projection(session_id, &message.id)?;
        let projection_codec = self
            .event_authority
            .read()
            .as_ref()
            .expect("canonical mutation admission requires SQLite authority")
            .projection_codec
            .clone();
        let expected_message = projection_codec.recovery_publication_message_record(pending);
        let projected_pending_message = projection
            .meta
            .pending_recovery_message
            .as_ref()
            .map(|pending| projection_codec.recovery_publication_message_record(pending));
        let recovery_id = expected_message.recovery_id.as_str();
        let message_id = expected_message.message_id.as_str();
        let source_obligation_id = backend_recovery_obligation_id(session_id, recovery_id);
        let publication_obligation_id =
            recovery_publication_obligation_id(session_id, recovery_id, message_id);
        let current_publication = self.canonical_obligation(&publication_obligation_id)?;
        match decide_recovery_publication_commit(RecoveryPublicationCommitFacts {
            session_id,
            recovery_id,
            message_id,
            candidate_message_id: &message.id,
            source_obligation_id: &source_obligation_id,
            expected_message: &expected_message,
            current_obligation: current_publication.as_ref(),
            projected_pending_message: projected_pending_message.as_ref(),
            message_already_exists: existing.is_some(),
        }) {
            Ok(RecoveryPublicationCommitDecision::Publish) => {}
            Ok(RecoveryPublicationCommitDecision::AlreadyPublished) => return Ok(false),
            Err(RecoveryPublicationCommitRejection::MessageIdentityMismatch) => {
                return Err("recovery publication message identity changed".to_string());
            }
            Err(RecoveryPublicationCommitRejection::ObligationIdentityMismatch) => {
                return Err(
                    "recovery publication obligation identity is inconsistent".to_string(),
                );
            }
            Err(RecoveryPublicationCommitRejection::PendingIdentityMismatch) => {
                return Err("pending backend recovery publication identity changed".to_string());
            }
            Err(RecoveryPublicationCommitRejection::NoLongerPending) => {
                return Err("backend recovery publication is no longer pending".to_string());
            }
        }
        let inserted = existing.is_none();
        if inserted {
            projection.meta.message_count =
                add_sqlite_count(projection.meta.message_count, 1, "session message count")?;
            if projection.meta.first_message_preview.is_empty() {
                projection.meta.first_message_preview =
                    super::first_message_preview(std::slice::from_ref(&message));
            }
        }
        projection.meta.pending_recovery_message = None;
        projection.meta.updated_at = message.timestamp;
        projection.messages = vec![message];
        let completed_publication = recovery_publication_obligation_mutation(
            publication_obligation_id,
            crate::domain::local_event::ObligationRecord::RecoveryPublication {
                session_id: session_id.to_string(),
                recovery_id: recovery_id.to_string(),
                message_id: message_id.to_string(),
                source_obligation_id,
                detail:
                    crate::domain::local_event::RecoveryPublicationObligationRecord::Completed {
                        published_at_bits: projection.meta.updated_at.to_bits(),
                    },
                state: crate::domain::local_event::ObligationStateRecord::Completed,
            },
            None,
            current_publication.as_ref(),
        )?;
        self.commit_session_projection_snapshot_with_kind_and_mutations(
            projection,
            crate::domain::local_event::CommitOperationKind::Projection,
            vec![completed_publication],
        )?;
        Ok(inserted)
    }

    pub fn record_backend_session_established(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        expected_provider_session_generation: u64,
        observation_id: &str,
        backend_session_id: String,
        context_carry: Option<ContextCarryState>,
    ) -> Result<ProviderSessionEstablishmentOutcome, String> {
        #[cfg(test)]
        if let Some(hook) = self.backend_established_hook.read().clone() {
            hook(session_id, &backend_session_id)?;
        }
        let projection_codec = self
            .event_authority
            .read()
            .as_ref()
            .map(|authority| authority.projection_codec.clone());
        let mut fenced = false;
        let updated = self.update_meta_if_changed(app_data_dir, session_id, |meta| {
            let actual_generation = meta.provider_session_generation;
            let mut state = match projection_codec.as_ref() {
                Some(codec) => codec.backend_recovery_from_meta(meta, false),
                #[cfg(test)]
                None => test_domain_backend_recovery_meta(meta, false),
                #[cfg(not(test))]
                None => unreachable!("production mutation admission requires a projection codec"),
            };
            match state
                .observe_provider_session_established(
                    observation_id,
                    &backend_session_id,
                    expected_provider_session_generation,
                    context_carry,
                )
                .map_err(|rejection| match rejection {
                    BackendRecoveryProjectionRejection::InvalidObservationIdentity => {
                        "provider session observation identity is empty".to_string()
                    }
                    BackendRecoveryProjectionRejection::ConflictingProviderIdentity => {
                        "provider session observation identity has conflicting backend identity"
                            .to_string()
                    }
                    BackendRecoveryProjectionRejection::StaleProviderGeneration => format!(
                        "Provider session generation changed while recording establishment: expected {expected_provider_session_generation}, actual {actual_generation}"
                    ),
                    BackendRecoveryProjectionRejection::ProviderGenerationExhausted => {
                        "provider session generation is exhausted".to_string()
                    }
                    BackendRecoveryProjectionRejection::QueuePaused
                    | BackendRecoveryProjectionRejection::DurableEvidenceMismatch => {
                        "provider session establishment decision is inconsistent".to_string()
                    }
                })? {
                DomainProviderSessionEstablishment::Applied => {
                    match projection_codec.as_ref() {
                        Some(codec) => codec.apply_backend_recovery_to_meta(meta, state),
                        #[cfg(test)]
                        None => test_apply_domain_backend_recovery_meta(meta, state),
                        #[cfg(not(test))]
                        None => {
                            unreachable!("production mutation admission requires a projection codec")
                        }
                    }
                }
                DomainProviderSessionEstablishment::AlreadyApplied => return Ok(false),
                DomainProviderSessionEstablishment::Fenced => {
                    fenced = true;
                    return Ok(false);
                }
            }
            meta.updated_at = now_timestamp();
            Ok(true)
        })?;
        if let Some(meta) = updated {
            return Ok(ProviderSessionEstablishmentOutcome::Settled(Box::new(meta)));
        }
        if fenced {
            return Ok(ProviderSessionEstablishmentOutcome::Fenced);
        }
        let Some(meta) = self.get_session_meta(app_data_dir, session_id)? else {
            return Ok(ProviderSessionEstablishmentOutcome::Missing);
        };
        let state = match projection_codec.as_ref() {
            Some(codec) => codec.backend_recovery_from_meta(&meta, false),
            #[cfg(test)]
            None => test_domain_backend_recovery_meta(&meta, false),
            #[cfg(not(test))]
            None => unreachable!("production mutation admission requires a projection codec"),
        };
        if !state.owns_provider_establishment(observation_id, &backend_session_id) {
            return Err(
                "provider session observation replay no longer owns the durable generation"
                    .to_string(),
            );
        }
        Ok(ProviderSessionEstablishmentOutcome::Settled(Box::new(meta)))
    }

}
