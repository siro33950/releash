impl SessionStore {
    fn read_session_metadata_inventory(
        &self,
        app_data_dir: &Path,
    ) -> Result<Vec<SessionMeta>, String> {
        #[cfg(test)]
        if !self.canonical_authority_active() {
            return self.test_storage().list_metas(app_data_dir);
        }
        self.read_canonical_metadata_inventory(app_data_dir)
    }

    fn ensure_canonical_mutation_admission(&self) -> Result<(), String> {
        match self.event_authority.read().as_ref() {
            Some(_) => Ok(()),
            None => {
                #[cfg(test)]
                return Ok(());
                #[cfg(not(test))]
                return Err("agent-session SQLite event authority is not configured".to_string());
            }
        }
    }

    fn canonical_authority_active(&self) -> bool {
        self.event_authority.read().is_some()
    }

    #[cfg(test)]
    pub fn new(storage: Arc<dyn SessionStoragePort>) -> Self {
        Self {
            storage: Some(storage),
            event_authority: RwLock::new(None),
            state_change_listeners: RwLock::new(Vec::new()),
            event_log_recovery_listeners: RwLock::new(Vec::new()),
            runtime_terminal_participant_provider: RwLock::new(None),
            #[cfg(test)]
            permission_response_reservations: RwLock::new(HashMap::new()),
            #[cfg(test)]
            save_hook: RwLock::new(None),
            #[cfg(test)]
            append_message_hook: RwLock::new(None),
            #[cfg(test)]
            persist_parts_hook: RwLock::new(None),
            #[cfg(test)]
            append_event_hook: RwLock::new(None),
            #[cfg(test)]
            set_state_hook: RwLock::new(None),
            #[cfg(test)]
            projection_hook: RwLock::new(None),
            #[cfg(test)]
            appended_event_hook: RwLock::new(None),
            #[cfg(test)]
            event_projection_hook: RwLock::new(None),
            #[cfg(test)]
            atomic_event_commit_hook: RwLock::new(None),
            #[cfg(test)]
            backend_established_hook: RwLock::new(None),
            #[cfg(test)]
            projected_read_model_hook: RwLock::new(None),
        }
    }

    pub(crate) fn new_canonical(
        repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository>,
        installation_id: String,
        projection_codec: Arc<dyn AgentSessionProjectionCodec>,
    ) -> Self {
        Self {
            #[cfg(test)]
            storage: None,
            event_authority: RwLock::new(Some(AgentSessionEventAuthority {
                repository,
                installation_id,
                projection_codec,
            })),
            state_change_listeners: RwLock::new(Vec::new()),
            event_log_recovery_listeners: RwLock::new(Vec::new()),
            runtime_terminal_participant_provider: RwLock::new(None),
            #[cfg(test)]
            permission_response_reservations: RwLock::new(HashMap::new()),
            #[cfg(test)]
            save_hook: RwLock::new(None),
            #[cfg(test)]
            append_message_hook: RwLock::new(None),
            #[cfg(test)]
            persist_parts_hook: RwLock::new(None),
            #[cfg(test)]
            append_event_hook: RwLock::new(None),
            #[cfg(test)]
            set_state_hook: RwLock::new(None),
            #[cfg(test)]
            projection_hook: RwLock::new(None),
            #[cfg(test)]
            appended_event_hook: RwLock::new(None),
            #[cfg(test)]
            event_projection_hook: RwLock::new(None),
            #[cfg(test)]
            atomic_event_commit_hook: RwLock::new(None),
            #[cfg(test)]
            backend_established_hook: RwLock::new(None),
            #[cfg(test)]
            projected_read_model_hook: RwLock::new(None),
        }
    }

    #[cfg(test)]
    fn test_storage(&self) -> &dyn SessionStoragePort {
        self.storage
            .as_deref()
            .expect("test file-session storage is not configured")
    }

    #[cfg(test)]
    pub(crate) fn set_local_event_repository(
        &self,
        repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository>,
        installation_id: String,
        projection_codec: Arc<dyn AgentSessionProjectionCodec>,
    ) {
        *self.event_authority.write() = Some(AgentSessionEventAuthority {
            repository,
            installation_id,
            projection_codec,
        });
    }

    #[cfg(test)]
    pub(crate) fn local_event_repository(
        &self,
    ) -> Option<Arc<dyn crate::domain::local_event::LocalEventTransactionRepository>> {
        self.event_authority
            .read()
            .as_ref()
            .map(|authority| authority.repository.clone())
    }

    pub(crate) fn set_runtime_terminal_participant_provider(
        &self,
        provider: Arc<dyn RuntimeTerminalParticipantProvider>,
    ) {
        *self.runtime_terminal_participant_provider.write() = Some(provider);
    }

    /// Fail-closed admission fence for mutations that could start or drain a
    /// provider/workflow effect. The owner secondary index is the authority;
    /// this does not hydrate a session or infer recovery from live state.
    pub(crate) async fn ensure_no_unresolved_recovery(
        &self,
        owner: &str,
    ) -> Result<(), crate::domain::local_event::SafeOperationFailure> {
        use crate::domain::local_event::{
            LocalEventQuery, LocalEventQueryResult, SessionOperationFailureKind,
        };

        if owner.is_empty() {
            return Err(crate::domain::local_event::SafeOperationFailure::new(
                SessionOperationFailureKind::Internal,
                false,
                "The recovery owner identity is invalid.",
                "recovery-owner-invalid",
            ));
        }
        let authority = match self.event_authority.read().clone() {
            Some(authority) => authority,
            None => {
                // Legacy unit fixtures have no #1499 authority. Never extend
                // this bypass to a production build: missing canonical
                // recovery authority must close mutation admission.
                #[cfg(test)]
                return Ok(());
                #[cfg(not(test))]
                return Err(crate::domain::local_event::SafeOperationFailure::new(
                    SessionOperationFailureKind::StorageUnavailable,
                    true,
                    "The pending recovery authority is unavailable.",
                    format!("recovery-authority-{owner}"),
                ));
            }
        };
        let owner = owner.to_string();
        const PAGE_LIMIT: usize = 200;
        let mut cursor = None;
        loop {
            let result = authority
                .repository
                .query(LocalEventQuery::PendingRecoveryPage {
                    limit: PAGE_LIMIT,
                    partition: None,
                    owner: Some(owner.clone()),
                    ordered_key_prefix: None,
                    shutdown_plan: None,
                    cursor,
                })
                .await
                .map_err(|_| {
                    crate::domain::local_event::SafeOperationFailure::new(
                        SessionOperationFailureKind::StorageUnavailable,
                        true,
                        "The pending recovery inventory is unavailable.",
                        format!("recovery-inventory-{owner}"),
                    )
                })?;
            let LocalEventQueryResult::PendingRecoveryPage(page) = result else {
                return Err(crate::domain::local_event::SafeOperationFailure::new(
                    SessionOperationFailureKind::StorageCorrupt,
                    false,
                    "The pending recovery inventory is incompatible.",
                    format!("recovery-inventory-{owner}"),
                ));
            };
            for entry in page.entries {
                if entry.owner != owner {
                    return Err(crate::domain::local_event::SafeOperationFailure::new(
                        SessionOperationFailureKind::StorageCorrupt,
                        false,
                        "The pending recovery owner index is inconsistent.",
                        entry.obligation_id,
                    ));
                }
                if let Some(identity) = entry
                    .record
                    .unresolved_recovery_original_identity(&entry.obligation_id)
                {
                    return Err(crate::domain::local_event::SafeOperationFailure::new(
                        SessionOperationFailureKind::OutcomeUnknown,
                        true,
                        "Unresolved recovery must be resolved before this operation.",
                        identity.clone(),
                    )
                    .with_detail(&format!("Pending recovery identity: {identity}")));
                }
            }
            cursor = page.next_cursor;
            if cursor.is_none() {
                return Ok(());
            }
        }
    }

    fn read_session_projection(
        &self,
        session_id: &str,
    ) -> Result<Option<CanonicalAgentSessionProjection>, String> {
        self.read_session_projection_with_revision(session_id)
            .map(|projection| projection.map(|(projection, _)| projection))
    }

    fn read_session_projection_with_revision(
        &self,
        session_id: &str,
    ) -> Result<
        Option<(
            CanonicalAgentSessionProjection,
            crate::domain::local_event::Revision,
        )>,
        String,
    > {
        let Some(authority) = self.event_authority.read().clone() else {
            return Ok(None);
        };
        let result = authority
            .repository
            .query_blocking(
                crate::domain::local_event::LocalEventQuery::SessionProjectionByIdentity {
                    session_id: session_id.to_string(),
                },
            )
            .map_err(|error| format!("agent SQLite projection read failed: {error}"))?;
        let crate::domain::local_event::LocalEventQueryResult::SessionProjectionByIdentity(
            projection,
        ) = result
        else {
            return Err("agent SQLite projection query returned the wrong shape".to_string());
        };
        projection
            .map(|projection| {
                authority
                    .projection_codec
                    .decode(&projection.projection)
                    .map(|decoded| (decoded, projection.revision))
            })
            .transpose()
    }

    /// Read the bounded durable queue identity projection in its canonical
    /// execution order. Retry payloads remain obligation-owned and are not
    /// retained here.
    pub(crate) fn canonical_pending_send_queue(
        &self,
        session_id: &str,
    ) -> Result<Vec<CanonicalQueuedSend>, String> {
        self.read_session_projection(session_id)?
            .map(|projection| projection.pending_send_queue)
            .ok_or_else(|| format!("Session projection not found: {session_id}"))
    }

    /// Check that a queued effect still names one exact durable queue entry.
    /// `input_ref` is optional because older accepted-effect DTOs do not carry
    /// it; callers that have the receipt should supply it for the full match.
    pub(crate) fn canonical_queue_contains_exact(
        &self,
        session_id: &str,
        queue_item_id: &str,
        human_message_id: &str,
        reserved_turn_id: &str,
        input_ref: Option<&str>,
    ) -> Result<bool, String> {
        Ok(self
            .canonical_pending_send_queue(session_id)?
            .iter()
            .any(|entry| {
                entry.queue_item_id == queue_item_id
                    && entry.human_message_id == human_message_id
                    && entry.reserved_turn_id == reserved_turn_id
                    && input_ref.is_none_or(|input_ref| entry.input_ref == input_ref)
            }))
    }

    fn canonical_obligation(
        &self,
        obligation_id: &str,
    ) -> Result<Option<crate::domain::local_event::ObligationView>, String> {
        let authority = self
            .event_authority
            .read()
            .clone()
            .ok_or_else(|| "agent-session SQLite authority is unavailable".to_string())?;
        let obligation_id = obligation_id.to_string();
        std::thread::scope(|scope| {
            scope
                .spawn(move || {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|error| {
                            format!("failed to create obligation read runtime: {error}")
                        })?
                        .block_on(async move {
                            match authority
                                .repository
                                .query(
                                    crate::domain::local_event::LocalEventQuery::ObligationByIdentity {
                                        obligation_id,
                                    },
                                )
                                .await
                                .map_err(|error| {
                                    format!("agent SQLite obligation read failed: {error}")
                                })?
                            {
                                crate::domain::local_event::LocalEventQueryResult::ObligationByIdentity(obligation) => Ok(obligation),
                                _ => Err("agent SQLite obligation query returned the wrong shape".to_string()),
                            }
                        })
                })
                .join()
                .map_err(|_| "obligation read worker panicked".to_string())?
        })
    }

    fn canonical_terminal(
        &self,
        session_id: &str,
        turn_id: u64,
    ) -> Result<Option<crate::domain::local_event::TerminalRecordView>, String> {
        let authority = self
            .event_authority
            .read()
            .clone()
            .ok_or_else(|| "agent-session SQLite authority is unavailable".to_string())?;
        let session_id = session_id.to_string();
        let turn_id = turn_id.to_string();
        std::thread::scope(|scope| {
            scope
                .spawn(move || {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|error| {
                            format!("failed to create terminal read runtime: {error}")
                        })?
                        .block_on(async move {
                            match authority
                                .repository
                                .query(crate::domain::local_event::LocalEventQuery::TerminalByTurn {
                                    session_id,
                                    turn_id,
                                })
                                .await
                                .map_err(|error| {
                                    format!("agent SQLite terminal read failed: {error}")
                                })?
                            {
                                crate::domain::local_event::LocalEventQueryResult::TerminalByTurn(terminal) => Ok(terminal),
                                _ => Err("agent SQLite terminal query returned the wrong shape".to_string()),
                            }
                        })
                })
                .join()
                .map_err(|_| "terminal read worker panicked".to_string())?
        })
    }

    pub(crate) fn canonical_message_projection(
        &self,
        session_id: &str,
        message_id: &str,
    ) -> Result<Option<ChatMessage>, String> {
        let Some(authority) = self.event_authority.read().clone() else {
            return Ok(None);
        };
        let codec = authority.projection_codec.clone();
        let session_id = session_id.to_string();
        let message_id = message_id.to_string();
        std::thread::scope(|scope| {
            scope
                .spawn(move || {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|error| {
                            format!("failed to create agent message read runtime: {error}")
                        })?
                        .block_on(async move {
                            let result = authority
                                .repository
                                .query(crate::domain::local_event::LocalEventQuery::MessageProjectionByIdentity {
                                    session_id: session_id.clone(),
                                    message_id: message_id.clone(),
                                })
                                .await
                                .map_err(|error| {
                                    format!("agent SQLite message projection read failed: {error}")
                                })?;
                            let crate::domain::local_event::LocalEventQueryResult::MessageProjectionByIdentity(
                                projection,
                            ) = result
                            else {
                                return Err("agent SQLite message projection query returned the wrong shape".to_string());
                            };
                            projection
                                .map(|projection| codec.decode_message(&projection.projection))
                                .transpose()
                        })
                })
                .join()
                .map_err(|_| "agent SQLite message read worker panicked".to_string())?
        })
    }

    /// Reads only the workflow turn-completion namespace from the pending
    /// index. Callers retain the returned cursor and decide how many bounded
    /// pages to replay; this method never falls back to a full inventory scan.
    pub(crate) fn pending_workflow_turn_completion_page(
        &self,
        owner: Option<&str>,
        turn_id: Option<u64>,
        limit: usize,
        cursor: Option<crate::domain::local_event::QueryCursor>,
    ) -> Result<PendingWorkflowTurnCompletionPage, String> {
        const MAX_PAGE: usize = 128;
        if limit == 0 || limit > MAX_PAGE || owner.is_some_and(str::is_empty) {
            return Err("workflow turn-completion page request is invalid".to_string());
        }
        if turn_id.is_some() && owner.is_none() {
            return Err(
                "workflow turn-completion turn lookup requires a session owner".to_string(),
            );
        }
        let authority = self
            .event_authority
            .read()
            .clone()
            .ok_or_else(|| "agent-session SQLite authority is unavailable".to_string())?;
        let codec = authority.projection_codec.clone();
        let owner = owner.map(str::to_string);
        let ordered_key_prefix = workflow_turn_completion_ordered_key_prefix(turn_id);
        std::thread::scope(|scope| {
            scope
                .spawn(move || {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|error| {
                            format!(
                                "failed to create workflow turn-completion read runtime: {error}"
                            )
                        })?
                        .block_on(async move {
                            let result = authority
                                .repository
                                .query(
                                    crate::domain::local_event::LocalEventQuery::PendingRecoveryPage {
                                        limit,
                                        // Prefix queries are already an exact
                                        // bounded namespace and the closed
                                        // query contract treats partition and
                                        // owner as mutually exclusive. Each
                                        // decoded row is still required to be
                                        // in Owner below.
                                        partition: None,
                                        owner,
                                        ordered_key_prefix: Some(ordered_key_prefix),
                                        shutdown_plan: None,
                                        cursor,
                                    },
                                )
                                .await
                                .map_err(|error| {
                                    format!(
                                        "workflow turn-completion pending read failed: {error}"
                                    )
                                })?;
                            let crate::domain::local_event::LocalEventQueryResult::PendingRecoveryPage(
                                page,
                            ) = result
                            else {
                                return Err(
                                    "workflow turn-completion pending query returned the wrong shape"
                                        .to_string(),
                                );
                            };
                            let mut entries = Vec::with_capacity(page.entries.len());
                            for stored in page.entries {
                                let pending =
                                    crate::domain::local_event::validate_pending_workflow_turn_completion(
                                        &stored.obligation_id,
                                        &stored.owner,
                                        &stored.ordered_key,
                                        stored.partition,
                                        stored.shutdown_plan.is_some(),
                                        &stored.record,
                                    )
                                    .map_err(|rejection| match rejection {
                                        crate::domain::local_event::PendingWorkflowTurnCompletionRejection::IncompatibleRecord => {
                                            "completed workflow turn-completion obligation remained pending"
                                                .to_string()
                                        }
                                        crate::domain::local_event::PendingWorkflowTurnCompletionRejection::InvalidTurnIdentity => {
                                            "workflow turn-completion turn identity is invalid"
                                                .to_string()
                                        }
                                        _ => {
                                            "workflow turn-completion obligation identity is inconsistent"
                                                .to_string()
                                        }
                                    })?;
                                let terminal = authority
                                    .repository
                                    .query(
                                        crate::domain::local_event::LocalEventQuery::TerminalByTurn {
                                            session_id: pending.session_id.clone(),
                                            turn_id: pending.turn_id.to_string(),
                                        },
                                    )
                                    .await
                                    .map_err(|error| {
                                        format!(
                                            "workflow turn-completion terminal read failed: {error}"
                                        )
                                    })?;
                                let crate::domain::local_event::LocalEventQueryResult::TerminalByTurn(
                                    Some(terminal),
                                ) = terminal
                                else {
                                    return Err(
                                        "workflow turn-completion terminal record is missing"
                                            .to_string(),
                                    );
                                };
                                crate::domain::local_event::validate_workflow_turn_completion_terminal(
                                    &terminal,
                                    &pending,
                                )
                                .map_err(|rejection| match rejection {
                                    crate::domain::local_event::PendingWorkflowTurnCompletionRejection::MessageIdentityMismatch => {
                                        "workflow turn-completion message reference is inconsistent"
                                            .to_string()
                                    }
                                    _ => {
                                        "workflow turn-completion terminal record is inconsistent"
                                            .to_string()
                                    }
                                })?;
                                let message = authority
                                    .repository
                                    .query(
                                        crate::domain::local_event::LocalEventQuery::MessageProjectionByIdentity {
                                            session_id: pending.session_id.clone(),
                                            message_id: pending.message_id.clone(),
                                        },
                                    )
                                    .await
                                    .map_err(|error| {
                                        format!(
                                            "workflow turn-completion message read failed: {error}"
                                        )
                                    })?;
                                let crate::domain::local_event::LocalEventQueryResult::MessageProjectionByIdentity(
                                    Some(message),
                                ) = message
                                else {
                                    return Err(
                                        "workflow turn-completion message projection is missing"
                                            .to_string(),
                                    );
                                };
                                let message = codec.decode_message(&message.projection)?;
                                let final_text_parts = codec
                                    .workflow_final_text_parts(&message, &pending.message_id)?;
                                let identity =
                                    crate::domain::local_event::validate_workflow_turn_completion_notification(
                                        &pending,
                                        &final_text_parts,
                                    )
                                    .map_err(|_| {
                                        "workflow turn-completion notification binding is inconsistent"
                                            .to_string()
                                    })?;
                                let input =
                                    codec.workflow_turn_complete_input(&pending, final_text_parts);
                                entries.push(PendingWorkflowTurnCompletion {
                                    obligation_id: stored.obligation_id,
                                    revision: stored.revision,
                                    session_id: pending.session_id,
                                    workflow_context: pending.workflow_context,
                                    input,
                                    terminal_identity: pending.terminal_identity,
                                    message_id: pending.message_id,
                                    notification_sha256: identity.notification_sha256,
                                });
                            }
                            Ok(PendingWorkflowTurnCompletionPage {
                                entries,
                                next_cursor: page.next_cursor,
                            })
                        })
                })
                .join()
                .map_err(|_| {
                    "workflow turn-completion read worker panicked".to_string()
                })?
        })
    }

    #[cfg(test)]
    pub(crate) fn pending_workflow_turn_completion(
        &self,
        session_id: &str,
        turn_id: u64,
    ) -> Result<Option<PendingWorkflowTurnCompletion>, String> {
        let page =
            self.pending_workflow_turn_completion_page(Some(session_id), Some(turn_id), 2, None)?;
        if page.next_cursor.is_some() || page.entries.len() > 1 {
            return Err(
                "multiple pending workflow turn-completions exist for one session turn".to_string(),
            );
        }
        Ok(page.entries.into_iter().next())
    }

    /// Removes the pending membership only after the workflow side has
    /// durably accepted this exact notification. Replays of the same consume
    /// are successful; a different binding fails closed.
    #[cfg(test)]
    pub(crate) fn complete_workflow_turn_completion(
        &self,
        entry: &PendingWorkflowTurnCompletion,
    ) -> Result<(), String> {
        self.settle_workflow_turn_completion(
            entry,
            crate::domain::local_event::WorkflowObligationTerminalOutcome::Applied,
        )
    }

    pub(crate) fn settle_workflow_turn_completion(
        &self,
        entry: &PendingWorkflowTurnCompletion,
        outcome: crate::domain::local_event::WorkflowObligationTerminalOutcome,
    ) -> Result<(), String> {
        let projection_codec = self
            .event_authority
            .read()
            .as_ref()
            .ok_or_else(|| "agent-session SQLite authority is unavailable".to_string())?
            .projection_codec
            .clone();
        let current = self
            .canonical_obligation(&entry.obligation_id)?
            .ok_or_else(|| {
                "workflow turn-completion obligation disappeared before consume".to_string()
            })?;
        let settled_at_bits = now_timestamp().to_bits();
        let decision = crate::domain::local_event::decide_workflow_turn_completion_settlement(
            crate::domain::local_event::WorkflowTurnCompletionSettlementFacts {
                obligation_id: &entry.obligation_id,
                revision: entry.revision,
                session_id: &entry.session_id,
                workflow_context: &entry.workflow_context,
                terminal_identity: &entry.terminal_identity,
                message_id: &entry.message_id,
                turn_id: entry.input.turn_id,
                exit_code: entry.input.exit_code,
                final_text_parts: &entry.input.final_text_parts,
                failure_signal: projection_codec
                    .workflow_failure_signal(entry.input.failure_signal),
                token_usage: entry.input.token_usage,
                interrupted: entry.input.interrupted,
                notification_sha256: &entry.notification_sha256,
            },
            &current,
            outcome,
            settled_at_bits,
        )
        .map_err(|rejection| match rejection {
            crate::domain::local_event::WorkflowTurnCompletionSettlementRejection::ConsumeBindingMismatch => {
                "workflow turn-completion consume binding is inconsistent".to_string()
            }
            crate::domain::local_event::WorkflowTurnCompletionSettlementRejection::CompletedObligationMismatch => {
                "completed workflow turn-completion obligation is inconsistent".to_string()
            }
            crate::domain::local_event::WorkflowTurnCompletionSettlementRejection::PendingObligationMismatch => {
                "pending workflow turn-completion obligation is inconsistent".to_string()
            }
            crate::domain::local_event::WorkflowTurnCompletionSettlementRejection::IncompatibleObligation => {
                "workflow turn-completion obligation has an incompatible kind or state".to_string()
            }
            crate::domain::local_event::WorkflowTurnCompletionSettlementRejection::AlreadyTerminal => {
                "workflow turn-completion obligation was already terminal before settle".to_string()
            }
        })?;
        let (notification_digest, detail) = match decision {
            crate::domain::local_event::WorkflowTurnCompletionSettlementDecision::AlreadySettled => {
                return Ok(());
            }
            crate::domain::local_event::WorkflowTurnCompletionSettlementDecision::Apply {
                notification_digest,
                detail,
            } => (notification_digest, detail),
        };
        let record = crate::domain::local_event::ObligationRecord::WorkflowTurnCompletion {
            session_id: entry.session_id.clone(),
            turn_id: entry.input.turn_id.to_string(),
            terminal_identity: entry.terminal_identity.clone(),
            notification_sha256: notification_digest,
            detail,
            state: crate::domain::local_event::ObligationStateRecord::Completed,
        };
        let mutation = crate::domain::local_event::LocalStateMutation::Obligation(
            crate::domain::local_event::ObligationMutation {
                obligation_id: entry.obligation_id.clone(),
                record,
                pending: None,
                expected: crate::domain::local_event::RevisionGuard::Expected(current.revision),
                revision: current.revision.next().ok_or_else(|| {
                    "workflow turn-completion obligation revision exhausted".to_string()
                })?,
            },
        );
        let mutation_identity = mutation.canonical_identity_v1().map_err(str::to_string)?;
        let commit_identity =
            crate::domain::local_event::workflow_turn_completion_consume_commit_identity(
                &entry.obligation_id,
                current.revision.value(),
                &mutation_identity,
            );
        let payload_hash = commit_identity.digest;
        let commit_identity = commit_identity.identity;
        let authority = self
            .event_authority
            .read()
            .clone()
            .ok_or_else(|| "agent-session SQLite authority is unavailable".to_string())?;
        let idempotency_key = format!(
            "workflow-turn-complete.consume:{}",
            entry.notification_sha256
        );
        let obligation_id = entry.obligation_id.clone();
        let commit_result = std::thread::scope(|scope| {
            scope
                .spawn(move || {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|error| {
                            format!(
                                "failed to create workflow turn-completion consume runtime: {error}"
                            )
                        })?
                        .block_on(async move {
                            let commit_id =
                                crate::domain::local_event::CommitIdentity::parse(&commit_identity)
                                    .map_err(|_| {
                                        "workflow turn-completion consume identity is invalid"
                                            .to_string()
                                    })?;
                            let batch = crate::domain::local_event::LocalAtomicBatch {
                                commit_id: commit_id.clone(),
                                idempotency: crate::domain::local_event::IdempotencyBinding {
                                    installation_id: authority.installation_id.clone(),
                                    operation_kind:
                                        crate::domain::local_event::CommitOperationKind::Recovery,
                                    idempotency_key,
                                    payload_hash,
                                },
                                expected_heads: Vec::new(),
                                events: Vec::new(),
                                state_mutations: vec![mutation],
                            };
                            match authority.repository.commit_batch(batch).await {
                                Ok(_) => Ok(()),
                                Err(
                                    crate::domain::local_event::CommitBatchError::OutcomeUnknown {
                                        identity,
                                    },
                                ) => match authority.repository.resolve_commit(identity).await {
                                    Ok(crate::domain::local_event::CommitResolution::Committed(_)) => {
                                        Ok(())
                                    }
                                    Ok(crate::domain::local_event::CommitResolution::NotCommitted) => {
                                        Err("workflow turn-completion consume was not committed"
                                            .to_string())
                                    }
                                    Err(error) => Err(format!(
                                        "workflow turn-completion consume outcome could not be resolved: {error}"
                                    )),
                                },
                                Err(error) => Err(format!(
                                    "workflow turn-completion consume failed: {error}"
                                )),
                            }
                        })
                })
                .join()
                .map_err(|_| {
                    "workflow turn-completion consume worker panicked".to_string()
                })?
        });
        if let Err(error) = commit_result {
            if let Some(current) = self.canonical_obligation(&obligation_id)? {
                if let crate::domain::local_event::ObligationRecord::WorkflowTurnCompletion {
                    session_id,
                    turn_id,
                    terminal_identity,
                    notification_sha256,
                    detail,
                    state: crate::domain::local_event::ObligationStateRecord::Completed,
                } = current.record
                {
                    if session_id == entry.session_id
                        && turn_id.parse::<u64>().ok() == Some(entry.input.turn_id)
                        && terminal_identity == entry.terminal_identity
                        && notification_sha256 == notification_digest
                        && current.pending.is_none()
                        && detail.terminal_outcome().is_some()
                    {
                        return Ok(());
                    }
                }
            }
            return Err(error);
        }
        Ok(())
    }

    fn canonical_content_blob(
        &self,
        session_id: &str,
        identity: String,
    ) -> Result<Option<crate::domain::local_event::AgentContentBlobRecord>, String> {
        let authority = self
            .event_authority
            .read()
            .clone()
            .ok_or_else(|| "agent-session SQLite authority is unavailable".to_string())?;
        let blob_session_id = format!("blob:{session_id}");
        std::thread::scope(|scope| {
            scope
                .spawn(move || {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|error| {
                            format!("failed to create content blob read runtime: {error}")
                        })?
                        .block_on(async move {
                            let result = authority
                                .repository
                                .query(crate::domain::local_event::LocalEventQuery::MessageProjectionByIdentity {
                                    session_id: blob_session_id,
                                    message_id: identity,
                                })
                                .await
                                .map_err(|error| format!("SQLite content blob read failed: {error}"))?;
                            let crate::domain::local_event::LocalEventQueryResult::MessageProjectionByIdentity(record) = result else {
                                return Err("SQLite content blob query returned the wrong shape".to_string());
                            };
                            record
                                .map(|record| match record.projection {
                                    crate::domain::local_event::MessageProjectionRecord::AgentContentBlob(blob) => Ok(blob),
                                    _ => Err("SQLite content blob is incompatible".to_string()),
                                })
                                .transpose()
                        })
                })
                .join()
                .map_err(|_| "SQLite content blob read worker panicked".to_string())?
        })
    }

    fn read_canonical_metadata_inventory(
        &self,
        _app_data_dir: &Path,
    ) -> Result<Vec<SessionMeta>, String> {
        let authority = self.event_authority.read().clone().ok_or_else(|| {
            "agent-session SQLite projection authority is unavailable".to_string()
        })?;
        let mut after_session_id = None;
        let mut metas = Vec::new();
        loop {
            let result = authority
                .repository
                .query_blocking(
                    crate::domain::local_event::LocalEventQuery::SessionProjectionPage {
                        limit: 200,
                        after_session_id: after_session_id.clone(),
                    },
                )
                .map_err(|error| format!("agent SQLite projection page read failed: {error}"))?;
            let crate::domain::local_event::LocalEventQueryResult::SessionProjectionPage(page) =
                result
            else {
                return Err(
                    "agent SQLite projection page query returned the wrong shape".to_string(),
                );
            };
            let page_len = page.len();
            for projection in page {
                after_session_id = Some(projection.session_id);
                if !matches!(
                    &projection.projection,
                    crate::domain::local_event::SessionProjectionRecord::AgentSession(_)
                ) {
                    continue;
                }
                metas.push(
                    authority
                        .projection_codec
                        .decode(&projection.projection)?
                        .meta,
                );
            }
            if page_len < 200 {
                break;
            }
        }
        Ok(metas)
    }

    fn canonical_message_page(
        &self,
        session_id: &str,
        cursor: Option<PageCursor>,
        limit: usize,
    ) -> Result<SessionPage, String> {
        let authority = self.event_authority.read().clone().ok_or_else(|| {
            "agent-session SQLite projection authority is unavailable".to_string()
        })?;
        let codec = authority.projection_codec.clone();
        let session_id = session_id.to_string();
        let before_position = cursor
            .map(|cursor| i64::try_from(cursor.0))
            .transpose()
            .map_err(|_| "agent message page cursor exceeds i64::MAX".to_string())?;
        std::thread::scope(|scope| {
            scope
                .spawn(move || {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|error| {
                            format!("failed to create agent message page runtime: {error}")
                        })?
                        .block_on(async move {
                            let result = authority
                                .repository
                                .query(crate::domain::local_event::LocalEventQuery::MessageProjectionPage {
                                    session_id: session_id.clone(),
                                    before_position,
                                    limit,
                                })
                                .await
                                .map_err(|error| {
                                    format!("agent SQLite message page read failed: {error}")
                                })?;
                            let crate::domain::local_event::LocalEventQueryResult::MessageProjectionPage(
                                page,
                            ) = result
                            else {
                                return Err("agent SQLite message page query returned the wrong shape".to_string());
                            };
                            let mut messages = Vec::with_capacity(page.entries.len());
                            let mut message_metadata = Vec::with_capacity(page.entries.len());
                            for entry in page.entries {
                                let message = codec.decode_message(&entry.message.projection)?;
                                message_metadata.push(MessagePageMetadata {
                                    message_id: message.id.clone(),
                                    token_meta: None,
                                    run_meta: None,
                                });
                                messages.push(message);
                            }
                            let latest_token_usage = match authority
                                .repository
                                .query(crate::domain::local_event::LocalEventQuery::SessionProjectionByIdentity {
                                    session_id,
                                })
                                .await
                                .map_err(|error| {
                                    format!("agent SQLite projection read failed: {error}")
                                })?
                            {
                                crate::domain::local_event::LocalEventQueryResult::SessionProjectionByIdentity(Some(projection)) => {
                                    codec.decode(&projection.projection)?.latest_token_usage
                                }
                                crate::domain::local_event::LocalEventQueryResult::SessionProjectionByIdentity(None) => None,
                                _ => return Err("agent SQLite projection query returned the wrong shape".to_string()),
                            };
                            let next_cursor = page
                                .next_before_position
                                .map(|position| {
                                    u64::try_from(position)
                                        .map(PageCursor)
                                        .map_err(|_| "agent message page cursor is invalid".to_string())
                                })
                                .transpose()?;
                            Ok(SessionPage {
                                messages,
                                message_metadata,
                                has_more: next_cursor.is_some(),
                                next_cursor,
                                total_count: page.total_count,
                                latest_token_usage,
                            })
                        })
                })
                .join()
                .map_err(|_| "agent SQLite message page worker panicked".to_string())?
        })
    }

    fn canonical_all_messages(&self, session_id: &str) -> Result<Vec<ChatMessage>, String> {
        let mut cursor = None;
        let mut chunks = Vec::new();
        loop {
            let page = self.canonical_message_page(session_id, cursor, 200)?;
            let next = page.next_cursor;
            chunks.push(page.messages);
            let Some(next) = next else {
                break;
            };
            cursor = Some(next);
        }
        chunks.reverse();
        Ok(chunks.into_iter().flatten().collect())
    }

    fn commit_agent_events(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        events: &[AgentSessionEvent],
    ) -> Result<(), String> {
        self.commit_agent_events_with_kind_and_queue_front(
            app_data_dir,
            session_id,
            events,
            crate::domain::local_event::CommitOperationKind::Projection,
            None,
            Vec::new(),
            None,
        )
    }

    fn commit_agent_events_with_queue_pause_guard(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        events: &[AgentSessionEvent],
        expected_queue_paused: bool,
    ) -> Result<(), String> {
        self.commit_agent_events_with_kind_and_queue_front(
            app_data_dir,
            session_id,
            events,
            crate::domain::local_event::CommitOperationKind::Projection,
            None,
            Vec::new(),
            Some(expected_queue_paused),
        )
    }

    fn commit_agent_events_with_kind(
        &self,
        _app_data_dir: &Path,
        session_id: &str,
        events: &[AgentSessionEvent],
        operation_kind: crate::domain::local_event::CommitOperationKind,
    ) -> Result<(), String> {
        self.commit_agent_events_with_kind_and_queue_front(
            _app_data_dir,
            session_id,
            events,
            operation_kind,
            None,
            Vec::new(),
            None,
        )
    }

    #[allow(clippy::too_many_arguments)] // One atomic projection boundary receives each guard and participant explicitly.
    fn commit_agent_events_with_kind_and_queue_front(
        &self,
        _app_data_dir: &Path,
        session_id: &str,
        events: &[AgentSessionEvent],
        operation_kind: crate::domain::local_event::CommitOperationKind,
        expected_queue_front: Option<ExpectedAcceptedQueueFront>,
        additional_mutations: Vec<crate::domain::local_event::LocalStateMutation>,
        expected_queue_paused: Option<bool>,
    ) -> Result<(), String> {
        if events.is_empty() {
            return Ok(());
        }
        let Some(authority) = self.event_authority.read().clone() else {
            #[cfg(test)]
            return Ok(());
            #[cfg(not(test))]
            return Err("agent-session SQLite event authority is not configured".to_string());
        };
        let fallback_meta = Some(
            self.read_session_projection(session_id)?
                .ok_or_else(|| format!("Session projection not found: {session_id}"))?
                .meta,
        );
        #[cfg(test)]
        let atomic_event_commit_hook = self.atomic_event_commit_hook.read().clone();
        let session_id = session_id.to_string();
        let events = events.to_vec();
        let expected_queue_front = expected_queue_front.clone();
        let additional_mutations = additional_mutations.clone();
        std::thread::scope(|scope| {
            scope
                .spawn(move || {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|error| {
                            format!("failed to create agent event commit runtime: {error}")
                        })?
                        .block_on(async move {
                            let stream_id = crate::domain::local_event::StreamId::agent_session(
                                &session_id,
                            )
                            .map_err(|_| "agent session stream identity is invalid".to_string())?;
                            let codec = authority.projection_codec.as_ref();
                            let encoded_events = codec.encode_events_for_identity(&events)?;
                            let mut mutation_identities =
                                Vec::with_capacity(additional_mutations.len());
                            for mutation in &additional_mutations {
                                let encoded = authority
                                    .repository
                                    .canonical_mutation_identity_v1(mutation)?;
                                mutation_identities.push(encoded);
                            }
                            let payload_hash =
                                crate::domain::local_event::agent_event_payload_identity(
                                    &session_id,
                                    &encoded_events,
                                    mutation_identities.iter().map(Vec::as_slice),
                                );
                            for _ in 0..4 {
                                // Stream head metadata is returned with every page. A one-row
                                // read is sufficient for optimistic append; normal mutation state
                                // comes from the point-addressed session projection below.
                                let head = authority
                                    .repository
                                    .load_stream(crate::domain::local_event::LoadStreamRequest {
                                        stream_id: stream_id.clone(),
                                        after: None,
                                        limit: 1,
                                    })
                                    .await
                                    .map_err(|error| {
                                        format!("agent SQLite head read failed: {error}")
                                    })?
                                    .head;
                                let mut state_mutations = Vec::new();
                                {
                                    let codec = authority.projection_codec.as_ref();
                                    // Projection, revision, and recovery inputs come from one
                                    // SQLite reader snapshot; the projection mutation below
                                    // retains that revision as its commit fence.
                                    let result = authority
                                        .repository
                                        .query(crate::domain::local_event::LocalEventQuery::AgentSessionLifecycleSnapshot {
                                            session_id: session_id.clone(),
                                        })
                                        .await
                                        .map_err(|error| {
                                            format!("agent SQLite lifecycle snapshot read failed: {error}")
                                        })?;
                                    let crate::domain::local_event::LocalEventQueryResult::AgentSessionLifecycleSnapshot(
                                        stored,
                                    ) = result
                                    else {
                                        return Err("agent SQLite lifecycle snapshot query returned the wrong shape".to_string());
                                    };
                                    let (
                                        mut meta,
                                        title,
                                        mut reducer_events,
                                        queue_paused_at,
                                        mut pending_send_queue,
                                        mut session_aggregate,
                                        expected,
                                        revision,
                                    ) =
                                        match stored {
                                        Some(snapshot) => {
                                            let pending_obligations = snapshot.pending_obligations;
                                            let stored = snapshot.session;
                                            let decoded = codec.decode(&stored.projection)?;
                                            let session_aggregate = codec
                                                .restore_session_aggregate(
                                                    &decoded,
                                                    &pending_obligations,
                                                )?;
                                            let next = stored.revision.next().ok_or_else(|| {
                                                "agent projection revision exhausted".to_string()
                                            })?;
                                            (
                                                decoded.meta,
                                                decoded.title,
                                                decoded.reducer_events,
                                                decoded.queue_paused_at,
                                                decoded.pending_send_queue,
                                                session_aggregate,
                                                crate::domain::local_event::RevisionGuard::Expected(stored.revision),
                                                next,
                                            )
                                        }
                                        None => {
                                            let meta = fallback_meta.clone().ok_or_else(|| {
                                                "agent projection has no initialization metadata".to_string()
                                            })?;
                                            let session_aggregate =
                                                SessionAggregate::new(meta.id.clone()).map_err(
                                                    |error| {
                                                        format!(
                                                            "invalid initial Session aggregate: {error:?}"
                                                        )
                                                    },
                                                )?;
                                            (
                                                meta,
                                                None,
                                                Vec::new(),
                                                None,
                                                Vec::new(),
                                                session_aggregate,
                                                crate::domain::local_event::RevisionGuard::Absent,
                                                crate::domain::local_event::Revision::new(0)
                                                    .expect("zero revision"),
                                            )
                                        }
                                    };
                                    if expected_queue_paused.is_some_and(|expected| {
                                        expected != queue_paused_at.is_some()
                                    }) {
                                        return Err(
                                            "queue-pause authority changed before guarded event commit; retry"
                                                .to_string(),
                                        );
                                    }
                                    let mut previous_turn_id = meta.last_turn_id.unwrap_or(0);
                                    let mut consumed_expected_queue_front = false;
                                    for event in &events {
                                        if let AgentSessionEvent::TurnStarted {
                                            turn_id,
                                            message_id,
                                            ..
                                        } = event
                                        {
                                            if !turn_identity_advances(previous_turn_id, *turn_id) {
                                                return Err(format!(
                                                    "turn identity {turn_id} does not advance durable turn {previous_turn_id}"
                                                ));
                                            }
                                            match expected_queue_front.as_ref() {
                                                Some(expected_front) => {
                                                    session_aggregate
                                                        .apply_queue_start(
                                                            &expected_front.queue_item_id,
                                                            message_id,
                                                            Turn::start(*turn_id),
                                                        )
                                                        .map_err(|rejection| match rejection {
                                                            QueueStartRejection::InvalidReservedTurnIdentity => {
                                                                "canonical queue front has an invalid turn identity".to_string()
                                                            }
                                                            QueueStartRejection::IdentityMismatch => {
                                                                "accepted queued turn does not match the canonical queue front".to_string()
                                                            }
                                                            QueueStartRejection::Transition(
                                                                TransitionRejection::QueuePaused,
                                                            ) => format!(
                                                                "{ACCEPTED_QUEUE_START_BLOCKED}: canonical queue is paused"
                                                            ),
                                                            QueueStartRejection::Transition(
                                                                TransitionRejection::UnresolvedRecovery,
                                                            ) => format!(
                                                                "{ACCEPTED_QUEUE_START_BLOCKED}: canonical backend recovery is active"
                                                            ),
                                                            QueueStartRejection::Transition(_) => format!(
                                                                "{ACCEPTED_QUEUE_START_BLOCKED}: canonical turn is active or session state is {:?}",
                                                                meta.state,
                                                            ),
                                                        })?;
                                                    pending_send_queue.remove(0);
                                                    consumed_expected_queue_front = true;
                                                }
                                                None => match session_aggregate
                                                    .apply_observed_turn_start(Turn::start(*turn_id))
                                                {
                                                    TransitionOutcome::Applied => {}
                                                    TransitionOutcome::Rejected(
                                                        TransitionRejection::QueueNotEmpty,
                                                    ) => {
                                                        return Err(
                                                            "turn start cannot bypass the canonical queue front"
                                                                .to_string(),
                                                        );
                                                    }
                                                    _ => {
                                                        return Err(format!(
                                                            "canonical turn is active or session state is {:?}",
                                                            meta.state,
                                                        ));
                                                    }
                                                },
                                            }
                                            previous_turn_id = *turn_id;
                                        }
                                    }
                                    if expected_queue_front.is_some()
                                        && !consumed_expected_queue_front
                                    {
                                        return Err(
                                            "accepted queued turn commit omitted TurnStarted"
                                                .to_string(),
                                        );
                                    }
                                    reducer_events =
                                        bounded_reducer_events(reducer_events, &events);
                                    let last_turn_interruption =
                                        latest_turn_interruption(&reducer_events);
                                    let last_turn_id = reducer_events.iter().rev().find_map(
                                        |event| match event {
                                            AgentSessionEvent::TurnStarted { turn_id, .. } => {
                                                Some(*turn_id)
                                            }
                                            _ => None,
                                        },
                                    );
                                    let mut touched_message_ids = reducer_events
                                        .iter()
                                        .rev()
                                        .find_map(|event| match event {
                                            AgentSessionEvent::TurnStarted {
                                                message_id,
                                                assistant_message_id,
                                                ..
                                            } => Some([
                                                message_id.clone(),
                                                assistant_message_id.clone().unwrap_or_else(|| {
                                                    format!("{message_id}:agent")
                                                }),
                                            ]),
                                            _ => None,
                                        })
                                        .into_iter()
                                        .flatten()
                                        .collect::<HashSet<_>>();
                                    for event in &events {
                                        if let AgentSessionEvent::SessionErrored {
                                            message_id,
                                            ..
                                        } = event
                                        {
                                            touched_message_ids.insert(message_id.clone());
                                        }
                                    }
                                    let projected =
                                        TurnEventLog::from_events(reducer_events.clone()).project();
                                    meta.state = projected.status.session_state;
                                    meta.error_reason = error_reason_for_state(
                                        &meta.state,
                                        &projected.error_reason,
                                    );
                                    meta.state_revision = next_sqlite_counter(
                                        meta.state_revision,
                                        "session state revision",
                                    )?;
                                    meta.last_turn_interruption = last_turn_interruption;
                                    meta.last_turn_id = last_turn_id;
                                    let latest_token_usage = projected
                                        .workflow_turn_complete
                                        .as_ref()
                                        .and_then(|turn| turn.token_usage)
                                        .map(|usage| TokenUsage {
                                            input_tokens: usage.input_tokens,
                                            output_tokens: usage.output_tokens,
                                            total_tokens: usage.total_tokens(),
                                            context_window_tokens: None,
                                        });
                                    let mut inserted_messages = Vec::new();
                                    for message in projected.messages.iter().filter(|message| {
                                        touched_message_ids.contains(&message.id)
                                    }) {
                                        let encoded_message = codec.encode_message(message)?;
                                        let result = authority
                                            .repository
                                            .query(crate::domain::local_event::LocalEventQuery::MessageProjectionByIdentity {
                                                session_id: session_id.clone(),
                                                message_id: message.id.clone(),
                                            })
                                            .await
                                            .map_err(|error| {
                                                format!("agent SQLite message projection read failed: {error}")
                                            })?;
                                        let crate::domain::local_event::LocalEventQueryResult::MessageProjectionByIdentity(
                                            stored,
                                        ) = result
                                        else {
                                            return Err("agent SQLite message projection query returned the wrong shape".to_string());
                                        };
                                        if stored.as_ref().is_some_and(|stored| {
                                            stored.projection == encoded_message
                                        }) {
                                            continue;
                                        }
                                        let (expected, revision) = match stored {
                                            Some(stored) => (
                                                crate::domain::local_event::RevisionGuard::Expected(stored.revision),
                                                stored.revision.next().ok_or_else(|| {
                                                    "agent message projection revision exhausted".to_string()
                                                })?,
                                            ),
                                            None => {
                                                inserted_messages.push(message.clone());
                                                (
                                                    crate::domain::local_event::RevisionGuard::Absent,
                                                    crate::domain::local_event::Revision::new(0)
                                                        .expect("zero revision"),
                                                )
                                            }
                                        };
                                        state_mutations.push(
                                            crate::domain::local_event::LocalStateMutation::MessageProjection(
                                                crate::domain::local_event::MessageProjectionMutation {
                                                    session_id: session_id.clone(),
                                                    message_id: message.id.clone(),
                                                    projection: encoded_message,
                                                    expected,
                                                    revision,
                                                },
                                            ),
                                        );
                                    }
                                    if !inserted_messages.is_empty() {
                                        meta.message_count = add_sqlite_count(
                                            meta.message_count,
                                            inserted_messages.len(),
                                            "session message count",
                                        )?;
                                        if meta.first_message_preview.is_empty() {
                                            meta.first_message_preview =
                                                super::first_message_preview(&inserted_messages);
                                        }
                                    }
                                    let projection = codec.encode(
                                        &CanonicalAgentSessionProjection {
                                            meta,
                                            title,
                                            messages: Vec::new(),
                                            reducer_events,
                                            queue_paused_at: projected.queue_paused_at,
                                            latest_token_usage,
                                            pending_send_queue,
                                        },
                                    )?;
                                    state_mutations.insert(
                                        0,
                                        crate::domain::local_event::LocalStateMutation::SessionProjection(
                                            crate::domain::local_event::SessionProjectionMutation {
                                                session_id: session_id.clone(),
                                                projection,
                                                expected,
                                                revision,
                                            },
                                        ),
                                    );
                                }
                                state_mutations.extend(additional_mutations.clone());
                                let identity = format!(
                                    "session-event-{}",
                                    hex_lower(payload_hash)
                                );
                                let occurred_at_ms = (now_timestamp() * 1000.0).round() as i64;
                                #[cfg(test)]
                                if let Some(hook) = &atomic_event_commit_hook {
                                    hook(operation_kind)?;
                                }
                                let batch = crate::domain::local_event::LocalAtomicBatch {
                                    commit_id: crate::domain::local_event::CommitIdentity::parse(
                                        &identity,
                                    )
                                    .map_err(|_| {
                                        "agent event commit identity is invalid".to_string()
                                    })?,
                                    idempotency: crate::domain::local_event::IdempotencyBinding {
                                        installation_id: authority.installation_id.clone(),
                                        operation_kind,
                                        idempotency_key: hex_lower(payload_hash),
                                        payload_hash,
                                    },
                                    expected_heads: vec![
                                        crate::domain::local_event::ExpectedStreamHead {
                                            stream_id: stream_id.clone(),
                                            expected: head,
                                        },
                                    ],
                                    events: events
                                        .iter()
                                        .cloned()
                                        .map(|event| {
                                            crate::domain::local_event::UncommittedDomainEvent {
                                                stream_id: stream_id.clone(),
                                                event: crate::domain::local_event::LocalDomainEvent::AgentSession(event),
                                                occurred_at_ms,
                                            }
                                        })
                                        .collect(),
                                    state_mutations,
                                };
                                match authority.repository.commit_batch(batch).await {
                                    Ok(_) => return Ok(()),
                                    Err(crate::domain::local_event::CommitBatchError::EffectAdmissionBlocked)
                                        if expected_queue_front.is_some() =>
                                    {
                                        return Err(format!(
                                            "{ACCEPTED_QUEUE_START_BLOCKED}: unresolved owner recovery is active"
                                        ));
                                    }
                                    Err(crate::domain::local_event::CommitBatchError::StreamHeadConflict { .. })
                                        if expected_queue_paused.is_some() =>
                                    {
                                        return Err(
                                            "queue-pause authority changed during guarded event commit; retry"
                                                .to_string(),
                                        );
                                    }
                                    Err(crate::domain::local_event::CommitBatchError::StreamHeadConflict { .. }) => continue,
                                    Err(crate::domain::local_event::CommitBatchError::PayloadConflict)
                                        if expected_queue_paused.is_some() =>
                                    {
                                        return Err(
                                            "queue-pause authority changed during guarded event commit; retry"
                                                .to_string(),
                                        );
                                    }
                                    Err(crate::domain::local_event::CommitBatchError::PayloadConflict)
                                        if expected_queue_front.is_some() => continue,
                                    Err(crate::domain::local_event::CommitBatchError::OutcomeUnknown { identity }) => {
                                        match authority.repository.resolve_commit(identity).await {
                                            Ok(crate::domain::local_event::CommitResolution::Committed(_)) => return Ok(()),
                                            Ok(crate::domain::local_event::CommitResolution::NotCommitted) => continue,
                                            Err(error) => {
                                                return Err(format!(
                                                    "accepted queued turn commit outcome could not be resolved: {error}"
                                                ));
                                            }
                                        }
                                    }
                                    Err(error) => {
                                        return Err(format!(
                                            "agent SQLite event commit failed: {error}"
                                        ));
                                    }
                                }
                            }
                            Err("agent SQLite event commit remained contended".to_string())
                        })
                })
                .join()
                .map_err(|_| "agent SQLite event commit worker panicked".to_string())?
        })
    }

    #[allow(clippy::too_many_arguments)] // One transaction boundary receives every atomic participant explicitly.
    fn commit_agent_events_with_additional_mutations(
        &self,
        session_id: &str,
        events: &[AgentSessionEvent],
        additional_mutations: Vec<crate::domain::local_event::LocalStateMutation>,
        terminal_message_patch: Option<TerminalMessageProjectionPatch>,
        projection_meta_patch: Option<EventProjectionMetaPatch>,
        terminal_participant: Option<(
            Arc<dyn RuntimeTerminalParticipantProvider>,
            crate::domain::local_event::TerminalRecordMutation,
        )>,
        operation_kind: crate::domain::local_event::CommitOperationKind,
    ) -> Result<(), String> {
        if events.is_empty()
            && additional_mutations.is_empty()
            && terminal_message_patch.is_none()
            && projection_meta_patch.is_none()
            && terminal_participant.is_none()
        {
            return Ok(());
        }
        let authority = self
            .event_authority
            .read()
            .clone()
            .ok_or_else(|| "agent-session SQLite authority is unavailable".to_string())?;
        let current_projection = self.read_session_projection(session_id)?;
        let fallback_meta = current_projection
            .as_ref()
            .map(|projection| projection.meta.clone());
        let terminal_pause_retry_required = terminal_requires_queue_pause(events);
        let supplies_queue_pause = events
            .iter()
            .any(|event| matches!(event, AgentSessionEvent::QueuePaused { .. }));
        let expected_terminal_queue_paused =
            terminal_pause_retry_required.then_some(!supplies_queue_pause);
        if let Some(expected_queue_paused) = expected_terminal_queue_paused {
            let queue_is_paused = current_projection
                .as_ref()
                .is_some_and(|projection| projection.queue_paused_at.is_some());
            if expected_queue_paused != queue_is_paused {
                return Err(
                    "terminal queue-pause authority changed before atomic commit; retry"
                        .to_string(),
                );
            }
        }
        #[cfg(test)]
        let atomic_event_commit_hook = self.atomic_event_commit_hook.read().clone();
        let session_id = session_id.to_string();
        let events = events.to_vec();
        std::thread::scope(|scope| {
            scope
                .spawn(move || {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|error| {
                            format!("failed to create atomic agent event runtime: {error}")
                        })?
                        .block_on(async move {
                            let stream_id = crate::domain::local_event::StreamId::agent_session(
                                &session_id,
                            )
                            .map_err(|_| "agent session stream identity is invalid".to_string())?;
                            let codec = authority.projection_codec.as_ref();
                            let encoded_events = codec.encode_events_for_identity(&events)?;
                            let mut mutation_identities =
                                Vec::with_capacity(additional_mutations.len());
                            for mutation in &additional_mutations {
                                let encoded = authority
                                    .repository
                                    .canonical_mutation_identity_v1(mutation)?;
                                mutation_identities.push(encoded);
                            }
                            let identity =
                                crate::domain::local_event::agent_atomic_event_payload_identity(
                                    &session_id,
                                    operation_kind.label(),
                                    &encoded_events,
                                    mutation_identities.iter().map(Vec::as_slice),
                                    |identity| {
                                        if let Some(patch) = &terminal_message_patch {
                                            codec.hash_terminal_message_projection_patch(
                                                identity, patch,
                                            )?;
                                        }
                                        if let Some(patch) = &projection_meta_patch {
                                            codec.hash_event_projection_meta_patch(identity, patch)?;
                                        }
                                        Ok::<(), String>(())
                                    },
                                )?;
                            let payload_hash = identity.digest;
                            let identity = identity.identity;
                            let commit_id =
                                crate::domain::local_event::CommitIdentity::parse(&identity)
                                    .map_err(|_| {
                                        "atomic agent event commit identity is invalid".to_string()
                                    })?;
                            let occurred_at_ms = (now_timestamp() * 1000.0).round() as i64;
                            for _ in 0..4 {
                                let head = authority
                                    .repository
                                    .load_stream(crate::domain::local_event::LoadStreamRequest {
                                        stream_id: stream_id.clone(),
                                        after: None,
                                        limit: 1,
                                    })
                                    .await
                                    .map_err(|error| {
                                        format!("agent SQLite head read failed: {error}")
                                    })?
                                    .head;
                                let mut state_mutations =
                                    prepare_canonical_event_projection_mutations(
                                        &authority,
                                        &session_id,
                                        &events,
                                        fallback_meta.clone(),
                                        terminal_message_patch.as_ref(),
                                        expected_terminal_queue_paused,
                                    )
                                    .await?;
                                if let Some(patch) = &projection_meta_patch {
                                    let codec = authority.projection_codec.as_ref();
                                    patch_event_projection_meta(
                                        codec,
                                        &mut state_mutations,
                                        patch,
                                    )?;
                                }
                                state_mutations.extend(additional_mutations.clone());
                                let participant_events =
                                    if let Some((provider, terminal)) = &terminal_participant {
                                    // This point-query must happen after the session projection
                                    // read above. Stop acceptance mutates that same projection,
                                    // so an acceptance racing after a `none` answer makes this
                                    // batch conflict and the next loop re-queries participants.
                                        let participants = provider.prepare(terminal).await?;
                                        state_mutations.extend(participants.mutations);
                                        participants.events
                                    } else {
                                        Vec::new()
                                    };
                                #[cfg(test)]
                                if let Some(hook) = &atomic_event_commit_hook {
                                    hook(operation_kind)?;
                                }
                                let batch = crate::domain::local_event::LocalAtomicBatch {
                                    commit_id: commit_id.clone(),
                                    idempotency: crate::domain::local_event::IdempotencyBinding {
                                        installation_id: authority.installation_id.clone(),
                                        operation_kind,
                                        idempotency_key: hex_lower(payload_hash),
                                        payload_hash,
                                    },
                                    expected_heads: vec![
                                        crate::domain::local_event::ExpectedStreamHead {
                                            stream_id: stream_id.clone(),
                                            expected: head,
                                        },
                                    ],
                                    events: events
                                        .iter()
                                        .chain(participant_events.iter())
                                        .cloned()
                                        .map(|event| {
                                            crate::domain::local_event::UncommittedDomainEvent {
                                                stream_id: stream_id.clone(),
                                                event: crate::domain::local_event::LocalDomainEvent::AgentSession(event),
                                                occurred_at_ms,
                                            }
                                        })
                                        .collect(),
                                    state_mutations,
                                };
                                match authority.repository.commit_batch(batch).await {
                                    Ok(_) => return Ok(()),
                                    Err(crate::domain::local_event::CommitBatchError::StreamHeadConflict { .. })
                                        if terminal_pause_retry_required =>
                                    {
                                        return Err(
                                            "terminal queue-pause authority changed during atomic commit; retry"
                                                .to_string(),
                                        );
                                    }
                                    Err(crate::domain::local_event::CommitBatchError::StreamHeadConflict { .. }) => continue,
                                    Err(crate::domain::local_event::CommitBatchError::PayloadConflict)
                                        if terminal_pause_retry_required =>
                                    {
                                        return Err(
                                            "terminal queue-pause authority changed during atomic commit; retry"
                                                .to_string(),
                                        );
                                    }
                                    Err(crate::domain::local_event::CommitBatchError::PayloadConflict)
                                        if terminal_participant.is_some() => continue,
                                    Err(crate::domain::local_event::CommitBatchError::PayloadConflict)
                                        if projection_meta_patch.is_some() => continue,
                                    Err(crate::domain::local_event::CommitBatchError::OutcomeUnknown { identity }) => {
                                        match authority.repository.resolve_commit(identity).await {
                                            Ok(crate::domain::local_event::CommitResolution::Committed(_)) => return Ok(()),
                                            Ok(crate::domain::local_event::CommitResolution::NotCommitted) => continue,
                                            Err(error) => return Err(format!("atomic agent event commit outcome could not be resolved: {error}")),
                                        }
                                    }
                                    Err(error) => {
                                        return Err(format!(
                                            "atomic agent event commit failed: {error}"
                                        ));
                                    }
                                }
                            }
                            Err("atomic agent event commit remained contended".to_string())
                        })
                })
                .join()
                .map_err(|_| "atomic agent event commit worker panicked".to_string())?
        })
    }

    fn commit_session_projection_snapshot(
        &self,
        projection: CanonicalAgentSessionProjection,
    ) -> Result<(), String> {
        self.commit_session_projection_snapshot_with_kind(
            projection,
            crate::domain::local_event::CommitOperationKind::Projection,
        )
    }

    fn commit_user_session_projection_snapshot(
        &self,
        projection: CanonicalAgentSessionProjection,
    ) -> Result<(), String> {
        self.commit_session_projection_snapshot_with_kind(
            projection,
            crate::domain::local_event::CommitOperationKind::UserMutation,
        )
    }

    fn commit_session_projection_snapshot_with_kind(
        &self,
        projection: CanonicalAgentSessionProjection,
        operation_kind: crate::domain::local_event::CommitOperationKind,
    ) -> Result<(), String> {
        self.commit_session_projection_snapshot_with_kind_and_mutations(
            projection,
            operation_kind,
            Vec::new(),
        )
    }

    fn commit_session_projection_snapshot_with_kind_and_mutations(
        &self,
        mut projection: CanonicalAgentSessionProjection,
        operation_kind: crate::domain::local_event::CommitOperationKind,
        additional_mutations: Vec<crate::domain::local_event::LocalStateMutation>,
    ) -> Result<(), String> {
        let Some(authority) = self.event_authority.read().clone() else {
            #[cfg(test)]
            return Ok(());
            #[cfg(not(test))]
            return Err("agent-session SQLite event authority is not configured".to_string());
        };
        let codec = authority.projection_codec.clone();
        let session_id = projection.meta.id.clone();
        let content_blobs = codec.externalize_message_content(&mut projection.messages)?;
        let encoded_messages = projection
            .messages
            .iter()
            .map(|message| {
                codec
                    .encode_message(message)
                    .map(|encoded| (message.id.clone(), encoded))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let encoded = codec.encode(&projection)?;
        let encoded_identity_v1 = codec.encode_session_identity_v1(&encoded)?;
        std::thread::scope(|scope| {
            scope
                .spawn(move || {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|error| {
                            format!("failed to create agent projection commit runtime: {error}")
                        })?
                        .block_on(async move {
                            let result = authority
                                .repository
                                .query(crate::domain::local_event::LocalEventQuery::SessionProjectionByIdentity {
                                    session_id: session_id.clone(),
                                })
                                .await
                                .map_err(|error| {
                                    format!("agent SQLite projection read failed: {error}")
                                })?;
                            let crate::domain::local_event::LocalEventQueryResult::SessionProjectionByIdentity(
                                stored,
                            ) = result
                            else {
                                return Err("agent SQLite projection query returned the wrong shape".to_string());
                            };
                            if encoded_messages.is_empty()
                                && additional_mutations.is_empty()
                                && stored.as_ref().is_some_and(|stored| {
                                    stored.projection == encoded
                                })
                            {
                                return Ok(());
                            }
                            let (expected, revision) = match stored {
                                Some(stored) => (
                                    crate::domain::local_event::RevisionGuard::Expected(stored.revision),
                                    stored.revision.next().ok_or_else(|| {
                                        "agent projection revision exhausted".to_string()
                                    })?,
                                ),
                                None => (
                                    crate::domain::local_event::RevisionGuard::Absent,
                                    crate::domain::local_event::Revision::new(0)
                                        .expect("zero revision"),
                                ),
                            };
                            let mut message_identities =
                                Vec::with_capacity(encoded_messages.len());
                            for (message_id, message) in &encoded_messages {
                                let message_identity_v1 =
                                    codec.encode_message_identity_v1(message)?;
                                message_identities.push((message_id.clone(), message_identity_v1));
                            }
                            let mut mutation_identities =
                                Vec::with_capacity(additional_mutations.len());
                            for mutation in &additional_mutations {
                                let encoded = authority
                                    .repository
                                    .canonical_mutation_identity_v1(mutation)?;
                                mutation_identities.push(encoded);
                            }
                            let binding =
                                crate::domain::local_event::session_projection_binding_identity(
                                    &encoded_identity_v1,
                                    revision.value(),
                                    message_identities
                                        .iter()
                                        .map(|(message_id, encoded)| {
                                            (message_id.as_str(), encoded.as_slice())
                                        }),
                                    mutation_identities.iter().map(Vec::as_slice),
                                );
                            let binding_hash = binding.digest;
                            let identity = binding.identity;
                            let mut state_mutations = vec![
                                crate::domain::local_event::LocalStateMutation::SessionProjection(
                                    crate::domain::local_event::SessionProjectionMutation {
                                        session_id: session_id.clone(),
                                        projection: encoded,
                                        expected,
                                        revision,
                                    },
                                ),
                            ];
                            state_mutations.extend(
                                prepare_canonical_content_blob_mutations(
                                    &authority.repository,
                                    &session_id,
                                    content_blobs,
                                )
                                .await?,
                            );
                            for (message_id, encoded_message) in encoded_messages {
                                let result = authority
                                    .repository
                                    .query(crate::domain::local_event::LocalEventQuery::MessageProjectionByIdentity {
                                        session_id: session_id.clone(),
                                        message_id: message_id.clone(),
                                    })
                                    .await
                                    .map_err(|error| {
                                        format!("agent SQLite message projection read failed: {error}")
                                    })?;
                                let crate::domain::local_event::LocalEventQueryResult::MessageProjectionByIdentity(
                                    stored,
                                ) = result
                                else {
                                    return Err("agent SQLite message projection query returned the wrong shape".to_string());
                                };
                                if stored.as_ref().is_some_and(|stored| {
                                    stored.projection == encoded_message
                                }) {
                                    continue;
                                }
                                let (expected, revision) = match stored {
                                    Some(stored) => (
                                        crate::domain::local_event::RevisionGuard::Expected(stored.revision),
                                        stored.revision.next().ok_or_else(|| {
                                            "agent message projection revision exhausted".to_string()
                                        })?,
                                    ),
                                    None => (
                                        crate::domain::local_event::RevisionGuard::Absent,
                                        crate::domain::local_event::Revision::new(0)
                                            .expect("zero revision"),
                                    ),
                                };
                                state_mutations.push(
                                    crate::domain::local_event::LocalStateMutation::MessageProjection(
                                        crate::domain::local_event::MessageProjectionMutation {
                                            session_id: session_id.clone(),
                                            message_id,
                                            projection: encoded_message,
                                            expected,
                                            revision,
                                        },
                                    ),
                                );
                            }
                            state_mutations.extend(additional_mutations);
                            let batch = crate::domain::local_event::LocalAtomicBatch {
                                commit_id: crate::domain::local_event::CommitIdentity::parse(
                                    &identity,
                                )
                                .map_err(|_| {
                                    "agent projection commit identity is invalid".to_string()
                                })?,
                                idempotency: crate::domain::local_event::IdempotencyBinding {
                                    installation_id: authority.installation_id.clone(),
                                    operation_kind,
                                    idempotency_key: hex_lower(binding_hash),
                                    payload_hash: binding_hash,
                                },
                                expected_heads: Vec::new(),
                                events: Vec::new(),
                                state_mutations,
                            };
                            authority
                                .repository
                                .commit_batch(batch)
                                .await
                                .map(|_| ())
                                .map_err(|error| {
                                    format!("agent SQLite projection commit failed: {error}")
                                })
                        })
                })
                .join()
                .map_err(|_| "agent SQLite projection commit worker panicked".to_string())?
        })
    }

}
