mod tests {
    use std::collections::HashSet;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Barrier};

    use super::*;

    fn ids(values: impl IntoIterator<Item = String>) -> HashSet<String> {
        values.into_iter().collect()
    }

    fn turn_started(turn_id: u64) -> AgentSessionEvent {
        AgentSessionEvent::TurnStarted {
            turn_id,
            message_id: format!("human-{turn_id}"),
            assistant_message_id: Some(format!("agent-{turn_id}")),
            prompt: crate::domain::agent_session::events::PromptInput::default(),
            at: turn_id as f64,
        }
    }

    fn interrupted(turn_id: u64) -> AgentSessionEvent {
        AgentSessionEvent::TurnInterrupted {
            turn_id,
            reason: crate::domain::agent_session::events::InterruptReason::Abort,
            exit_code: 130,
            error: None,
        }
    }

    fn completed_recovery_after_failure() -> Vec<AgentSessionEvent> {
        vec![
            AgentSessionEvent::BackendSessionRecoveryStarted {
                recovery_id: "recovery-a".to_string(),
                old_provider_session_generation: 0,
                reason: BackendSessionRecoveryReason::BackendSessionLost,
                at: 1.0,
            },
            AgentSessionEvent::BackendSessionRecoveryFailed {
                recovery_id: "recovery-a".to_string(),
                error: "first recovery failed".to_string(),
                at: 2.0,
            },
            AgentSessionEvent::BackendSessionRecoveryStarted {
                recovery_id: "recovery-b".to_string(),
                old_provider_session_generation: 0,
                reason: BackendSessionRecoveryReason::BackendSessionLost,
                at: 3.0,
            },
            AgentSessionEvent::BackendSessionRecoveryCompleted {
                recovery_id: "recovery-b".to_string(),
                provider_session_generation: 1,
                at: 4.0,
            },
        ]
    }

    fn publish_recovery_message(
        writer: &SessionStore,
        app_data_dir: &Path,
        session_id: &str,
        pending: &PendingRecoveryMessage,
    ) {
        let message_id = match pending {
            PendingRecoveryMessage::Notice { message_id, .. }
            | PendingRecoveryMessage::Error { message_id, .. } => message_id.clone(),
        };
        let message = ChatMessage {
            id: message_id,
            role: MessageRole::Agent,
            content: String::new(),
            thinking: None,
            activities: None,
            parts: Some(Vec::new()),
            streaming_final_seq: 0,
            timestamp: now_timestamp(),
            mentions: None,
        };
        assert!(writer
            .publish_pending_recovery_message(app_data_dir, session_id, pending, message)
            .unwrap());
    }

    fn stop_resolution(turn_id: u64) -> AgentSessionEvent {
        AgentSessionEvent::StopResolutionRecorded {
            operation_id: format!("stop-{turn_id}"),
            turn_id,
            resolution: crate::domain::agent_session::events::StopResolution::Superseded,
            at: 9.0,
        }
    }

    #[test]
    fn backend_recovery_obligation_mutation_keeps_closed_identity_and_detail() {
        let record = crate::domain::local_event::ObligationRecord::BackendSessionRecovery {
            session_id: "session-1".to_string(),
            recovery_id: "recovery-1".to_string(),
            detail:
                crate::domain::local_event::BackendSessionRecoveryObligationRecord::EffectReserved {
                    old_provider_session_generation: 7,
                    reason: BackendSessionRecoveryReason::BackendSessionLost,
                    reserved_at_bits: 42,
                },
            state: crate::domain::local_event::ObligationStateRecord::EffectReserved,
        };
        let mutation = backend_recovery_obligation_mutation(
            "backend-recovery:session-1:recovery-1".to_string(),
            record.clone(),
            None,
            None,
        )
        .unwrap();

        assert!(matches!(
            mutation,
            crate::domain::local_event::LocalStateMutation::Obligation(
                crate::domain::local_event::ObligationMutation {
                    obligation_id,
                    record: stored,
                    ..
                }
            ) if obligation_id == "backend-recovery:session-1:recovery-1" && stored == record
        ));
    }

    #[test]
    fn terminal_projection_drops_same_turn_duplicate_but_keeps_resolution_fact() {
        let previous = vec![turn_started(4), interrupted(4)];
        let supplied = vec![
            AgentSessionEvent::FinalPartsRecorded {
                turn_id: 4,
                message_id: "agent-4".to_string(),
                parts: Vec::new(),
            },
            interrupted(4),
            stop_resolution(4),
        ];

        assert_eq!(
            complete_terminal_projection_events(&previous, &supplied),
            vec![stop_resolution(4)]
        );
    }

    #[test]
    fn terminal_projection_ignores_old_turn_candidate_after_newer_turn_started() {
        let previous = vec![turn_started(4), interrupted(4), turn_started(5)];
        let supplied = vec![
            AgentSessionEvent::QueuePaused { at: 8.0 },
            interrupted(4),
            stop_resolution(4),
        ];

        assert_eq!(
            complete_terminal_projection_events(&previous, &supplied),
            vec![stop_resolution(4)]
        );
    }

    #[test]
    fn failed_terminal_adds_pause_only_when_latest_projection_is_unpaused() {
        let terminal = AgentSessionEvent::TurnCompleted {
            turn_id: 4,
            exit_code: 1,
            stop_reason: None,
            token_usage: None,
        };
        let supplied = vec![terminal.clone(), AgentSessionEvent::QueuePaused { at: 9.0 }];

        assert_eq!(
            complete_terminal_projection_events(&[turn_started(4)], &supplied),
            supplied
        );
        assert_eq!(
            complete_terminal_projection_events(
                &[turn_started(4), AgentSessionEvent::QueuePaused { at: 8.0 },],
                &supplied,
            ),
            vec![terminal]
        );
    }

    #[test]
    fn permission_response_pending_replays_before_claim_but_effect_reserved_does_not() {
        let store = crate::test_support::build_session_store();
        let response = crate::domain::agent_session::entities::PermissionResponse {
            request_id: "permission-1".to_string(),
            decision: crate::domain::agent_session::entities::PermissionResponseDecision::Allow {
                updated_input: None,
                answers: None,
            },
        };
        let first = store
            .reserve_permission_response(
                Path::new("/unused"),
                "session-1",
                7,
                "permission-1",
                response.clone(),
            )
            .unwrap();
        let replay = store
            .reserve_permission_response(
                Path::new("/unused"),
                "session-1",
                7,
                "permission-1",
                response.clone(),
            )
            .unwrap();
        assert_eq!(replay, first);
        assert_eq!(
            store.load_permission_response_obligation(&first).unwrap(),
            Some(crate::domain::local_event::ObligationStateRecord::Pending)
        );

        store
            .claim_permission_response_effect("session-1", &first)
            .unwrap();
        let retry = store.reserve_permission_response(
            Path::new("/unused"),
            "session-1",
            7,
            "permission-1",
            response,
        );
        assert!(retry.unwrap_err().contains("requires reconciliation"));
    }

    fn rewrite_persisted_worktree_path(app_data_dir: &Path, session_id: &str, worktree_path: &str) {
        let meta_path = app_data_dir
            .join("sessions")
            .join(session_id)
            .join("meta.json");
        let mut meta: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&meta_path).unwrap()).unwrap();
        meta["worktreePath"] = serde_json::Value::String(worktree_path.to_string());
        std::fs::write(meta_path, serde_json::to_vec_pretty(&meta).unwrap()).unwrap();
    }

    #[test]
    fn worktree_session_queries_match_legacy_trailing_slash_without_prefix_collision() {
        let app_data_dir = tempfile::tempdir().unwrap();
        let writer = Arc::new(crate::test_support::build_session_store());
        let legacy = super::super::create_session_internal(
            &writer,
            app_data_dir.path(),
            "/repo/",
            Some("claude".to_string()),
        )
        .unwrap();
        let canonical = super::super::create_session_internal(
            &writer,
            app_data_dir.path(),
            "/repo",
            Some("claude".to_string()),
        )
        .unwrap();
        let other = super::super::create_session_internal(
            &writer,
            app_data_dir.path(),
            "/repository",
            Some("claude".to_string()),
        )
        .unwrap();

        // Simulate metadata written before worktree paths were normalized on save.
        rewrite_persisted_worktree_path(app_data_dir.path(), &legacy.id, "/repo/");
        drop(writer);

        let reader = crate::test_support::build_session_store();
        let expected = HashSet::from([legacy.id.clone(), canonical.id.clone()]);
        for query in ["/repo", "/repo/"] {
            let summaries = reader.list_sessions(app_data_dir.path(), query).unwrap();
            assert_eq!(
                ids(summaries.iter().map(|session| session.id.clone())),
                expected
            );
            assert!(
                summaries
                    .iter()
                    .all(|session| session.worktree_path == "/repo"),
                "read models must expose the normalized identity"
            );

            assert_eq!(
                ids(reader
                    .list_worktree_sessions(app_data_dir.path(), query)
                    .unwrap()
                    .into_iter()
                    .map(|session| session.id),),
                expected
            );
            assert_eq!(
                ids(reader
                    .list_worktree_sessions_full(app_data_dir.path(), query)
                    .unwrap()
                    .into_iter()
                    .map(|session| session.id),),
                expected
            );
        }

        assert_eq!(
            ids(reader
                .list_sessions(app_data_dir.path(), "/repository")
                .unwrap()
                .into_iter()
                .map(|session| session.id),),
            HashSet::from([other.id])
        );
    }

    #[test]
    fn published_lists_restore_recovery_snapshot_and_classification_after_restart() {
        let app_data_dir = tempfile::tempdir().unwrap();
        let writer = crate::test_support::build_session_store();
        let active = super::super::create_session_internal(
            &writer,
            app_data_dir.path(),
            "/repo",
            Some("codex".to_string()),
        )
        .unwrap();
        let closed = super::super::create_session_internal(
            &writer,
            app_data_dir.path(),
            "/repo",
            Some("codex".to_string()),
        )
        .unwrap();
        writer
            .set_session_state(app_data_dir.path(), &closed.id, SessionState::Closed)
            .unwrap();
        let workflow = super::super::build_new_session_with_id(
            "00000000-0000-4000-8000-000000000149".to_string(),
            "/repo",
            Some("codex".to_string()),
            crate::domain::agent_session::PermissionMode::Edit,
            None,
            false,
            true,
            Some(WorkflowNodeContextDto {
                execution_id: "recovery-workflow-execution".to_string(),
                node_execution_id: "recovery-workflow-node".to_string(),
                workflow_name: "Recovery workflow".to_string(),
                node_name: "Recover session".to_string(),
                attempt: 1,
                parent_node_name: None,
                parent_attempt: None,
                order: 0,
                startup_timeout_secs: None,
                startup_max_retries: None,
                stale_timeout_secs: None,
            }),
        );
        writer
            .save_full_session_for_restore(app_data_dir.path(), &workflow)
            .unwrap();

        for session_id in [&active.id, &closed.id, &workflow.id] {
            writer
                .begin_backend_session_recovery(
                    app_data_dir.path(),
                    session_id,
                    &format!("recovery-{session_id}"),
                    BackendSessionRecoveryReason::BackendSessionLost,
                )
                .unwrap();
        }

        assert_eq!(
            ids(writer
                .list_sessions(app_data_dir.path(), "/repo")
                .unwrap()
                .into_iter()
                .map(|session| session.id)),
            HashSet::from([active.id.clone(), workflow.id.clone()])
        );
        assert_eq!(
            ids(writer
                .list_closed_sessions(app_data_dir.path(), "/repo")
                .unwrap()
                .into_iter()
                .map(|session| session.id)),
            HashSet::from([closed.id.clone()])
        );
        drop(writer);

        let reopened = crate::test_support::build_session_store();
        assert_eq!(
            ids(reopened
                .list_sessions(app_data_dir.path(), "/repo")
                .unwrap()
                .into_iter()
                .map(|session| session.id)),
            HashSet::from([active.id.clone(), workflow.id.clone()])
        );
        assert_eq!(
            ids(reopened
                .list_closed_sessions(app_data_dir.path(), "/repo")
                .unwrap()
                .into_iter()
                .map(|session| session.id)),
            HashSet::from([closed.id.clone()])
        );
        for session_id in [&active.id, &closed.id, &workflow.id] {
            let recovery = TurnEventLog::from_events(
                reopened
                    .load_session_events(app_data_dir.path(), session_id)
                    .unwrap(),
            )
            .project()
            .backend_recovery;
            assert_eq!(
                recovery,
                Some(BackendSessionRecoveryProjection::Recovering {
                    recovery_id: format!("recovery-{session_id}"),
                    old_provider_session_generation: 0,
                    reason: BackendSessionRecoveryReason::BackendSessionLost,
                })
            );
        }
        assert_eq!(
            reopened
                .get_session_meta(app_data_dir.path(), &closed.id)
                .unwrap()
                .unwrap()
                .state,
            SessionState::Closed,
            "recovery publication must never reopen a closed session"
        );
        let published = reopened
            .list_sessions(app_data_dir.path(), "/repo")
            .unwrap();
        let workflow_after_restart = published
            .iter()
            .find(|session| session.id == workflow.id)
            .expect("workflow-owned recovery remains published under its owner");
        assert!(workflow_after_restart.workflow_node_session);
        let owner = workflow_after_restart
            .workflow_node_context
            .as_ref()
            .expect("workflow recovery owner context");
        assert_eq!(owner.execution_id, "recovery-workflow-execution");
        assert_eq!(owner.node_execution_id, "recovery-workflow-node");
        assert_eq!(
            ids(published.into_iter().map(|session| session.id)),
            HashSet::from([active.id, workflow.id])
        );
        assert_eq!(
            ids(reopened
                .list_closed_sessions(app_data_dir.path(), "/repo")
                .unwrap()
                .into_iter()
                .map(|session| session.id)),
            HashSet::from([closed.id])
        );
    }

    #[test]
    fn recovery_start_atomically_persists_publication_snapshot_with_recovering_projection() {
        let app_data_dir = tempfile::tempdir().unwrap();
        let local_store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                app_data_dir.path().to_path_buf(),
            ),
        )
        .unwrap();
        let installation_id = local_store.installation_id().to_string();
        let writer = crate::test_support::build_session_store();
        let repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository> =
            local_store.clone();
        writer.set_local_event_repository(
            repository,
            installation_id.clone(),
            Arc::new(
                crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
            ),
        );
        let session = super::super::create_session_internal(
            &writer,
            app_data_dir.path(),
            "/repo",
            Some("codex".to_string()),
        )
        .unwrap();

        local_store.fault_injector().arm_fail_before_begin();
        assert!(writer
            .begin_backend_session_recovery(
                app_data_dir.path(),
                &session.id,
                "atomic-recovery",
                BackendSessionRecoveryReason::BackendSessionLost,
            )
            .is_err());
        let unchanged = writer
            .read_session_projection(&session.id)
            .unwrap()
            .unwrap();
        assert!(unchanged.meta.recovery_publication_snapshot.is_none());
        assert!(unchanged.reducer_events.iter().all(|event| !matches!(
            event,
            AgentSessionEvent::BackendSessionRecoveryStarted { .. }
        )));

        writer
            .begin_backend_session_recovery(
                app_data_dir.path(),
                &session.id,
                "atomic-recovery",
                BackendSessionRecoveryReason::BackendSessionLost,
            )
            .unwrap();
        drop(writer);

        let reopened = crate::test_support::build_session_store();
        let repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository> =
            local_store;
        reopened.set_local_event_repository(
            repository,
            installation_id,
            Arc::new(
                crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
            ),
        );
        let projection = reopened
            .read_session_projection(&session.id)
            .unwrap()
            .unwrap();
        let snapshot = projection
            .meta
            .recovery_publication_snapshot
            .expect("recovery publication snapshot persisted with the start event");
        assert_eq!(snapshot.recovery_id, "atomic-recovery");
        assert_eq!(
            snapshot.classification.list,
            RecoveryPublicationList::SessionList
        );
        assert_eq!(snapshot.summary.state, SessionState::Active);
        assert_eq!(
            TurnEventLog::from_events(projection.reducer_events)
                .project()
                .backend_recovery,
            Some(BackendSessionRecoveryProjection::Recovering {
                recovery_id: "atomic-recovery".to_string(),
                old_provider_session_generation: 0,
                reason: BackendSessionRecoveryReason::BackendSessionLost,
            })
        );
        assert_eq!(
            ids(reopened
                .list_sessions(app_data_dir.path(), "/repo")
                .unwrap()
                .into_iter()
                .map(|summary| summary.id)),
            HashSet::from([session.id])
        );
    }

    #[test]
    fn recovery_start_atomically_loses_to_queue_pause_after_its_projection_read() {
        let app_data_dir = tempfile::tempdir().unwrap();
        let local_store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                app_data_dir.path().to_path_buf(),
            ),
        )
        .unwrap();
        let writer = Arc::new(crate::test_support::build_session_store());
        let repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository> =
            local_store.clone();
        writer.set_local_event_repository(
            repository,
            local_store.installation_id().to_string(),
            Arc::new(
                crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
            ),
        );
        let session = super::super::create_session_internal(
            &writer,
            app_data_dir.path(),
            "/repo",
            Some("codex".to_string()),
        )
        .unwrap();

        let first_recovery_commit = Arc::new(AtomicBool::new(true));
        let projection_read = Arc::new(Barrier::new(2));
        let release_recovery = Arc::new(Barrier::new(2));
        writer.set_atomic_event_commit_hook_for_test(Arc::new({
            let first_recovery_commit = first_recovery_commit.clone();
            let projection_read = projection_read.clone();
            let release_recovery = release_recovery.clone();
            move |operation_kind| {
                if operation_kind == crate::domain::local_event::CommitOperationKind::Recovery
                    && first_recovery_commit.swap(false, Ordering::SeqCst)
                {
                    projection_read.wait();
                    release_recovery.wait();
                }
                Ok(())
            }
        }));

        let recovery_writer = writer.clone();
        let recovery_data_dir = app_data_dir.path().to_path_buf();
        let recovery_session_id = session.id.clone();
        let recovery = std::thread::spawn(move || {
            recovery_writer.begin_backend_session_recovery(
                &recovery_data_dir,
                &recovery_session_id,
                "stop-race-recovery",
                BackendSessionRecoveryReason::BackendSessionLost,
            )
        });
        projection_read.wait();
        writer
            .append_session_events(
                app_data_dir.path(),
                &session.id,
                &[AgentSessionEvent::QueuePaused { at: 8.0 }],
            )
            .unwrap();
        release_recovery.wait();

        let outcome = recovery.join().unwrap().unwrap();
        assert!(matches!(
            outcome,
            BackendSessionRecoveryStartOutcome::SuppressedByQueuePause
        ));
        let projection = writer
            .read_session_projection(&session.id)
            .unwrap()
            .unwrap();
        assert_eq!(projection.queue_paused_at, Some(8.0));
        assert!(projection.reducer_events.iter().all(|event| !matches!(
            event,
            AgentSessionEvent::BackendSessionRecoveryStarted { .. }
        )));
        assert!(writer
            .canonical_obligation(&backend_recovery_obligation_id(
                &session.id,
                "stop-race-recovery"
            ))
            .unwrap()
            .is_none());
    }

    #[test]
    fn stop_acceptance_prepared_from_stale_revision_loses_to_recovery_start() {
        let app_data_dir = tempfile::tempdir().unwrap();
        let local_store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                app_data_dir.path().to_path_buf(),
            ),
        )
        .unwrap();
        let writer = crate::test_support::build_session_store();
        let repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository> =
            local_store.clone();
        writer.set_local_event_repository(
            repository,
            local_store.installation_id().to_string(),
            Arc::new(
                crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
            ),
        );
        let session = super::super::create_session_internal(
            &writer,
            app_data_dir.path(),
            "/repo",
            Some("codex".to_string()),
        )
        .unwrap();
        writer
            .append_session_events(app_data_dir.path(), &session.id, &[turn_started(1)])
            .unwrap();
        let expected_stop_revision = writer
            .read_session_projection(&session.id)
            .unwrap()
            .unwrap()
            .meta
            .state_revision;

        let recovery = writer
            .begin_backend_session_recovery(
                app_data_dir.path(),
                &session.id,
                "recovery-wins-before-stop-acceptance",
                BackendSessionRecoveryReason::BackendSessionLost,
            )
            .unwrap();
        assert!(matches!(
            recovery,
            BackendSessionRecoveryStartOutcome::Started(_)
        ));

        let stop_events = [
            AgentSessionEvent::StopOperationAccepted {
                operation_id: "stop-after-stale-snapshot".to_string(),
                target_turn_id: 1,
                at: 9.0,
            },
            AgentSessionEvent::TurnInterruptRequested {
                turn_id: 1,
                at: 9.0,
            },
            AgentSessionEvent::ObligationRecorded {
                obligation_id: "stop-interrupt:session:1".to_string(),
                kind: crate::domain::agent_session::events::ObligationKind::ProviderInterrupt,
                state: crate::domain::agent_session::events::ObligationState::EffectReserved,
                at: 9.0,
            },
            AgentSessionEvent::QueuePaused { at: 9.0 },
        ];
        assert!(writer
            .prepare_event_projection_mutations_if_current_revision(
                &session.id,
                expected_stop_revision,
                &stop_events,
            )
            .unwrap()
            .is_none());

        let projection = writer
            .read_session_projection(&session.id)
            .unwrap()
            .unwrap();
        assert!(projection.reducer_events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::BackendSessionRecoveryStarted { .. }
        )));
        assert!(projection.reducer_events.iter().all(|event| !matches!(
            event,
            AgentSessionEvent::StopOperationAccepted { .. }
                | AgentSessionEvent::TurnInterruptRequested { .. }
                | AgentSessionEvent::QueuePaused { .. }
        )));
    }

    #[test]
    fn next_turn_id_advances_past_every_durable_queue_reservation() {
        let app_data_dir = tempfile::tempdir().unwrap();
        let local_store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                app_data_dir.path().to_path_buf(),
            ),
        )
        .unwrap();
        let writer = crate::test_support::build_session_store();
        let repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository> =
            local_store.clone();
        writer.set_local_event_repository(
            repository,
            local_store.installation_id().to_string(),
            Arc::new(
                crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
            ),
        );
        let session = super::super::create_session_internal(
            &writer,
            app_data_dir.path(),
            "/repo",
            Some("codex".to_string()),
        )
        .unwrap();
        writer
            .append_session_events(app_data_dir.path(), &session.id, &[turn_started(1)])
            .unwrap();
        let mut projection = writer
            .read_session_projection(&session.id)
            .unwrap()
            .unwrap();
        projection.pending_send_queue.extend([
            CanonicalQueuedSend {
                queue_item_id: "queue-2".to_string(),
                human_message_id: "human-2".to_string(),
                reserved_turn_id: "2".to_string(),
                input_ref: "input-2".to_string(),
            },
            CanonicalQueuedSend {
                queue_item_id: "queue-4".to_string(),
                human_message_id: "human-4".to_string(),
                reserved_turn_id: "4".to_string(),
                input_ref: "input-4".to_string(),
            },
        ]);
        writer
            .commit_session_projection_snapshot(projection)
            .unwrap();

        assert_eq!(
            writer
                .next_turn_id(app_data_dir.path(), &session.id)
                .unwrap(),
            5
        );
    }

    #[test]
    fn legacy_active_turn_zero_is_idle_for_first_send_and_atomic_queue_claim() {
        let app_data_dir = tempfile::tempdir().unwrap();
        let local_store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                app_data_dir.path().to_path_buf(),
            ),
        )
        .unwrap();
        let writer = crate::test_support::build_session_store();
        let repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository> =
            local_store.clone();
        writer.set_local_event_repository(
            repository,
            local_store.installation_id().to_string(),
            Arc::new(
                crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
            ),
        );
        let session = super::super::create_session_internal(
            &writer,
            app_data_dir.path(),
            "/repo",
            Some("codex".to_string()),
        )
        .unwrap();

        let initial = writer.send_acceptance_allocation(&session.id).unwrap();
        assert_eq!(initial.next_turn_id, 1);
        assert!(!initial.has_active_turn);

        // Reproduce the projection written by older builds: creation marked
        // the session Active even though no TurnStarted boundary existed.
        let mut legacy = writer
            .read_session_projection(&session.id)
            .unwrap()
            .unwrap();
        legacy.meta.state = SessionState::Active;
        legacy.meta.last_turn_id = Some(0);
        legacy.reducer_events.clear();
        legacy.pending_send_queue.push(CanonicalQueuedSend {
            queue_item_id: "queue-1".to_string(),
            human_message_id: "human-1".to_string(),
            reserved_turn_id: "1".to_string(),
            input_ref: "input-1".to_string(),
        });
        writer.commit_session_projection_snapshot(legacy).unwrap();

        assert_eq!(
            writer
                .accepted_queue_start_readiness(app_data_dir.path(), &session.id)
                .unwrap(),
            Some(true)
        );
        writer
            .append_accepted_queued_turn_started_and_project_state(
                app_data_dir.path(),
                &session.id,
                "queue-1",
                turn_started(1),
            )
            .unwrap();

        let claimed = writer
            .read_session_projection(&session.id)
            .unwrap()
            .unwrap();
        assert_eq!(claimed.meta.state, SessionState::Active);
        assert_eq!(claimed.meta.last_turn_id, Some(1));
        assert!(claimed.pending_send_queue.is_empty());
        assert!(reducer_has_active_turn(&claimed.reducer_events));
        assert_eq!(
            writer
                .accepted_queue_start_readiness(app_data_dir.path(), &session.id)
                .unwrap(),
            Some(false)
        );
    }

    #[test]
    fn send_acceptance_rejects_an_allocation_from_an_older_queue_projection() {
        let app_data_dir = tempfile::tempdir().unwrap();
        let local_store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                app_data_dir.path().to_path_buf(),
            ),
        )
        .unwrap();
        let writer = crate::test_support::build_session_store();
        let repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository> =
            local_store.clone();
        writer.set_local_event_repository(
            repository,
            local_store.installation_id().to_string(),
            Arc::new(
                crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
            ),
        );
        let session = super::super::create_session_internal(
            &writer,
            app_data_dir.path(),
            "/repo",
            Some("codex".to_string()),
        )
        .unwrap();
        writer
            .append_session_events(app_data_dir.path(), &session.id, &[turn_started(1)])
            .unwrap();

        let stale = writer.send_acceptance_allocation(&session.id).unwrap();
        assert_eq!(stale.next_turn_id, 2);
        assert!(stale.has_active_turn);
        let mut projection = writer
            .read_session_projection(&session.id)
            .unwrap()
            .unwrap();
        projection.pending_send_queue.push(CanonicalQueuedSend {
            queue_item_id: "queue-winner".to_string(),
            human_message_id: "human-winner".to_string(),
            reserved_turn_id: "2".to_string(),
            input_ref: "input-winner".to_string(),
        });
        writer
            .commit_session_projection_snapshot(projection)
            .unwrap();
        assert!(writer
            .canonical_queue_contains_exact(
                &session.id,
                "queue-winner",
                "human-winner",
                "2",
                Some("input-winner"),
            )
            .unwrap());
        assert!(!writer
            .canonical_queue_contains_exact(
                &session.id,
                "queue-winner",
                "human-winner",
                "2",
                Some("different-input"),
            )
            .unwrap());

        let prompt = crate::domain::agent_session::events::PromptInput {
            content: "stale queued input".to_string(),
            ..Default::default()
        };
        let disposition = crate::domain::agent_session::events::SendDisposition::Queued {
            queue_item_id: "queue-stale".to_string(),
        };
        let error = writer
            .prepare_send_acceptance_mutations(SendAcceptanceProjectionInput {
                session_id: &session.id,
                initial_session: None,
                session_projection_guard: stale.session_projection_guard,
                human_message_id: "human-stale",
                prompt: &prompt,
                disposition: &disposition,
                reserved_turn_id: Some("2"),
                input_ref: "input-stale",
                events: &[],
            })
            .unwrap_err();
        assert!(error.contains("allocation projection changed"));

        let fresh = writer.send_acceptance_allocation(&session.id).unwrap();
        assert_eq!(fresh.next_turn_id, 3);
        assert!(fresh.has_active_turn);
        assert!(fresh.has_pending_queue);
        assert_ne!(
            fresh.session_projection_guard,
            stale.session_projection_guard
        );
    }

    #[tokio::test]
    async fn completed_recovery_after_failure_allows_immediate_and_queued_turn_cas() {
        let app_data_dir = tempfile::tempdir().unwrap();
        let local_store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                app_data_dir.path().to_path_buf(),
            ),
        )
        .unwrap();
        let writer = crate::test_support::build_session_store();
        let repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository> =
            local_store.clone();
        writer.set_local_event_repository(
            repository.clone(),
            local_store.installation_id().to_string(),
            Arc::new(
                crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
            ),
        );

        let immediate = super::super::create_session_internal(
            &writer,
            app_data_dir.path(),
            "/repo",
            Some("codex".to_string()),
        )
        .unwrap();
        let mut immediate_projection = writer
            .read_session_projection(&immediate.id)
            .unwrap()
            .unwrap();
        immediate_projection.reducer_events = completed_recovery_after_failure();
        writer
            .commit_session_projection_snapshot(immediate_projection)
            .unwrap();
        let allocation = writer.send_acceptance_allocation(&immediate.id).unwrap();
        let prompt = crate::domain::agent_session::events::PromptInput {
            content: "resend after recovery".to_string(),
            ..Default::default()
        };
        let disposition =
            crate::domain::agent_session::events::SendDisposition::StartedTurn {
                turn_id: "1".to_string(),
            };
        let mutations = writer
            .prepare_send_acceptance_mutations(SendAcceptanceProjectionInput {
                session_id: &immediate.id,
                initial_session: None,
                session_projection_guard: allocation.session_projection_guard,
                human_message_id: "human-1",
                prompt: &prompt,
                disposition: &disposition,
                reserved_turn_id: Some("1"),
                input_ref: "input-1",
                events: &[turn_started(1)],
            })
            .unwrap();
        assert!(!mutations.is_empty());
        writer
            .begin_backend_session_recovery(
                app_data_dir.path(),
                &immediate.id,
                "recovery-after-send-prepare",
                BackendSessionRecoveryReason::BackendSessionLost,
            )
            .unwrap();
        let conflict = repository
            .commit_batch(crate::domain::local_event::LocalAtomicBatch {
                commit_id: crate::domain::local_event::CommitIdentity::parse(
                    "send-recovery-snapshot-fence",
                )
                .unwrap(),
                idempotency: crate::domain::local_event::IdempotencyBinding {
                    installation_id: local_store.installation_id().to_string(),
                    operation_kind: crate::domain::local_event::CommitOperationKind::Projection,
                    idempotency_key: "send-recovery-snapshot-fence".to_string(),
                    payload_hash: [31; 32],
                },
                expected_heads: Vec::new(),
                events: Vec::new(),
                state_mutations: mutations,
            })
            .await
            .expect_err("recovery revision must fence a send prepared from an older snapshot");
        assert!(matches!(
            conflict,
            crate::domain::local_event::CommitBatchError::PayloadConflict
                | crate::domain::local_event::CommitBatchError::StreamHeadConflict { .. }
        ));

        let queued = super::super::create_session_internal(
            &writer,
            app_data_dir.path(),
            "/repo",
            Some("codex".to_string()),
        )
        .unwrap();
        let mut queued_projection = writer
            .read_session_projection(&queued.id)
            .unwrap()
            .unwrap();
        queued_projection.reducer_events = completed_recovery_after_failure();
        queued_projection
            .pending_send_queue
            .push(CanonicalQueuedSend {
                queue_item_id: "queue-1".to_string(),
                human_message_id: "human-1".to_string(),
                reserved_turn_id: "1".to_string(),
                input_ref: "input-1".to_string(),
            });
        writer
            .commit_session_projection_snapshot(queued_projection)
            .unwrap();
        writer
            .append_accepted_queued_turn_started_and_project_state(
                app_data_dir.path(),
                &queued.id,
                "queue-1",
                turn_started(1),
            )
            .unwrap();
        let started = writer
            .read_session_projection(&queued.id)
            .unwrap()
            .unwrap();
        assert!(started.pending_send_queue.is_empty());
        assert!(started.reducer_events.iter().any(
            |event| matches!(event, AgentSessionEvent::TurnStarted { turn_id: 1, .. })
        ));
    }

    #[tokio::test]
    async fn failed_recovery_publication_then_success_publication_allows_resend() {
        let app_data_dir = tempfile::tempdir().unwrap();
        let local_store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                app_data_dir.path().to_path_buf(),
            ),
        )
        .unwrap();
        let writer = Arc::new(crate::test_support::build_session_store());
        let repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository> =
            local_store.clone();
        writer.set_local_event_repository(
            repository.clone(),
            local_store.installation_id().to_string(),
            Arc::new(
                crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
            ),
        );
        let session = super::super::create_session_internal(
            &writer,
            app_data_dir.path(),
            "/repo",
            Some("codex".to_string()),
        )
        .unwrap();

        writer
            .begin_backend_session_recovery(
                app_data_dir.path(),
                &session.id,
                "recovery-a",
                BackendSessionRecoveryReason::BackendSessionLost,
            )
            .unwrap();
        let failed = writer
            .fail_backend_session_recovery(
                app_data_dir.path(),
                &session.id,
                "recovery-a",
                "first recovery failed",
            )
            .unwrap();
        let failure_publication = failed
            .pending_recovery_message
            .expect("failure publication is durable");
        publish_recovery_message(
            &writer,
            app_data_dir.path(),
            &session.id,
            &failure_publication,
        );

        writer
            .begin_backend_session_recovery(
                app_data_dir.path(),
                &session.id,
                "recovery-b",
                BackendSessionRecoveryReason::BackendSessionLost,
            )
            .unwrap();
        let completed = writer
            .complete_backend_session_recovery(
                app_data_dir.path(),
                &session.id,
                "recovery-b",
                0,
                "replacement-provider".to_string(),
            )
            .unwrap();
        let success_publication = completed
            .pending_recovery_message
            .expect("success publication is durable");
        publish_recovery_message(
            &writer,
            app_data_dir.path(),
            &session.id,
            &success_publication,
        );

        let lifecycle =
            crate::adaptor::gateway::agent_session::LocalAgentSessionLifecycleRepository::new(
                repository,
                writer.clone(),
            );
        let restored =
            crate::domain::agent_session::repository::AgentSessionLifecycleRepository::restore_session(
                &lifecycle,
                &session.id,
            )
            .await
            .unwrap();
        assert_eq!(
            restored.admit_send(),
            Ok(
                crate::domain::agent_session::aggregates::session::SendDispositionDecision::StartImmediately
            )
        );

        let allocation = writer.send_acceptance_allocation(&session.id).unwrap();
        let prompt = crate::domain::agent_session::events::PromptInput {
            content: "resend after published recovery".to_string(),
            ..Default::default()
        };
        let disposition =
            crate::domain::agent_session::events::SendDisposition::StartedTurn {
                turn_id: "1".to_string(),
            };
        let mutations = writer
            .prepare_send_acceptance_mutations(SendAcceptanceProjectionInput {
                session_id: &session.id,
                initial_session: None,
                session_projection_guard: allocation.session_projection_guard,
                human_message_id: "human-1",
                prompt: &prompt,
                disposition: &disposition,
                reserved_turn_id: Some("1"),
                input_ref: "input-1",
                events: &[turn_started(1)],
            })
            .unwrap();
        assert!(!mutations.is_empty());
    }

    #[test]
    fn accepted_queued_turn_can_commit_only_the_canonical_front_without_regression() {
        let app_data_dir = tempfile::tempdir().unwrap();
        let local_store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                app_data_dir.path().to_path_buf(),
            ),
        )
        .unwrap();
        let writer = crate::test_support::build_session_store();
        let repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository> =
            local_store.clone();
        writer.set_local_event_repository(
            repository,
            local_store.installation_id().to_string(),
            Arc::new(
                crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
            ),
        );
        let session = super::super::create_session_internal(
            &writer,
            app_data_dir.path(),
            "/repo",
            Some("codex".to_string()),
        )
        .unwrap();
        writer
            .append_session_events(
                app_data_dir.path(),
                &session.id,
                &[turn_started(1), interrupted(1)],
            )
            .unwrap();
        let mut projection = writer
            .read_session_projection(&session.id)
            .unwrap()
            .unwrap();
        projection.pending_send_queue.extend([
            CanonicalQueuedSend {
                queue_item_id: "queue-2".to_string(),
                human_message_id: "human-2".to_string(),
                reserved_turn_id: "2".to_string(),
                input_ref: "input-2".to_string(),
            },
            CanonicalQueuedSend {
                queue_item_id: "queue-3".to_string(),
                human_message_id: "human-3".to_string(),
                reserved_turn_id: "3".to_string(),
                input_ref: "input-3".to_string(),
            },
        ]);
        writer
            .commit_session_projection_snapshot(projection)
            .unwrap();

        assert!(writer
            .append_accepted_queued_turn_started_and_project_state(
                app_data_dir.path(),
                &session.id,
                "queue-3",
                turn_started(3),
            )
            .is_err());
        let unchanged = writer
            .read_session_projection(&session.id)
            .unwrap()
            .unwrap();
        assert_eq!(unchanged.meta.last_turn_id, Some(1));
        assert_eq!(
            unchanged
                .pending_send_queue
                .iter()
                .map(|entry| entry.queue_item_id.as_str())
                .collect::<Vec<_>>(),
            vec!["queue-2", "queue-3"]
        );

        writer
            .append_session_events(
                app_data_dir.path(),
                &session.id,
                &[AgentSessionEvent::QueuePaused { at: 8.0 }],
            )
            .unwrap();
        let paused = writer
            .read_session_projection(&session.id)
            .unwrap()
            .unwrap();
        assert_eq!(paused.queue_paused_at, Some(8.0));
        assert!(writer
            .append_accepted_queued_turn_started_and_project_state(
                app_data_dir.path(),
                &session.id,
                "queue-2",
                turn_started(2),
            )
            .is_err());
        let still_paused = writer
            .read_session_projection(&session.id)
            .unwrap()
            .unwrap();
        assert_eq!(still_paused.meta.state, paused.meta.state);
        assert_eq!(still_paused.queue_paused_at, paused.queue_paused_at);
        assert_eq!(still_paused.reducer_events, paused.reducer_events);
        assert_eq!(still_paused.pending_send_queue, paused.pending_send_queue);

        writer
            .append_session_events(
                app_data_dir.path(),
                &session.id,
                &[AgentSessionEvent::QueueResumed {
                    expected_paused_at: 8.0,
                    at: 9.0,
                }],
            )
            .unwrap();
        writer
            .append_accepted_queued_turn_started_and_project_state(
                app_data_dir.path(),
                &session.id,
                "queue-2",
                turn_started(2),
            )
            .unwrap();
        let after_front = writer
            .read_session_projection(&session.id)
            .unwrap()
            .unwrap();
        assert_eq!(after_front.meta.last_turn_id, Some(2));
        assert_eq!(after_front.pending_send_queue.len(), 1);
        assert_eq!(after_front.pending_send_queue[0].queue_item_id, "queue-3");

        assert!(writer
            .append_accepted_queued_turn_started_and_project_state(
                app_data_dir.path(),
                &session.id,
                "queue-3",
                turn_started(2),
            )
            .is_err());
        assert_eq!(
            writer
                .read_session_projection(&session.id)
                .unwrap()
                .unwrap()
                .meta
                .last_turn_id,
            Some(2)
        );

        writer
            .append_session_events(
                app_data_dir.path(),
                &session.id,
                &[
                    interrupted(2),
                    AgentSessionEvent::SessionClosed { at: 12.0 },
                ],
            )
            .unwrap();
        let closed = writer
            .read_session_projection(&session.id)
            .unwrap()
            .unwrap();
        assert_eq!(closed.meta.state, SessionState::Closed);
        assert!(writer
            .append_accepted_queued_turn_started_and_project_state(
                app_data_dir.path(),
                &session.id,
                "queue-3",
                turn_started(3),
            )
            .is_err());
        let still_closed = writer
            .read_session_projection(&session.id)
            .unwrap()
            .unwrap();
        assert_eq!(still_closed.meta.state, SessionState::Closed);
        assert_eq!(still_closed.reducer_events, closed.reducer_events);
        assert_eq!(still_closed.pending_send_queue, closed.pending_send_queue);
        assert!(still_closed
            .reducer_events
            .iter()
            .all(|event| !matches!(event, AgentSessionEvent::TurnStarted { turn_id: 3, .. })));
    }

    #[test]
    fn accepted_queued_turn_cannot_cross_canonical_backend_recovery() {
        let app_data_dir = tempfile::tempdir().unwrap();
        let local_store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                app_data_dir.path().to_path_buf(),
            ),
        )
        .unwrap();
        let writer = crate::test_support::build_session_store();
        let repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository> =
            local_store.clone();
        writer.set_local_event_repository(
            repository,
            local_store.installation_id().to_string(),
            Arc::new(
                crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
            ),
        );
        let session = super::super::create_session_internal(
            &writer,
            app_data_dir.path(),
            "/repo",
            Some("codex".to_string()),
        )
        .unwrap();
        let mut projection = writer
            .read_session_projection(&session.id)
            .unwrap()
            .unwrap();
        projection.pending_send_queue.push(CanonicalQueuedSend {
            queue_item_id: "queue-1".to_string(),
            human_message_id: "human-1".to_string(),
            reserved_turn_id: "1".to_string(),
            input_ref: "input-1".to_string(),
        });
        writer
            .commit_session_projection_snapshot(projection)
            .unwrap();
        let recovery = writer
            .begin_backend_session_recovery(
                app_data_dir.path(),
                &session.id,
                "recovery-wins-before-queued-start",
                BackendSessionRecoveryReason::BackendSessionLost,
            )
            .unwrap();
        assert!(matches!(
            recovery,
            BackendSessionRecoveryStartOutcome::Started(_)
        ));
        let recovering = writer
            .read_session_projection(&session.id)
            .unwrap()
            .unwrap();
        assert_eq!(recovering.meta.state, SessionState::Idle);

        assert!(writer
            .append_accepted_queued_turn_started_and_project_state(
                app_data_dir.path(),
                &session.id,
                "queue-1",
                turn_started(1),
            )
            .is_err());
        let unchanged = writer
            .read_session_projection(&session.id)
            .unwrap()
            .unwrap();
        assert_eq!(unchanged.meta.state, recovering.meta.state);
        assert_eq!(unchanged.reducer_events, recovering.reducer_events);
        assert_eq!(unchanged.pending_send_queue, recovering.pending_send_queue);
        assert!(unchanged
            .reducer_events
            .iter()
            .all(|event| !matches!(event, AgentSessionEvent::TurnStarted { turn_id: 1, .. })));
    }

    #[test]
    fn stale_provider_establishment_cannot_cross_canonical_backend_recovery() {
        let app_data_dir = tempfile::tempdir().unwrap();
        let local_store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                app_data_dir.path().to_path_buf(),
            ),
        )
        .unwrap();
        let writer = crate::test_support::build_session_store();
        let repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository> =
            local_store.clone();
        writer.set_local_event_repository(
            repository,
            local_store.installation_id().to_string(),
            Arc::new(
                crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
            ),
        );
        let session = super::super::create_session_internal(
            &writer,
            app_data_dir.path(),
            "/repo",
            Some("codex".to_string()),
        )
        .unwrap();
        let recovery_id = "recovery-wins-before-stale-provider-observation";

        let recovery = writer
            .begin_backend_session_recovery(
                app_data_dir.path(),
                &session.id,
                recovery_id,
                BackendSessionRecoveryReason::BackendSessionLost,
            )
            .unwrap();
        assert!(matches!(
            recovery,
            BackendSessionRecoveryStartOutcome::Started(_)
        ));
        let recovering = writer
            .get_session_meta(app_data_dir.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(recovering.provider_session_generation, 0);
        assert!(recovering.agent_session_id.is_none());
        assert!(recovering.provider_session_observation_id.is_none());

        let outcome = writer
            .record_backend_session_established(
                app_data_dir.path(),
                &session.id,
                0,
                "stale-normal-provider-observation",
                "stale-provider".to_string(),
                None,
            )
            .unwrap();
        assert!(matches!(
            outcome,
            ProviderSessionEstablishmentOutcome::Fenced
        ));
        let unchanged = writer
            .get_session_meta(app_data_dir.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(unchanged.provider_session_generation, 0);
        assert!(unchanged.agent_session_id.is_none());
        assert!(unchanged.provider_session_observation_id.is_none());

        let completed = writer
            .complete_backend_session_recovery(
                app_data_dir.path(),
                &session.id,
                recovery_id,
                0,
                "replacement-provider".to_string(),
            )
            .unwrap();
        assert_eq!(completed.provider_session_generation, 1);
        assert_eq!(
            completed.agent_session_id.as_deref(),
            Some("replacement-provider")
        );
        assert_eq!(
            completed.provider_session_observation_id,
            Some(backend_recovery_provider_observation_id(recovery_id))
        );
    }

    #[test]
    fn ordinary_context_restore_completion_cannot_cross_active_backend_recovery() {
        let app_data_dir = tempfile::tempdir().unwrap();
        let local_store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                app_data_dir.path().to_path_buf(),
            ),
        )
        .unwrap();
        let writer = Arc::new(crate::test_support::build_session_store());
        let repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository> =
            local_store.clone();
        writer.set_local_event_repository(
            repository,
            local_store.installation_id().to_string(),
            Arc::new(
                crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
            ),
        );
        let session = super::super::create_session_internal(
            &writer,
            app_data_dir.path(),
            "/repo",
            Some("codex".to_string()),
        )
        .unwrap();
        writer
            .append_session_events(app_data_dir.path(), &session.id, &[turn_started(1)])
            .unwrap();
        let established = writer
            .record_backend_session_established(
                app_data_dir.path(),
                &session.id,
                0,
                "ordinary-context-restore-provider",
                "ordinary-provider".to_string(),
                None,
            )
            .unwrap();
        assert!(matches!(
            established,
            ProviderSessionEstablishmentOutcome::Settled(_)
        ));

        let first_context_completion_commit = Arc::new(AtomicBool::new(true));
        let projection_read = Arc::new(Barrier::new(2));
        let release_context_completion = Arc::new(Barrier::new(2));
        writer.set_atomic_event_commit_hook_for_test(Arc::new({
            let first_context_completion_commit = first_context_completion_commit.clone();
            let projection_read = projection_read.clone();
            let release_context_completion = release_context_completion.clone();
            move |operation_kind| {
                if operation_kind == crate::domain::local_event::CommitOperationKind::Projection
                    && first_context_completion_commit.swap(false, Ordering::SeqCst)
                {
                    projection_read.wait();
                    release_context_completion.wait();
                }
                Ok(())
            }
        }));

        let context_writer = writer.clone();
        let context_data_dir = app_data_dir.path().to_path_buf();
        let context_session_id = session.id.clone();
        let context_completion = std::thread::spawn(move || {
            context_writer.complete_context_restore_after_start_if_current(
                &context_data_dir,
                &context_session_id,
                ContextRestoreCompletionRequest::after_started_turn(0, 1, true, false, false),
            )
        });
        projection_read.wait();

        let recovery_id = "recovery-wins-before-ordinary-context-completion";
        let recovery = writer.begin_backend_session_recovery(
            app_data_dir.path(),
            &session.id,
            recovery_id,
            BackendSessionRecoveryReason::BackendSessionLost,
        );
        release_context_completion.wait();
        let recovery = recovery.unwrap();
        assert!(matches!(
            recovery,
            BackendSessionRecoveryStartOutcome::Started(_)
        ));

        let outcome = context_completion.join().unwrap().unwrap();
        assert!(outcome.is_none());

        let projection = writer
            .read_session_projection(&session.id)
            .unwrap()
            .unwrap();
        assert_eq!(projection.meta.provider_session_generation, 1);
        assert!(projection.meta.agent_session_id.is_none());
        assert!(projection.meta.provider_session_observation_id.is_none());
        assert_eq!(projection.meta.context_reinjection_generation, None);
        assert_eq!(
            projection.meta.context_carry,
            Some(ContextCarryState::Failed)
        );
        assert_eq!(
            projection
                .meta
                .recovery_publication_snapshot
                .as_ref()
                .map(|snapshot| snapshot.recovery_id.as_str()),
            Some(recovery_id)
        );
        assert_eq!(
            projection
                .reducer_events
                .iter()
                .filter(|event| matches!(
                    event,
                    AgentSessionEvent::BackendSessionRecoveryStarted {
                        recovery_id: stored_recovery_id,
                        ..
                    } if stored_recovery_id == recovery_id
                ))
                .count(),
            1
        );
    }

    #[test]
    fn ordinary_context_restore_completion_cannot_cross_a_newer_turn() {
        let app_data_dir = tempfile::tempdir().unwrap();
        let local_store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                app_data_dir.path().to_path_buf(),
            ),
        )
        .unwrap();
        let writer = Arc::new(crate::test_support::build_session_store());
        let repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository> =
            local_store.clone();
        writer.set_local_event_repository(
            repository,
            local_store.installation_id().to_string(),
            Arc::new(
                crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
            ),
        );
        let session = super::super::create_session_internal(
            &writer,
            app_data_dir.path(),
            "/repo",
            Some("codex".to_string()),
        )
        .unwrap();
        writer
            .append_session_events(app_data_dir.path(), &session.id, &[turn_started(1)])
            .unwrap();

        let first_context_completion_commit = Arc::new(AtomicBool::new(true));
        let projection_read = Arc::new(Barrier::new(2));
        let release_context_completion = Arc::new(Barrier::new(2));
        writer.set_atomic_event_commit_hook_for_test(Arc::new({
            let first_context_completion_commit = first_context_completion_commit.clone();
            let projection_read = projection_read.clone();
            let release_context_completion = release_context_completion.clone();
            move |operation_kind| {
                if operation_kind == crate::domain::local_event::CommitOperationKind::Projection
                    && first_context_completion_commit.swap(false, Ordering::SeqCst)
                {
                    projection_read.wait();
                    release_context_completion.wait();
                }
                Ok(())
            }
        }));

        let context_writer = writer.clone();
        let context_data_dir = app_data_dir.path().to_path_buf();
        let context_session_id = session.id.clone();
        let context_completion = std::thread::spawn(move || {
            context_writer.complete_context_restore_after_start_if_current(
                &context_data_dir,
                &context_session_id,
                ContextRestoreCompletionRequest::after_started_turn(0, 1, true, false, false),
            )
        });
        projection_read.wait();
        writer
            .append_session_events(app_data_dir.path(), &session.id, &[turn_started(2)])
            .unwrap();
        release_context_completion.wait();

        assert!(context_completion.join().unwrap().unwrap().is_none());
        let projection = writer
            .read_session_projection(&session.id)
            .unwrap()
            .unwrap();
        assert_eq!(projection.meta.last_turn_id, Some(2));
        assert_eq!(projection.meta.context_carry, None);
    }

    #[test]
    fn stale_recovery_context_completion_cannot_clear_newer_generation_marker() {
        let app_data_dir = tempfile::tempdir().unwrap();
        let local_store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                app_data_dir.path().to_path_buf(),
            ),
        )
        .unwrap();
        let writer = crate::test_support::build_session_store();
        let repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository> =
            local_store.clone();
        writer.set_local_event_repository(
            repository,
            local_store.installation_id().to_string(),
            Arc::new(
                crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
            ),
        );
        let session = super::super::create_session_internal(
            &writer,
            app_data_dir.path(),
            "/repo",
            Some("codex".to_string()),
        )
        .unwrap();

        let first_recovery_id = "first-context-recovery-generation";
        let first_recovery = writer
            .begin_backend_session_recovery(
                app_data_dir.path(),
                &session.id,
                first_recovery_id,
                BackendSessionRecoveryReason::BackendSessionLost,
            )
            .unwrap();
        assert!(matches!(
            first_recovery,
            BackendSessionRecoveryStartOutcome::Started(_)
        ));
        let first_completed = writer
            .complete_backend_session_recovery(
                app_data_dir.path(),
                &session.id,
                first_recovery_id,
                0,
                "first-recovery-provider".to_string(),
            )
            .unwrap();
        assert_eq!(first_completed.provider_session_generation, 1);
        assert_eq!(first_completed.context_reinjection_generation, Some(1));

        let second_recovery_id = "second-context-recovery-generation";
        let second_recovery = writer
            .begin_backend_session_recovery(
                app_data_dir.path(),
                &session.id,
                second_recovery_id,
                BackendSessionRecoveryReason::BackendSessionLost,
            )
            .unwrap();
        assert!(matches!(
            second_recovery,
            BackendSessionRecoveryStartOutcome::Started(_)
        ));
        let second_completed = writer
            .complete_backend_session_recovery(
                app_data_dir.path(),
                &session.id,
                second_recovery_id,
                1,
                "second-recovery-provider".to_string(),
            )
            .unwrap();
        assert_eq!(second_completed.provider_session_generation, 2);
        assert_eq!(second_completed.context_reinjection_generation, Some(2));

        let outcome = writer
            .complete_context_reinjection_if_required(app_data_dir.path(), &session.id, 1, true)
            .unwrap();
        assert!(outcome.is_none());

        let projection = writer
            .read_session_projection(&session.id)
            .unwrap()
            .unwrap();
        assert_eq!(projection.meta.provider_session_generation, 2);
        assert_eq!(
            projection.meta.agent_session_id.as_deref(),
            Some("second-recovery-provider")
        );
        assert_eq!(
            projection.meta.provider_session_observation_id,
            Some(backend_recovery_provider_observation_id(second_recovery_id))
        );
        assert_eq!(projection.meta.context_reinjection_generation, Some(2));
        assert_eq!(
            projection.meta.context_carry,
            Some(ContextCarryState::Failed)
        );
        assert!(projection.meta.recovery_publication_snapshot.is_none());
        assert!(matches!(
            projection.meta.pending_recovery_message,
            Some(PendingRecoveryMessage::Notice {
                ref recovery_id,
                ..
            }) if recovery_id == second_recovery_id
        ));
    }

    #[tokio::test]
    async fn prepared_stop_acceptance_loses_when_recovery_commits_before_stop() {
        let app_data_dir = tempfile::tempdir().unwrap();
        let local_store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                app_data_dir.path().to_path_buf(),
            ),
        )
        .unwrap();
        let writer = crate::test_support::build_session_store();
        let repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository> =
            local_store.clone();
        writer.set_local_event_repository(
            repository.clone(),
            local_store.installation_id().to_string(),
            Arc::new(
                crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
            ),
        );
        let session = super::super::create_session_internal(
            &writer,
            app_data_dir.path(),
            "/repo",
            Some("codex".to_string()),
        )
        .unwrap();
        writer
            .append_session_events(app_data_dir.path(), &session.id, &[turn_started(1)])
            .unwrap();
        let expected_stop_revision = writer
            .read_session_projection(&session.id)
            .unwrap()
            .unwrap()
            .meta
            .state_revision;
        let stop_events = [
            AgentSessionEvent::StopOperationAccepted {
                operation_id: "prepared-stop".to_string(),
                target_turn_id: 1,
                at: 9.0,
            },
            AgentSessionEvent::TurnInterruptRequested {
                turn_id: 1,
                at: 9.0,
            },
            AgentSessionEvent::QueuePaused { at: 9.0 },
        ];
        let stop_mutations = writer
            .prepare_event_projection_mutations_if_current_revision(
                &session.id,
                expected_stop_revision,
                &stop_events,
            )
            .unwrap()
            .expect("Stop preparation starts from the exact snapshot revision");

        let recovery = writer
            .begin_backend_session_recovery(
                app_data_dir.path(),
                &session.id,
                "recovery-commits-after-stop-preparation",
                BackendSessionRecoveryReason::BackendSessionLost,
            )
            .unwrap();
        assert!(matches!(
            recovery,
            BackendSessionRecoveryStartOutcome::Started(_)
        ));

        let error = repository
            .commit_batch(crate::domain::local_event::LocalAtomicBatch {
                commit_id: crate::domain::local_event::CommitIdentity::parse(
                    "prepared-stop-projection-cas",
                )
                .unwrap(),
                idempotency: crate::domain::local_event::IdempotencyBinding {
                    installation_id: local_store.installation_id().to_string(),
                    operation_kind: crate::domain::local_event::CommitOperationKind::Projection,
                    idempotency_key: "prepared-stop-projection-cas".to_string(),
                    payload_hash: [29; 32],
                },
                expected_heads: Vec::new(),
                events: Vec::new(),
                state_mutations: stop_mutations,
            })
            .await
            .expect_err("the Stop projection CAS must lose to the recovery commit");
        assert!(matches!(
            error,
            crate::domain::local_event::CommitBatchError::PayloadConflict
                | crate::domain::local_event::CommitBatchError::StreamHeadConflict { .. }
        ));
        let projection = writer
            .read_session_projection(&session.id)
            .unwrap()
            .unwrap();
        assert!(projection.reducer_events.iter().any(|event| matches!(
            event,
            AgentSessionEvent::BackendSessionRecoveryStarted { .. }
        )));
        assert!(projection.reducer_events.iter().all(|event| !matches!(
            event,
            AgentSessionEvent::StopOperationAccepted { .. }
                | AgentSessionEvent::TurnInterruptRequested { .. }
                | AgentSessionEvent::QueuePaused { .. }
        )));
    }

    #[test]
    fn recovering_publication_survives_sqlite_authority_restart_without_provider_resume() {
        let app_data_dir = tempfile::tempdir().unwrap();
        let local_store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                app_data_dir.path().to_path_buf(),
            ),
        )
        .unwrap();
        let installation_id = local_store.installation_id().to_string();
        let writer = crate::test_support::build_session_store();
        let repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository> =
            local_store.clone();
        writer.set_local_event_repository(
            repository,
            installation_id.clone(),
            Arc::new(
                crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
            ),
        );

        let active = super::super::create_session_internal(
            &writer,
            app_data_dir.path(),
            "/repo",
            Some("codex".to_string()),
        )
        .unwrap();
        let closed = super::super::create_session_internal(
            &writer,
            app_data_dir.path(),
            "/repo",
            Some("codex".to_string()),
        )
        .unwrap();
        let archived = super::super::create_session_internal(
            &writer,
            app_data_dir.path(),
            "/repo",
            Some("codex".to_string()),
        )
        .unwrap();
        let workflow = super::super::build_new_session_with_id(
            "00000000-0000-4000-8000-000000000150".to_string(),
            "/repo",
            Some("codex".to_string()),
            crate::domain::agent_session::PermissionMode::Edit,
            None,
            false,
            true,
            Some(WorkflowNodeContextDto {
                execution_id: "sqlite-restart-execution".to_string(),
                node_execution_id: "sqlite-restart-node".to_string(),
                workflow_name: "SQLite restart workflow".to_string(),
                node_name: "Recover session".to_string(),
                attempt: 1,
                parent_node_name: None,
                parent_attempt: None,
                order: 0,
                startup_timeout_secs: None,
                startup_max_retries: None,
                stale_timeout_secs: None,
            }),
        );
        writer
            .save_full_session_for_restore(app_data_dir.path(), &workflow)
            .unwrap();

        for session_id in [&active.id, &closed.id, &archived.id, &workflow.id] {
            writer
                .record_backend_session_established(
                    app_data_dir.path(),
                    session_id,
                    0,
                    &format!("provider-establishment-{session_id}"),
                    format!("provider-{session_id}"),
                    None,
                )
                .unwrap();
        }
        writer
            .set_session_state(app_data_dir.path(), &closed.id, SessionState::Closed)
            .unwrap();
        writer
            .set_session_state(app_data_dir.path(), &archived.id, SessionState::Archived)
            .unwrap();

        for session_id in [&active.id, &closed.id, &archived.id, &workflow.id] {
            writer
                .begin_backend_session_recovery(
                    app_data_dir.path(),
                    session_id,
                    &format!("restart-recovery-{session_id}"),
                    BackendSessionRecoveryReason::BackendSessionLost,
                )
                .unwrap();
        }

        let turn_lifecycle = super::super::create_session_internal(
            &writer,
            app_data_dir.path(),
            "/turn-lifecycle",
            Some("codex".to_string()),
        )
        .unwrap();
        writer
            .set_session_state(app_data_dir.path(), &turn_lifecycle.id, SessionState::Idle)
            .unwrap();
        writer
            .append_turn_started_and_project_state(
                app_data_dir.path(),
                &turn_lifecycle.id,
                turn_started(41),
            )
            .unwrap();
        assert_eq!(
            writer
                .get_session_meta(app_data_dir.path(), &turn_lifecycle.id)
                .unwrap()
                .unwrap()
                .state,
            SessionState::Active
        );
        writer
            .append_session_events(app_data_dir.path(), &turn_lifecycle.id, &[interrupted(41)])
            .unwrap();
        assert_eq!(
            writer
                .get_session_meta(app_data_dir.path(), &turn_lifecycle.id)
                .unwrap()
                .unwrap()
                .state,
            SessionState::Idle,
            "ordinary turn terminal projection remains reducer-owned"
        );

        // Release both the usecase-owned authority reference and the concrete
        // SQLite writer lock. The second open must rebuild every public read
        // from the permanent store rather than an in-memory projection.
        drop(writer);
        drop(local_store);

        let reopened_local_store =
            crate::adaptor::gateway::local_event_store::LocalEventStore::open(
                crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                    app_data_dir.path().to_path_buf(),
                ),
            )
            .unwrap();
        assert_eq!(reopened_local_store.installation_id(), installation_id);
        let reopened = crate::test_support::build_session_store();
        let repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository> =
            reopened_local_store;
        reopened.set_local_event_repository(
            repository,
            installation_id,
            Arc::new(
                crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
            ),
        );

        let published = reopened
            .list_sessions(app_data_dir.path(), "/repo")
            .unwrap();
        assert_eq!(
            ids(published
                .iter()
                .filter(|summary| !summary.workflow_node_session)
                .map(|summary| summary.id.clone())),
            HashSet::from([active.id.clone()])
        );
        assert_eq!(
            ids(published
                .iter()
                .filter(|summary| summary.workflow_node_session)
                .map(|summary| summary.id.clone())),
            HashSet::from([workflow.id.clone()])
        );
        let published_workflow = published
            .iter()
            .find(|summary| summary.id == workflow.id)
            .expect("workflow-owned session remains in the public workflow classification");
        let published_owner = published_workflow
            .workflow_node_context
            .as_ref()
            .expect("workflow owner survives SQLite reopen");
        assert_eq!(published_owner.execution_id, "sqlite-restart-execution");
        assert_eq!(published_owner.node_execution_id, "sqlite-restart-node");
        assert_eq!(
            ids(reopened
                .list_closed_sessions(app_data_dir.path(), "/repo")
                .unwrap()
                .into_iter()
                .map(|summary| summary.id)),
            HashSet::from([closed.id.clone()])
        );

        for (
            session_id,
            expected_current_state,
            expected_published_state,
            expected_list,
            expected_owner,
        ) in [
            (
                active.id.as_str(),
                SessionState::Idle,
                SessionState::Active,
                RecoveryPublicationList::SessionList,
                None,
            ),
            (
                closed.id.as_str(),
                SessionState::Closed,
                SessionState::Closed,
                RecoveryPublicationList::ClosedHistory,
                None,
            ),
            (
                archived.id.as_str(),
                SessionState::Archived,
                SessionState::Archived,
                RecoveryPublicationList::ArchivedHistory,
                None,
            ),
            (
                workflow.id.as_str(),
                SessionState::Idle,
                SessionState::Active,
                RecoveryPublicationList::SessionList,
                Some(("sqlite-restart-execution", "sqlite-restart-node")),
            ),
        ] {
            let projection = reopened
                .read_session_projection(session_id)
                .unwrap()
                .expect("session projection survives SQLite reopen");
            assert_eq!(projection.meta.state, expected_current_state);
            assert_eq!(projection.meta.agent_session_id, None);
            assert_eq!(projection.meta.provider_session_generation, 1);
            assert_eq!(projection.meta.context_reinjection_generation, None);
            assert_eq!(
                projection.meta.context_carry,
                Some(ContextCarryState::Failed)
            );

            let snapshot = projection
                .meta
                .recovery_publication_snapshot
                .as_ref()
                .expect("recovering projection retains its publication snapshot");
            let recovery_id = format!("restart-recovery-{session_id}");
            assert_eq!(snapshot.recovery_id, recovery_id);
            assert_eq!(snapshot.summary.id, session_id);
            assert_eq!(snapshot.summary.state, expected_published_state);
            let expected_provider_session_id = format!("provider-{session_id}");
            assert_eq!(
                snapshot.summary.agent_session_id.as_deref(),
                Some(expected_provider_session_id.as_str())
            );
            assert_eq!(snapshot.classification.list, expected_list);
            match (
                snapshot.classification.workflow_owner.as_ref(),
                expected_owner,
            ) {
                (None, None) => {}
                (Some(owner), Some((execution_id, node_execution_id))) => {
                    assert_eq!(owner.execution_id.as_deref(), Some(execution_id));
                    assert_eq!(owner.node_execution_id.as_deref(), Some(node_execution_id));
                }
                other => panic!("unexpected recovery publication owner: {other:?}"),
            }
            assert_eq!(
                TurnEventLog::from_events(projection.reducer_events.clone())
                    .project()
                    .backend_recovery,
                Some(BackendSessionRecoveryProjection::Recovering {
                    recovery_id: recovery_id.clone(),
                    old_provider_session_generation: 1,
                    reason: BackendSessionRecoveryReason::BackendSessionLost,
                })
            );

            let obligation = reopened
                .canonical_obligation(&backend_recovery_obligation_id(session_id, &recovery_id))
                .unwrap()
                .expect("recovery effect remains durably reserved");
            assert!(matches!(
                obligation.record,
                crate::domain::local_event::ObligationRecord::BackendSessionRecovery {
                    session_id: stored_session_id,
                    recovery_id: stored_recovery_id,
                    detail:
                        crate::domain::local_event::BackendSessionRecoveryObligationRecord::EffectReserved {
                            old_provider_session_generation: 1,
                            reason: BackendSessionRecoveryReason::BackendSessionLost,
                            ..
                        },
                    state: crate::domain::local_event::ObligationStateRecord::EffectReserved,
                } if stored_session_id == session_id && stored_recovery_id == recovery_id
            ));
        }

        let turn_projection = reopened
            .read_session_projection(&turn_lifecycle.id)
            .unwrap()
            .expect("ordinary turn projection survives SQLite reopen");
        assert_eq!(turn_projection.meta.state, SessionState::Idle);
        assert!(turn_projection.meta.recovery_publication_snapshot.is_none());
        assert_eq!(
            TurnEventLog::from_events(turn_projection.reducer_events)
                .project()
                .backend_recovery,
            None
        );
    }

    #[test]
    fn projection_reason_change_notifies_listener_when_state_stays_error() {
        let app_data_dir = tempfile::tempdir().unwrap();
        let store = crate::test_support::build_session_store();
        let session = super::super::create_session_internal(
            &store,
            app_data_dir.path(),
            "/repo",
            Some("codex".to_string()),
        )
        .unwrap();
        let notifications = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let notifications_for_listener = Arc::clone(&notifications);
        store.register_state_change_listener(Arc::new(move |_, _, state, _| {
            notifications_for_listener.lock().push(*state);
        }));

        store
            .append_error_episode_and_materialize(
                app_data_dir.path(),
                &session.id,
                ErrorEpisodeInput {
                    message_id: "fatal-1".to_string(),
                    reason: "first fatal".to_string(),
                    at: 1.0,
                },
            )
            .unwrap();
        notifications.lock().clear();
        store
            .append_error_episode_and_materialize(
                app_data_dir.path(),
                &session.id,
                ErrorEpisodeInput {
                    message_id: "fatal-2".to_string(),
                    reason: "latest fatal".to_string(),
                    at: 2.0,
                },
            )
            .unwrap();

        assert_eq!(*notifications.lock(), vec![SessionState::Error]);
        let meta = store
            .get_session_meta(app_data_dir.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(meta.error_reason.as_deref(), Some("latest fatal"));
    }

    #[test]
    fn fork_session_clears_parent_error_reason_from_disk_and_later_error_state() {
        let app_data_dir = tempfile::tempdir().unwrap();
        let store = crate::test_support::build_session_store();
        let parent = super::super::create_session_internal(
            &store,
            app_data_dir.path(),
            "/repo",
            Some("codex".to_string()),
        )
        .unwrap();
        store
            .append_error_episode_and_materialize(
                app_data_dir.path(),
                &parent.id,
                ErrorEpisodeInput {
                    message_id: "fatal-parent".to_string(),
                    reason: "parent fatal".to_string(),
                    at: 1.0,
                },
            )
            .unwrap();

        let fork = store.fork_session(app_data_dir.path(), &parent.id).unwrap();
        let cached_meta = store
            .get_session_meta(app_data_dir.path(), &fork.id)
            .unwrap()
            .unwrap();
        assert_eq!(cached_meta.state, SessionState::Idle);
        assert_eq!(cached_meta.error_reason, None);
        drop(store);

        let reloaded_store = crate::test_support::build_session_store();
        let disk_meta = reloaded_store
            .get_session_meta(app_data_dir.path(), &fork.id)
            .unwrap()
            .unwrap();
        assert_eq!(disk_meta.error_reason, None);

        reloaded_store
            .set_session_state(app_data_dir.path(), &fork.id, SessionState::Error)
            .unwrap();
        let errored = reloaded_store
            .get_session_shell(app_data_dir.path(), &fork.id)
            .unwrap()
            .unwrap();
        assert_eq!(errored.state, SessionState::Error);
        assert_eq!(errored.error_reason, None);
    }

    #[test]
    fn b060_application_shutdown_inventory_uses_canonical_session_ownership_and_state() {
        let app_data_dir = tempfile::tempdir().unwrap();
        let local_store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                app_data_dir.path().to_path_buf(),
            ),
        )
        .unwrap();

        let store = crate::test_support::build_session_store();
        let repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository> =
            local_store.clone();
        store.set_local_event_repository(
            repository,
            local_store.installation_id().to_string(),
            Arc::new(
                crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
            ),
        );

        let create = |identity: &str,
                      workflow_node_session: bool|
         -> crate::usecase::agent_session::session::ChatSession {
            super::super::create_session_internal_with_attributes(
                &store,
                app_data_dir.path(),
                &format!("/repo/{identity}"),
                Some("codex".to_string()),
                crate::domain::agent_session::PermissionMode::Edit,
                super::super::SessionCreationAttributes {
                    workflow_node_session,
                    ..Default::default()
                },
            )
            .unwrap()
        };

        let active = create("b060-active", false);
        let idle = create("b060-idle", false);
        let closed = create("b060-closed-recovery", false);
        let archived = create("b060-archived-recovery", false);
        let workflow_child = create("b060-workflow-child", true);
        store
            .set_session_state(app_data_dir.path(), &active.id, SessionState::Active)
            .unwrap();
        store
            .set_session_state(app_data_dir.path(), &closed.id, SessionState::Closed)
            .unwrap();
        store
            .set_session_state(app_data_dir.path(), &archived.id, SessionState::Archived)
            .unwrap();

        let mut inventory = store
            .application_shutdown_target_session_ids(app_data_dir.path())
            .unwrap();
        inventory.sort();
        let mut expected = vec![active.id.clone(), idle.id.clone()];
        expected.sort();
        assert_eq!(inventory, expected);
        assert!(!inventory.contains(&closed.id));
        assert!(!inventory.contains(&archived.id));
        assert!(!inventory.contains(&workflow_child.id));
    }
}
