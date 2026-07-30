use super::*;
use crate::domain::agent_session::entities::{
    InterruptReason, PermissionRequestBody, PermissionRequestStatus,
};
use crate::domain::agent_session::events::InterruptReason as EventInterruptReason;

fn restored(
    state: SessionState,
    turn: Option<Turn>,
    queue: Vec<QueueItem>,
    paused: bool,
    recovery_fact: RecoveryFact,
) -> Session {
    Session::restore(SessionRestore {
        id: "session-1".to_string(),
        revision: 7,
        state,
        has_messages: false,
        has_provider_session: false,
        current_turn: turn,
        last_terminal: None,
        queue: QueueState::restore(queue, paused),
        recovery_fact,
    })
    .unwrap()
}

fn streaming_turn(id: u64) -> Turn {
    Turn::start(id)
}

fn queue_item(id: &str) -> QueueItem {
    QueueItem {
        id: id.to_string(),
        operation_id: format!("operation-{id}"),
        reserved_turn_id: Some(format!("turn-{id}")),
        human_message_id: None,
    }
}

fn permission(id: &str) -> PermissionRequest {
    PermissionRequest {
        id: id.to_string(),
        tool_use_id: None,
        parent_tool_use_id: None,
        tool_name: "Read".to_string(),
        body: PermissionRequestBody::ToolApproval {
            input: crate::domain::agent_session::value_objects::JsonPayload::new_unchecked(
                "{}".to_string(),
            ),
        },
        title: None,
        display_name: None,
        description: None,
        decision_reason: None,
        status: PermissionRequestStatus::Pending,
    }
}

#[test]
fn workflow_turn_admission_uses_open_quiescent_and_recovery_together() {
    for state in [SessionState::Idle, SessionState::Done, SessionState::Error] {
        assert_eq!(
            restored(state, None, vec![], false, RecoveryFact::Resolved).admit_workflow_turn(),
            Ok(())
        );
    }
    for state in [SessionState::Closed, SessionState::Archived] {
        assert_eq!(
            restored(state, None, vec![], true, RecoveryFact::Resolved).admit_workflow_turn(),
            Err(TransitionRejection::SessionClosed)
        );
    }
    assert_eq!(
        restored(
            SessionState::Active,
            Some(streaming_turn(1)),
            vec![],
            false,
            RecoveryFact::Resolved,
        )
        .admit_workflow_turn(),
        Err(TransitionRejection::NotQuiescent)
    );
    assert_eq!(
        restored(
            SessionState::Idle,
            None,
            vec![],
            false,
            RecoveryFact::Unresolved,
        )
        .admit_workflow_turn(),
        Err(TransitionRejection::UnresolvedRecovery)
    );
}

#[test]
fn done_and_error_do_not_gate_send_without_current_bounded_facts() {
    for state in [SessionState::Done, SessionState::Error] {
        assert_eq!(
            restored(state, None, vec![], false, RecoveryFact::Resolved).admit_send(),
            Ok(SendDispositionDecision::StartImmediately)
        );
        assert_eq!(
            restored(
                state,
                None,
                vec![queue_item("1")],
                false,
                RecoveryFact::Resolved,
            )
            .admit_send(),
            Ok(SendDispositionDecision::Queue)
        );
    }
}

#[test]
fn observed_newer_turn_supersedes_stale_current_fact_but_not_the_queue() {
    let mut session = restored(
        SessionState::Active,
        Some(streaming_turn(1)),
        vec![],
        false,
        RecoveryFact::Resolved,
    );
    assert_eq!(
        session.apply_observed_turn_start(streaming_turn(2)),
        TransitionOutcome::Applied
    );
    assert_eq!(session.active_turn_id(), Some(2));

    let mut queued = restored(
        SessionState::Active,
        Some(streaming_turn(1)),
        vec![QueueItem {
            id: "queue-2".to_string(),
            operation_id: "operation-2".to_string(),
            reserved_turn_id: Some("2".to_string()),
            human_message_id: Some("human-2".to_string()),
        }],
        false,
        RecoveryFact::Resolved,
    );
    assert_eq!(
        queued.apply_observed_turn_start(streaming_turn(2)),
        TransitionOutcome::Rejected(TransitionRejection::QueueNotEmpty)
    );
}

#[test]
fn permission_response_is_fenced_to_current_turn_and_request() {
    let mut session = restored(
        SessionState::Active,
        Some(streaming_turn(3)),
        vec![],
        false,
        RecoveryFact::Resolved,
    );
    assert_eq!(
        session.request_permission(3, permission("permission-1")),
        TransitionOutcome::Applied
    );
    assert_eq!(
        session.admit_permission_response("permission-2"),
        Err(TransitionRejection::StaleTarget)
    );
    assert_eq!(session.admit_permission_response("permission-1"), Ok(3));
}

#[test]
fn terminal_application_is_current_idempotent_or_superseded() {
    let result = TurnResult::Interrupted {
        reason: InterruptReason::Abort,
        error: None,
    };
    let mut session = restored(
        SessionState::Active,
        Some(streaming_turn(4)),
        vec![queue_item("1")],
        false,
        RecoveryFact::Resolved,
    );
    assert_eq!(
        session.apply_terminal(3, result.clone()).application,
        TerminalApplication::Superseded
    );
    let applied = session.apply_terminal(4, result.clone());
    assert_eq!(applied.application, TerminalApplication::Current);
    assert!(applied.pause_queue);
    assert!(session.queue().is_paused());
    assert_eq!(
        session.apply_terminal(4, result).application,
        TerminalApplication::AlreadyApplied
    );
    assert_eq!(session.state(), SessionState::Idle);
}

#[test]
fn restore_rejects_impossible_current_state_combinations() {
    assert_eq!(
        Session::restore(SessionRestore {
            id: "session-1".to_string(),
            revision: 0,
            state: SessionState::Closed,
            has_messages: false,
            has_provider_session: false,
            current_turn: Some(streaming_turn(1)),
            last_terminal: None,
            queue: QueueState::restore(vec![], true),
            recovery_fact: RecoveryFact::Resolved,
        }),
        Err(SessionRestoreError::ClosedWithActiveTurn)
    );
}

#[test]
fn terminal_result_owns_the_canonical_session_projection() {
    let cases = [
        (
            TurnResult::Completed {
                stop_reason: None,
                token_usage: None,
            },
            SessionState::Done,
            false,
        ),
        (
            TurnResult::Failed {
                error: "provider crashed".to_string(),
                token_usage: None,
            },
            SessionState::Error,
            true,
        ),
        (
            TurnResult::Interrupted {
                reason: InterruptReason::SessionClosed,
                error: None,
            },
            SessionState::Idle,
            true,
        ),
    ];
    for (result, expected_state, expected_pause) in cases {
        let mut session = restored(
            SessionState::Active,
            Some(streaming_turn(8)),
            vec![queue_item("1")],
            false,
            RecoveryFact::Resolved,
        );
        let decision = session.apply_terminal(8, result);
        assert_eq!(decision.application, TerminalApplication::Current);
        assert_eq!(decision.pause_queue, expected_pause);
        assert_eq!(session.state(), expected_state);
        assert_eq!(session.queue().is_paused(), expected_pause);
    }
}

#[test]
fn terminal_outcome_owns_exit_and_interruption_classification() {
    assert_eq!(
        Session::terminal_outcome(&TurnResult::Interrupted {
            reason: InterruptReason::Timeout,
            error: None,
        }),
        TerminalOutcome {
            exit_code: 124,
            interrupted: true,
            pause_queue: true,
            session_state: SessionState::Error,
        }
    );
    assert_eq!(
        Session::terminal_outcome(&TurnResult::Completed {
            stop_reason: None,
            token_usage: None,
        }),
        TerminalOutcome {
            exit_code: 0,
            interrupted: false,
            pause_queue: false,
            session_state: SessionState::Done,
        }
    );
}

#[test]
fn lifecycle_and_queue_transitions_are_aggregate_owned() {
    let mut session = restored(
        SessionState::Done,
        None,
        vec![queue_item("1")],
        true,
        RecoveryFact::Resolved,
    );
    assert_eq!(session.resume_queue(), TransitionOutcome::Applied);
    assert_eq!(
        session.start_queue_head("other", streaming_turn(9)),
        TransitionOutcome::Rejected(TransitionRejection::StaleTarget)
    );
    assert_eq!(
        session.start_queue_head("1", streaming_turn(9)),
        TransitionOutcome::Applied
    );
    assert_eq!(session.state(), SessionState::Active);
    assert!(session
        .apply_lifecycle(SessionLifecycleCommand::Close)
        .is_ok());
    assert_eq!(session.state(), SessionState::Closed);
    assert!(session.queue().is_paused());
    assert_eq!(
        session.apply_lifecycle(SessionLifecycleCommand::Close),
        Err(TransitionRejection::InvalidLifecycle),
        "a second public close is rejected by the same lifecycle authority"
    );
    assert!(session
        .apply_lifecycle(SessionLifecycleCommand::ArchiveClosed)
        .is_ok());
    assert_eq!(session.state(), SessionState::Archived);
}

#[test]
fn queued_turn_start_validates_the_exact_head_and_recovery_fence() {
    let queued = QueueItem {
        id: "queue-1".into(),
        operation_id: "operation-1".into(),
        reserved_turn_id: Some("9".into()),
        human_message_id: Some("human-1".into()),
    };
    let mut session = restored(
        SessionState::Idle,
        None,
        vec![queued.clone()],
        false,
        RecoveryFact::Resolved,
    );
    assert_eq!(
        session.apply_queue_start("queue-1", "other", streaming_turn(9)),
        Err(QueueStartRejection::IdentityMismatch)
    );
    assert_eq!(
        session.apply_queue_start("queue-1", "human-1", streaming_turn(9)),
        Ok(QueueStartTransition {
            consumed_queue_item_id: "queue-1".into(),
        })
    );

    let mut recovering = restored(
        SessionState::Idle,
        None,
        vec![queued],
        false,
        RecoveryFact::Unresolved,
    );
    assert_eq!(
        recovering.apply_queue_start("queue-1", "human-1", streaming_turn(9)),
        Err(QueueStartRejection::Transition(
            TransitionRejection::UnresolvedRecovery
        ))
    );
}

#[test]
fn accepted_permission_result_projects_once_and_late_results_only_settle_the_operation() {
    let response = crate::domain::agent_session::entities::PermissionResponse {
        request_id: "permission-1".to_string(),
        decision: crate::domain::agent_session::entities::PermissionResponseDecision::Allow {
            updated_input: None,
            answers: None,
        },
    };
    let mut pending = restored(
        SessionState::Active,
        Some(Turn::restore(
            9,
            crate::domain::agent_session::value_objects::TurnPhase::WaitingPermission,
            Some(permission("permission-1")),
        )),
        Vec::new(),
        false,
        RecoveryFact::Resolved,
    );
    assert_eq!(
        pending.apply_accepted_permission_result(9, &response),
        PermissionEffectCompletion::ProjectResolution
    );
    assert_eq!(
        pending.apply_accepted_permission_result(9, &response),
        PermissionEffectCompletion::AlreadySettled
    );

    let mut terminal = restored(
        SessionState::Idle,
        None,
        Vec::new(),
        false,
        RecoveryFact::Resolved,
    );
    assert_eq!(
        terminal.apply_accepted_permission_result(9, &response),
        PermissionEffectCompletion::Superseded
    );
}

#[test]
fn lifecycle_command_returns_one_domain_owned_terminal_queue_and_effect_plan() {
    let mut active = restored(
        SessionState::Active,
        Some(streaming_turn(9)),
        Vec::new(),
        false,
        RecoveryFact::Resolved,
    );
    let transition = active
        .apply_lifecycle(SessionLifecycleCommand::ArchiveOpen)
        .unwrap();
    assert_eq!(
        transition.terminal,
        Some((
            9,
            TurnResult::Interrupted {
                reason: InterruptReason::SessionClosed,
                error: None,
            }
        ))
    );
    assert!(transition.requires_runtime_effect(true));
    assert_eq!(
        transition.lifecycle_events(12.0),
        vec![
            AgentSessionDomainEvent::TurnInterrupted {
                turn_id: 9,
                reason: EventInterruptReason::SessionClosed,
                exit_code: -1,
                error: None,
            },
            AgentSessionDomainEvent::SessionClosed { at: 12.0 },
            AgentSessionDomainEvent::QueuePaused { at: 12.0 },
        ]
    );
    assert_eq!(active.state(), SessionState::Archived);

    let mut closed = restored(
        SessionState::Closed,
        None,
        vec![queue_item("retained")],
        false,
        RecoveryFact::Resolved,
    );
    let transition = closed
        .apply_lifecycle(SessionLifecycleCommand::ArchiveClosed)
        .unwrap();
    assert!(transition.lifecycle_events(13.0).is_empty());
    assert!(!transition.requires_runtime_effect(true));
    assert!(!closed.queue().is_paused());
}

#[test]
fn lifecycle_revision_admission_and_transition_are_one_aggregate_operation() {
    let mut session = restored(
        SessionState::Idle,
        None,
        Vec::new(),
        false,
        RecoveryFact::Resolved,
    );
    assert_eq!(
        session.apply_lifecycle_at_revision(6, SessionLifecycleCommand::Close),
        Err(LifecycleCommandRejection::RevisionConflict {
            current_revision: 7,
        })
    );
    assert_eq!(session.state(), SessionState::Idle);
    assert!(session
        .apply_lifecycle_at_revision(7, SessionLifecycleCommand::Close)
        .is_ok());
    assert_eq!(session.state(), SessionState::Closed);
}

#[test]
fn send_admission_selects_and_applies_the_disposition_in_one_operation() {
    let mut session = restored(
        SessionState::Idle,
        None,
        Vec::new(),
        false,
        RecoveryFact::Resolved,
    );
    let transition = session
        .apply_send(SessionSendCommand {
            expected_session_id: "session-1".to_string(),
            workflow_turn: false,
            reserved_turn_id: None,
            disposition: SendDisposition::StartedTurn {
                turn_id: "8".to_string(),
            },
            human_message_id: "human-1".to_string(),
            input_ref: "input-1".to_string(),
        })
        .unwrap();
    assert_eq!(
        transition,
        SessionSendTransition {
            disposition: SendDisposition::StartedTurn {
                turn_id: "8".to_string(),
            },
            reserved_turn_id: None,
        }
    );
    assert_eq!(session.active_turn_id(), Some(8));

    let mut stale = restored(
        SessionState::Idle,
        None,
        Vec::new(),
        false,
        RecoveryFact::Resolved,
    );
    assert_eq!(
        stale.apply_send(SessionSendCommand {
            expected_session_id: "other".to_string(),
            workflow_turn: false,
            reserved_turn_id: None,
            disposition: SendDisposition::StartedTurn {
                turn_id: "8".to_string(),
            },
            human_message_id: "human-1".to_string(),
            input_ref: "input-1".to_string(),
        }),
        Err(SendCommandRejection::IdentityMismatch)
    );
    assert_eq!(stale.state(), SessionState::Idle);
}

#[test]
fn stop_revision_target_and_queue_pause_are_one_aggregate_operation() {
    let mut session = restored(
        SessionState::Active,
        Some(streaming_turn(9)),
        Vec::new(),
        false,
        RecoveryFact::Resolved,
    );
    assert_eq!(
        session.apply_stop(6, 9),
        Err(TransitionRejection::StaleTarget)
    );
    assert_eq!(
        session.apply_stop_command(7, "not-a-turn"),
        Err(StopCommandRejection::InvalidTurnIdentity)
    );
    assert_eq!(
        session.apply_stop(7, 8),
        Err(TransitionRejection::StaleTarget)
    );
    assert_eq!(
        session.apply_stop(7, 9),
        Ok(StopTransition {
            turn_id: 9,
            queue_was_paused: false,
        })
    );
    assert!(session.queue().is_paused());
}

#[test]
fn unresolved_recovery_blocks_admission_and_queue_resume() {
    let mut session = restored(
        SessionState::Error,
        None,
        vec![queue_item("1")],
        true,
        RecoveryFact::Unresolved,
    );
    assert_eq!(
        session.admit_send(),
        Err(TransitionRejection::UnresolvedRecovery)
    );
    assert_eq!(
        session.admit_backend_switch(),
        Err(TransitionRejection::QueueNotEmpty),
        "an accepted queue remains the first visible backend-switch conflict"
    );
    assert_eq!(
        session.resume_queue(),
        TransitionOutcome::Rejected(TransitionRejection::UnresolvedRecovery)
    );

    let recovery_only = restored(
        SessionState::Error,
        None,
        Vec::new(),
        false,
        RecoveryFact::Unresolved,
    );
    assert_eq!(
        recovery_only.admit_backend_switch(),
        Err(TransitionRejection::UnresolvedRecovery)
    );
}

#[test]
fn backend_switch_uses_current_bounded_work_not_historical_facts() {
    let mut has_messages = restored(
        SessionState::Idle,
        None,
        Vec::new(),
        false,
        RecoveryFact::Resolved,
    );
    has_messages.has_messages = true;
    assert_eq!(has_messages.admit_backend_switch(), Ok(()));

    let mut has_provider_session = restored(
        SessionState::Idle,
        None,
        Vec::new(),
        false,
        RecoveryFact::Resolved,
    );
    has_provider_session.has_provider_session = true;
    assert_eq!(has_provider_session.admit_backend_switch(), Ok(()));
    assert_eq!(
        has_provider_session.admit_backend_selection_change(),
        Err(TransitionRejection::NotQuiescent)
    );
    assert_eq!(
        has_messages.admit_backend_selection_change(),
        Err(TransitionRejection::NotQuiescent)
    );
}

#[test]
fn terminal_event_convergence_rejects_late_and_duplicate_candidates() {
    let previous = vec![
        AgentSessionDomainEvent::TurnStarted {
            turn_id: 7,
            message_id: "human".into(),
            assistant_message_id: Some("agent".into()),
            prompt: Default::default(),
            at: 1.0,
        },
        AgentSessionDomainEvent::TurnCompleted {
            turn_id: 7,
            exit_code: 0,
            stop_reason: None,
            token_usage: None,
        },
        AgentSessionDomainEvent::TurnStarted {
            turn_id: 8,
            message_id: "human-2".into(),
            assistant_message_id: Some("agent-2".into()),
            prompt: Default::default(),
            at: 2.0,
        },
    ];
    let duplicate = vec![AgentSessionDomainEvent::TurnCompleted {
        turn_id: 7,
        exit_code: 0,
        stop_reason: None,
        token_usage: None,
    }];
    assert_eq!(
        Session::decide_terminal_events(&previous, &duplicate),
        TerminalEventApplication::AlreadyApplied { turn_id: 7 }
    );
    let late = vec![AgentSessionDomainEvent::TurnInterrupted {
        turn_id: 6,
        reason: crate::domain::agent_session::events::InterruptReason::Crash,
        exit_code: 1,
        error: Some("late".into()),
    }];
    assert_eq!(
        Session::decide_terminal_events(&previous, &late),
        TerminalEventApplication::Superseded { turn_id: 6 }
    );
}

#[test]
fn canonical_active_turn_requires_event_and_projection_identity_to_agree() {
    let events = vec![AgentSessionDomainEvent::TurnStarted {
        turn_id: 7,
        message_id: "human".into(),
        assistant_message_id: Some("agent".into()),
        prompt: Default::default(),
        at: 1.0,
    }];
    assert!(Session::canonical_active_turn_matches(&events, Some(7), 7));
    assert!(!Session::canonical_active_turn_matches(&events, Some(6), 7));
    assert!(!Session::canonical_active_turn_matches(&events, Some(7), 6));
}

#[test]
fn terminal_event_convergence_completes_current_interruption_inside_the_aggregate() {
    let previous = vec![AgentSessionDomainEvent::TurnStarted {
        turn_id: 7,
        message_id: "human".into(),
        assistant_message_id: Some("agent".into()),
        prompt: Default::default(),
        at: 1.0,
    }];
    let supplied = vec![AgentSessionDomainEvent::TurnInterrupted {
        turn_id: 7,
        reason: EventInterruptReason::Crash,
        exit_code: 1,
        error: Some("crash".into()),
    }];
    let completed =
        Session::converge_terminal_events(&previous, &supplied, |events, message_id| {
            assert_eq!(events, previous.as_slice());
            assert_eq!(message_id, "agent");
            vec![crate::domain::agent_session::entities::MessagePart::Text {
                content: "partial".into(),
                parent_tool_use_id: None,
            }]
        });

    assert!(matches!(
        completed.as_slice(),
        [
            AgentSessionDomainEvent::FinalPartsRecorded {
                turn_id: 7,
                message_id,
                ..
            },
            AgentSessionDomainEvent::TurnInterrupted { turn_id: 7, .. }
        ] if message_id == "agent"
    ));
}

#[test]
fn current_turn_restore_owns_permission_phase_reduction() {
    let request = permission("permission-1");
    let mut events = vec![
        AgentSessionDomainEvent::TurnStarted {
            turn_id: 9,
            message_id: "human".into(),
            assistant_message_id: None,
            prompt: Default::default(),
            at: 1.0,
        },
        AgentSessionDomainEvent::PermissionRequested {
            turn_id: 9,
            tool_use_id: None,
            request: request.clone(),
        },
    ];
    assert_eq!(
        Session::current_turn_from_events(&events, false)
            .expect("current turn")
            .phase(),
        TurnPhase::WaitingPermission
    );
    events.push(AgentSessionDomainEvent::PermissionResolved {
        turn_id: 9,
        tool_use_id: None,
        request_id: Some(request.id),
        decision: crate::domain::agent_session::events::PermissionDecision::Allowed,
        answers: None,
    });
    assert_eq!(
        Session::current_turn_from_events(&events, false)
            .expect("current turn")
            .phase(),
        TurnPhase::Streaming
    );
}

#[test]
fn lifecycle_projection_is_reduced_by_the_aggregate() {
    let events = vec![
        AgentSessionDomainEvent::TurnStarted {
            turn_id: 11,
            message_id: "human".into(),
            assistant_message_id: None,
            prompt: Default::default(),
            at: 1.0,
        },
        AgentSessionDomainEvent::TurnInterrupted {
            turn_id: 11,
            reason: crate::domain::agent_session::events::InterruptReason::Crash,
            exit_code: 1,
            error: Some("crash".into()),
        },
    ];
    assert_eq!(
        Session::project_lifecycle(&events),
        SessionLifecycleProjection {
            state: SessionState::Error,
            turn_phase: TurnPhase::Idle,
        }
    );
}

#[test]
fn interrupt_terminal_settles_tools_and_permissions_once() {
    let request = permission("permission-1");
    let mut events = vec![
        AgentSessionDomainEvent::TurnStarted {
            turn_id: 12,
            message_id: "human".into(),
            assistant_message_id: None,
            prompt: Default::default(),
            at: 1.0,
        },
        AgentSessionDomainEvent::ToolCallStarted {
            turn_id: 12,
            tool_use_id: "tool-1".into(),
            tool: "Read".into(),
            input: crate::domain::agent_session::value_objects::JsonPayload::new_unchecked(
                "{}".into(),
            ),
            parent_tool_use_id: None,
        },
        AgentSessionDomainEvent::PermissionRequested {
            turn_id: 12,
            tool_use_id: Some("tool-1".into()),
            request,
        },
    ];
    Session::finalize_interrupted_turn(
        &mut events,
        12,
        crate::domain::agent_session::events::InterruptReason::Crash,
        Some("provider crashed".into()),
        1,
    );
    let first_len = events.len();
    assert!(events.iter().any(|event| matches!(
        event,
        AgentSessionDomainEvent::ToolCallFailed { tool_use_id, .. } if tool_use_id == "tool-1"
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        AgentSessionDomainEvent::PermissionResolved {
            decision: crate::domain::agent_session::events::PermissionDecision::Cancelled,
            ..
        }
    )));
    Session::finalize_interrupted_turn(
        &mut events,
        12,
        crate::domain::agent_session::events::InterruptReason::Crash,
        Some("provider crashed".into()),
        1,
    );
    assert_eq!(events.len(), first_len);
}

#[test]
fn bounded_reducer_retains_only_session_latches_before_current_turn() {
    let previous = vec![
        AgentSessionDomainEvent::QueuePaused { at: 1.0 },
        AgentSessionDomainEvent::TurnStarted {
            turn_id: 1,
            message_id: "human-1".into(),
            assistant_message_id: None,
            prompt: Default::default(),
            at: 1.0,
        },
        AgentSessionDomainEvent::TurnCompleted {
            turn_id: 1,
            exit_code: 0,
            stop_reason: None,
            token_usage: None,
        },
    ];
    let appended = vec![AgentSessionDomainEvent::TurnStarted {
        turn_id: 2,
        message_id: "human-2".into(),
        assistant_message_id: None,
        prompt: Default::default(),
        at: 2.0,
    }];
    let bounded = Session::bounded_reducer_events(previous, &appended);
    assert_eq!(bounded.len(), 2);
    assert!(matches!(
        bounded[0],
        AgentSessionDomainEvent::QueuePaused { .. }
    ));
    assert!(matches!(
        bounded[1],
        AgentSessionDomainEvent::TurnStarted { turn_id: 2, .. }
    ));
}

#[test]
fn durable_terminal_admission_and_workflow_handoff_are_domain_owned() {
    let previous = vec![AgentSessionDomainEvent::TurnStarted {
        turn_id: 7,
        message_id: "human".into(),
        assistant_message_id: Some("agent".into()),
        prompt: Default::default(),
        at: 1.0,
    }];
    let terminal = vec![AgentSessionDomainEvent::TurnCompleted {
        turn_id: 7,
        exit_code: 0,
        stop_reason: None,
        token_usage: None,
    }];
    assert!(Session::terminal_commit_is_current(
        &previous, &terminal, false
    ));
    assert!(!Session::terminal_commit_is_current(
        &previous, &terminal, true
    ));

    let stale = vec![AgentSessionDomainEvent::TurnCompleted {
        turn_id: 6,
        exit_code: 0,
        stop_reason: None,
        token_usage: None,
    }];
    assert!(!Session::terminal_commit_is_current(
        &previous, &stale, false
    ));

    assert!(!Session::requires_workflow_turn_completion(true, 0, false));
    assert!(Session::requires_workflow_turn_completion(true, 1, false));
    assert!(Session::requires_workflow_turn_completion(true, 0, true));
    assert!(Session::requires_workflow_turn_completion(false, 0, false));
}

#[test]
fn restart_recovery_reconciles_projection_before_settling_idle() {
    assert_eq!(
        Session::decide_restart_recovery(SessionState::Active, SessionState::Error),
        SessionRestartDecision {
            reconcile_projection: Some(SessionState::Error),
            settled_state: SessionState::Idle,
        }
    );
    assert_eq!(
        Session::decide_restart_recovery(SessionState::Done, SessionState::Done),
        SessionRestartDecision {
            reconcile_projection: None,
            settled_state: SessionState::Idle,
        }
    );
}
