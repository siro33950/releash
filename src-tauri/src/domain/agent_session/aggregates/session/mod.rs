//! Agent-session lifecycle aggregate.
//!
//! Only bounded facts needed for admission and transitions are retained here.
//! Message bodies, turn history, operations, and obligations stay with their
//! existing authorities.

use std::collections::VecDeque;

use crate::domain::agent_session::entities::{
    InterruptReason, PermissionRequest, PermissionResponse, Turn, TurnResult,
};
use crate::domain::agent_session::events::{AgentSessionDomainEvent, SendDisposition};
use crate::domain::agent_session::value_objects::{SessionState, TurnPhase};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryFact {
    Resolved,
    Unresolved,
}

impl RecoveryFact {
    pub fn is_unresolved(self) -> bool {
        self == Self::Unresolved
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueItem {
    pub id: String,
    pub operation_id: String,
    pub reserved_turn_id: Option<String>,
    pub human_message_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueState {
    items: VecDeque<QueueItem>,
    paused: bool,
}

impl QueueState {
    pub fn restore(items: Vec<QueueItem>, paused: bool) -> Self {
        Self {
            items: items.into(),
            paused,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    fn enqueue(&mut self, item: QueueItem) -> TransitionOutcome {
        if self.items.iter().any(|current| current.id == item.id) {
            return TransitionOutcome::AlreadyApplied;
        }
        self.items.push_back(item);
        TransitionOutcome::Applied
    }

    fn pause(&mut self) -> TransitionOutcome {
        if self.paused {
            return TransitionOutcome::AlreadyApplied;
        }
        self.paused = true;
        TransitionOutcome::Applied
    }

    fn resume(&mut self) -> TransitionOutcome {
        if !self.paused {
            return TransitionOutcome::AlreadyApplied;
        }
        self.paused = false;
        TransitionOutcome::Applied
    }

    fn pop_head(&mut self, expected_id: &str) -> TransitionOutcome {
        let Some(head) = self.items.front() else {
            return TransitionOutcome::NotApplicable;
        };
        if head.id != expected_id {
            return TransitionOutcome::Rejected(TransitionRejection::StaleTarget);
        }
        self.items.pop_front();
        TransitionOutcome::Applied
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRestore {
    pub id: String,
    pub revision: u64,
    pub state: SessionState,
    pub has_messages: bool,
    pub has_provider_session: bool,
    pub current_turn: Option<Turn>,
    pub last_terminal: Option<TurnResult>,
    pub queue: QueueState,
    pub recovery_fact: RecoveryFact,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionRestoreError {
    EmptyIdentity,
    IdlePhaseWithCurrentTurn,
    PermissionWithoutWaitingTurn,
    ClosedWithActiveTurn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionOutcome {
    Applied,
    AlreadyApplied,
    NotApplicable,
    Rejected(TransitionRejection),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionEffectCompletion {
    ProjectResolution,
    AlreadySettled,
    Superseded,
    Rejected(TransitionRejection),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionRejection {
    SessionClosed,
    NotQuiescent,
    UnresolvedRecovery,
    NoActiveTurn,
    PermissionNotPending,
    QueuePaused,
    QueueNotEmpty,
    StaleTarget,
    InvalidLifecycle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LifecycleCommandRejection {
    RevisionUnrepresentable,
    RevisionConflict { current_revision: i64 },
    Transition(TransitionRejection),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendCommandRejection {
    IdentityMismatch,
    TurnIdentityUnavailable,
    InvalidTurnIdentity,
    Transition(TransitionRejection),
    TransitionNotApplied,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionLifecycleCommand {
    Close,
    ArchiveOpen,
    ArchiveClosed,
    SwitchBackend { backend_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSendCommand {
    pub expected_session_id: String,
    pub workflow_turn: bool,
    pub reserved_turn_id: Option<String>,
    pub disposition: SendDisposition,
    pub human_message_id: String,
    pub input_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSendTransition {
    pub disposition: SendDisposition,
    pub reserved_turn_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StopTransition {
    pub turn_id: u64,
    pub queue_was_paused: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopCommandRejection {
    InvalidTurnIdentity,
    Transition(TransitionRejection),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueStartRejection {
    InvalidReservedTurnIdentity,
    IdentityMismatch,
    Transition(TransitionRejection),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueStartTransition {
    pub consumed_queue_item_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionLifecycleTransition {
    pub terminal: Option<(u64, TurnResult)>,
    pub queue_was_paused: bool,
    pub pauses_queue: bool,
    pub emits_session_closed: bool,
    pub backend_id: Option<String>,
    runtime_effect_if_present: bool,
}

impl SessionLifecycleTransition {
    pub fn requires_runtime_effect(&self, has_live_runtime: bool) -> bool {
        has_live_runtime && self.runtime_effect_if_present
    }

    pub fn lifecycle_events(&self, at: f64) -> Vec<AgentSessionDomainEvent> {
        let mut events = Vec::new();
        if let Some((turn_id, _)) = self.terminal.as_ref() {
            events.push(AgentSessionDomainEvent::TurnInterrupted {
                turn_id: *turn_id,
                reason: crate::domain::agent_session::events::InterruptReason::SessionClosed,
                exit_code: -1,
                error: None,
            });
        }
        if self.emits_session_closed {
            events.push(AgentSessionDomainEvent::SessionClosed { at });
        }
        if self.pauses_queue && !self.queue_was_paused {
            events.push(AgentSessionDomainEvent::QueuePaused { at });
        }
        events
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendDispositionDecision {
    StartImmediately,
    Queue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalApplication {
    Current,
    AlreadyApplied,
    Superseded,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalDecision {
    pub application: TerminalApplication,
    pub result: TurnResult,
    pub pause_queue: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalOutcome {
    pub exit_code: i64,
    pub interrupted: bool,
    pub pause_queue: bool,
    pub session_state: SessionState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalEventApplication {
    NoTerminal,
    Current { terminal_index: usize, turn_id: u64 },
    AlreadyApplied { turn_id: u64 },
    Superseded { turn_id: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionLifecycleProjection {
    pub state: SessionState,
    pub turn_phase: TurnPhase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionRestartDecision {
    pub reconcile_projection: Option<SessionState>,
    pub settled_state: SessionState,
}

#[derive(Debug, Clone)]
pub struct UnresolvedPermission {
    pub turn_id: u64,
    pub tool_use_id: Option<String>,
    pub request_id: Option<String>,
    pub request: PermissionRequest,
}

impl UnresolvedPermission {
    fn key(&self) -> (Option<String>, Option<String>) {
        (self.tool_use_id.clone(), self.request_id.clone())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    id: String,
    revision: u64,
    state: SessionState,
    has_messages: bool,
    has_provider_session: bool,
    current_turn: Option<Turn>,
    last_terminal: Option<TurnResult>,
    queue: QueueState,
    recovery_fact: RecoveryFact,
}

impl Session {
    pub fn decide_restart_recovery(
        stored_state: SessionState,
        projected_state: SessionState,
    ) -> SessionRestartDecision {
        SessionRestartDecision {
            reconcile_projection: (stored_state != projected_state).then_some(projected_state),
            settled_state: SessionState::Idle,
        }
    }

    pub fn terminal_outcome(result: &TurnResult) -> TerminalOutcome {
        match result {
            TurnResult::Completed { .. } => TerminalOutcome {
                exit_code: 0,
                interrupted: false,
                pause_queue: false,
                session_state: SessionState::Done,
            },
            TurnResult::Failed { .. } => TerminalOutcome {
                exit_code: 1,
                interrupted: false,
                pause_queue: true,
                session_state: SessionState::Error,
            },
            TurnResult::Interrupted { reason, .. } => {
                use crate::domain::agent_session::entities::InterruptReason;
                let (exit_code, session_state) = match reason {
                    InterruptReason::Abort => (0, SessionState::Idle),
                    InterruptReason::Timeout => (124, SessionState::Error),
                    InterruptReason::Crash => (1, SessionState::Error),
                    InterruptReason::SessionClosed => (0, SessionState::Idle),
                };
                TerminalOutcome {
                    exit_code,
                    interrupted: true,
                    pause_queue: true,
                    session_state,
                }
            }
        }
    }

    pub fn bounded_reducer_events(
        mut previous: Vec<AgentSessionDomainEvent>,
        appended: &[AgentSessionDomainEvent],
    ) -> Vec<AgentSessionDomainEvent> {
        previous.extend_from_slice(appended);
        let Some(turn_start) = previous
            .iter()
            .rposition(|event| matches!(event, AgentSessionDomainEvent::TurnStarted { .. }))
        else {
            return previous;
        };

        let mut retained = Vec::new();
        if let Some(event) = previous[..turn_start].iter().rev().find(|event| {
            matches!(
                event,
                AgentSessionDomainEvent::QueuePaused { .. }
                    | AgentSessionDomainEvent::QueueResumed { .. }
            )
        }) {
            retained.push(event.clone());
        }
        if let Some(recovery_start) = previous[..turn_start].iter().rposition(|event| {
            matches!(
                event,
                AgentSessionDomainEvent::BackendSessionRecoveryStarted { .. }
            )
        }) {
            retained.extend(
                previous[recovery_start..turn_start]
                    .iter()
                    .filter(|event| {
                        matches!(
                            event,
                            AgentSessionDomainEvent::BackendSessionRecoveryStarted { .. }
                                | AgentSessionDomainEvent::SessionConfigurationReactivated { .. }
                                | AgentSessionDomainEvent::SessionGoalReactivated { .. }
                                | AgentSessionDomainEvent::BackendSessionRecoveryCompleted { .. }
                                | AgentSessionDomainEvent::BackendSessionRecoveryFailed { .. }
                        )
                    })
                    .cloned(),
            );
        }
        if let Some(event) = previous[..turn_start]
            .iter()
            .rev()
            .find(|event| matches!(event, AgentSessionDomainEvent::SessionClosed { .. }))
        {
            retained.push(event.clone());
        }
        retained.extend_from_slice(&previous[turn_start..]);
        retained
    }

    pub fn finalize_interrupted_turn(
        events: &mut Vec<AgentSessionDomainEvent>,
        turn_id: u64,
        reason: crate::domain::agent_session::events::InterruptReason,
        error: Option<String>,
        exit_code: i64,
    ) {
        if Self::turn_has_terminal(events, turn_id) {
            return;
        }

        let interrupted_content = Self::interruption_content(reason, error.as_deref());
        for tool_use_id in Self::unfinished_tool_calls(events, turn_id) {
            events.push(AgentSessionDomainEvent::ToolCallFailed {
                turn_id,
                tool_use_id,
                content: interrupted_content.clone(),
                content_ref: None,
                summary: None,
            });
        }

        for permission in Self::unresolved_permissions_for_turn(events, turn_id) {
            events.push(AgentSessionDomainEvent::PermissionResolved {
                turn_id,
                tool_use_id: permission.tool_use_id,
                request_id: permission.request_id,
                decision: crate::domain::agent_session::events::PermissionDecision::Cancelled,
                answers: None,
            });
        }

        events.push(AgentSessionDomainEvent::TurnInterrupted {
            turn_id,
            reason,
            exit_code,
            error,
        });
    }

    pub fn has_turn_terminal(events: &[AgentSessionDomainEvent], turn_id: u64) -> bool {
        events.iter().any(|event| {
            matches!(
                event,
                AgentSessionDomainEvent::TurnCompleted {
                    turn_id: candidate,
                    ..
                } | AgentSessionDomainEvent::TurnInterrupted {
                    turn_id: candidate,
                    ..
                } if *candidate == turn_id
            )
        })
    }

    pub fn unresolved_permissions_for_turn(
        events: &[AgentSessionDomainEvent],
        turn_id: u64,
    ) -> Vec<UnresolvedPermission> {
        let mut requested = Vec::new();
        let mut resolved = std::collections::HashSet::new();
        for event in events {
            match event {
                AgentSessionDomainEvent::PermissionRequested {
                    turn_id: id,
                    tool_use_id,
                    request,
                } if *id == turn_id => requested.push(UnresolvedPermission {
                    turn_id: *id,
                    tool_use_id: tool_use_id.clone(),
                    request_id: (!request.id.is_empty()).then(|| request.id.clone()),
                    request: request.clone(),
                }),
                AgentSessionDomainEvent::PermissionResolved {
                    turn_id: id,
                    tool_use_id,
                    request_id,
                    ..
                } if *id == turn_id => {
                    resolved.insert((tool_use_id.clone(), request_id.clone()));
                }
                _ => {}
            }
        }
        requested
            .into_iter()
            .filter(|permission| {
                let key = permission.key();
                !resolved.contains(&key)
                    && !resolved.iter().any(|resolved_key| {
                        key.1.is_some() && key.1 == resolved_key.1
                            || key.0.is_some() && key.0 == resolved_key.0
                    })
            })
            .collect()
    }

    pub fn latest_unresolved_permission(
        events: &[AgentSessionDomainEvent],
    ) -> Option<UnresolvedPermission> {
        let turn_id = events.iter().rev().find_map(|event| match event {
            AgentSessionDomainEvent::TurnStarted { turn_id, .. } => Some(*turn_id),
            _ => None,
        })?;
        if Self::has_turn_terminal(events, turn_id) {
            return None;
        }
        Self::unresolved_permissions_for_turn(events, turn_id).pop()
    }

    fn turn_has_terminal(events: &[AgentSessionDomainEvent], turn_id: u64) -> bool {
        events.iter().any(|event| {
            matches!(
                event,
                AgentSessionDomainEvent::TurnCompleted { turn_id: id, .. }
                    | AgentSessionDomainEvent::TurnInterrupted { turn_id: id, .. }
                    if *id == turn_id
            )
        })
    }

    fn unfinished_tool_calls(events: &[AgentSessionDomainEvent], turn_id: u64) -> Vec<String> {
        let mut started = Vec::new();
        let mut finished = std::collections::HashSet::new();
        for event in events {
            match event {
                AgentSessionDomainEvent::ToolCallStarted {
                    turn_id: id,
                    tool_use_id,
                    ..
                } if *id == turn_id => started.push(tool_use_id.clone()),
                AgentSessionDomainEvent::ToolCallSucceeded {
                    turn_id: id,
                    tool_use_id,
                    ..
                }
                | AgentSessionDomainEvent::ToolCallFailed {
                    turn_id: id,
                    tool_use_id,
                    ..
                } if *id == turn_id => {
                    finished.insert(tool_use_id.clone());
                }
                _ => {}
            }
        }
        started
            .into_iter()
            .filter(|tool_use_id| !finished.contains(tool_use_id))
            .collect()
    }

    fn interruption_content(
        reason: crate::domain::agent_session::events::InterruptReason,
        error: Option<&str>,
    ) -> String {
        let detail = error
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| reason.label());
        if detail.contains("中断") || detail.contains("interrupted") {
            detail.to_string()
        } else {
            format!("{detail} により中断")
        }
    }

    pub fn project_lifecycle(events: &[AgentSessionDomainEvent]) -> SessionLifecycleProjection {
        if events.iter().any(|event| {
            matches!(
                event,
                AgentSessionDomainEvent::SessionLifecycleOperationAccepted {
                    kind: crate::domain::agent_session::events::SessionLifecycleKind::Archive,
                    ..
                }
            )
        }) {
            return SessionLifecycleProjection {
                state: SessionState::Archived,
                turn_phase: TurnPhase::Idle,
            };
        }
        if events
            .iter()
            .any(|event| matches!(event, AgentSessionDomainEvent::SessionClosed { .. }))
        {
            return SessionLifecycleProjection {
                state: SessionState::Closed,
                turn_phase: TurnPhase::Idle,
            };
        }
        let session_errored = events.iter().fold(false, |errored, event| match event {
            AgentSessionDomainEvent::TurnStarted { .. } => false,
            AgentSessionDomainEvent::SessionErrored { .. } => true,
            _ => errored,
        });
        if session_errored {
            return SessionLifecycleProjection {
                state: SessionState::Error,
                turn_phase: TurnPhase::Idle,
            };
        }
        let Some(turn_id) = events.iter().rev().find_map(|event| match event {
            AgentSessionDomainEvent::TurnStarted { turn_id, .. } => Some(*turn_id),
            _ => None,
        }) else {
            return SessionLifecycleProjection {
                state: SessionState::Idle,
                turn_phase: TurnPhase::Idle,
            };
        };

        if let Some(terminal) = events.iter().rev().find(|event| {
            matches!(
                event,
                AgentSessionDomainEvent::TurnCompleted { turn_id: id, .. }
                    | AgentSessionDomainEvent::TurnInterrupted { turn_id: id, .. }
                    if *id == turn_id
            )
        }) {
            let state = match terminal {
                AgentSessionDomainEvent::TurnCompleted { exit_code: 0, .. } => SessionState::Done,
                AgentSessionDomainEvent::TurnCompleted { .. } => SessionState::Error,
                AgentSessionDomainEvent::TurnInterrupted {
                    reason:
                        crate::domain::agent_session::events::InterruptReason::Abort
                        | crate::domain::agent_session::events::InterruptReason::SessionClosed,
                    ..
                } => SessionState::Idle,
                AgentSessionDomainEvent::TurnInterrupted { .. } => SessionState::Error,
                _ => unreachable!("terminal event predicate is exhaustive"),
            };
            return SessionLifecycleProjection {
                state,
                turn_phase: TurnPhase::Idle,
            };
        }

        type PermissionKey = (Option<String>, Option<String>);
        let mut requested = Vec::<PermissionKey>::new();
        let mut resolved = std::collections::HashSet::<PermissionKey>::new();
        for event in events {
            match event {
                AgentSessionDomainEvent::PermissionRequested {
                    turn_id: permission_turn_id,
                    tool_use_id,
                    request,
                } if *permission_turn_id == turn_id => {
                    requested.push((
                        tool_use_id.clone(),
                        (!request.id.is_empty()).then(|| request.id.clone()),
                    ));
                }
                AgentSessionDomainEvent::PermissionResolved {
                    turn_id: permission_turn_id,
                    tool_use_id,
                    request_id,
                    ..
                } if *permission_turn_id == turn_id => {
                    resolved.insert((tool_use_id.clone(), request_id.clone()));
                }
                _ => {}
            }
        }
        let has_pending_permission = requested.into_iter().any(|key| {
            !resolved.contains(&key)
                && !resolved.iter().any(|resolved_key| {
                    key.1.is_some() && key.1 == resolved_key.1
                        || key.0.is_some() && key.0 == resolved_key.0
                })
        });
        SessionLifecycleProjection {
            state: SessionState::Active,
            turn_phase: if has_pending_permission {
                TurnPhase::WaitingPermission
            } else {
                TurnPhase::Streaming
            },
        }
    }

    pub fn current_turn_from_events(
        events: &[AgentSessionDomainEvent],
        session_closed: bool,
    ) -> Option<Turn> {
        if session_closed {
            return None;
        }
        let (start_index, turn_id) =
            events
                .iter()
                .enumerate()
                .rev()
                .find_map(|(index, event)| match event {
                    AgentSessionDomainEvent::TurnStarted { turn_id, .. } => Some((index, *turn_id)),
                    _ => None,
                })?;
        let current_events = &events[start_index + 1..];
        if current_events.iter().any(|event| {
            matches!(
                event,
                AgentSessionDomainEvent::TurnCompleted {
                    turn_id: terminal_turn_id,
                    ..
                } | AgentSessionDomainEvent::TurnInterrupted {
                    turn_id: terminal_turn_id,
                    ..
                } if *terminal_turn_id == turn_id
            ) || matches!(event, AgentSessionDomainEvent::SessionClosed { .. })
        }) {
            return None;
        }

        let mut permission: Option<PermissionRequest> = None;
        for event in current_events {
            match event {
                AgentSessionDomainEvent::PermissionRequested {
                    turn_id: permission_turn_id,
                    request,
                    ..
                } if *permission_turn_id == turn_id => permission = Some(request.clone()),
                AgentSessionDomainEvent::PermissionResolved {
                    turn_id: permission_turn_id,
                    request_id,
                    ..
                } if *permission_turn_id == turn_id
                    && permission.as_ref().is_some_and(|request| {
                        request_id.as_deref() == Some(request.id.as_str())
                    }) =>
                {
                    permission = None;
                }
                _ => {}
            }
        }
        let phase = if permission.is_some() {
            TurnPhase::WaitingPermission
        } else {
            TurnPhase::Streaming
        };
        Some(Turn::restore(turn_id, phase, permission))
    }

    pub fn canonical_active_turn_matches(
        events: &[AgentSessionDomainEvent],
        projected_last_turn_id: Option<u64>,
        expected_turn_id: u64,
    ) -> bool {
        Self::current_turn_from_events(events, false)
            .is_some_and(|turn| turn.id() == expected_turn_id)
            && projected_last_turn_id == Some(expected_turn_id)
    }

    pub fn projection_allows_queue_start(
        state: SessionState,
        events: &[AgentSessionDomainEvent],
    ) -> bool {
        state.is_open() && Self::current_turn_from_events(events, false).is_none()
    }

    pub fn decide_terminal_events(
        previous: &[AgentSessionDomainEvent],
        supplied: &[AgentSessionDomainEvent],
    ) -> TerminalEventApplication {
        let Some((terminal_index, turn_id)) =
            supplied
                .iter()
                .enumerate()
                .find_map(|(index, event)| match event {
                    AgentSessionDomainEvent::TurnCompleted { turn_id, .. }
                    | AgentSessionDomainEvent::TurnInterrupted { turn_id, .. } => {
                        Some((index, *turn_id))
                    }
                    _ => None,
                })
        else {
            return TerminalEventApplication::NoTerminal;
        };
        let current_turn_id = previous.iter().rev().find_map(|event| match event {
            AgentSessionDomainEvent::TurnStarted { turn_id, .. } => Some(*turn_id),
            _ => None,
        });
        if previous.iter().any(|event| {
            matches!(
                event,
                AgentSessionDomainEvent::TurnCompleted { turn_id: id, .. }
                    | AgentSessionDomainEvent::TurnInterrupted { turn_id: id, .. }
                    if *id == turn_id
            )
        }) {
            return TerminalEventApplication::AlreadyApplied { turn_id };
        }
        if current_turn_id != Some(turn_id) {
            return TerminalEventApplication::Superseded { turn_id };
        }
        TerminalEventApplication::Current {
            terminal_index,
            turn_id,
        }
    }

    pub fn terminal_turn_id(events: &[AgentSessionDomainEvent]) -> Option<u64> {
        events.iter().rev().find_map(|event| match event {
            AgentSessionDomainEvent::TurnCompleted { turn_id, .. }
            | AgentSessionDomainEvent::TurnInterrupted { turn_id, .. } => Some(*turn_id),
            _ => None,
        })
    }

    pub fn terminal_commit_is_current(
        previous: &[AgentSessionDomainEvent],
        supplied: &[AgentSessionDomainEvent],
        durable_terminal_exists: bool,
    ) -> bool {
        matches!(
            Self::decide_terminal_events(previous, supplied),
            TerminalEventApplication::NoTerminal | TerminalEventApplication::Current { .. }
        ) && !durable_terminal_exists
    }

    pub fn requires_workflow_turn_completion(
        interrupted: bool,
        exit_code: i64,
        has_failure_signal: bool,
    ) -> bool {
        !interrupted || exit_code != 0 || has_failure_signal
    }

    pub fn discard_non_current_terminal_events(
        supplied: &[AgentSessionDomainEvent],
        turn_id: u64,
    ) -> Vec<AgentSessionDomainEvent> {
        supplied
            .iter()
            .filter(|event| {
                !matches!(
                    event,
                    AgentSessionDomainEvent::TurnInterruptRequested { turn_id: id, .. }
                        | AgentSessionDomainEvent::FinalPartsRecorded { turn_id: id, .. }
                        | AgentSessionDomainEvent::ToolCallFailed { turn_id: id, .. }
                        | AgentSessionDomainEvent::PermissionResolved { turn_id: id, .. }
                        | AgentSessionDomainEvent::TurnCompleted { turn_id: id, .. }
                        | AgentSessionDomainEvent::TurnInterrupted { turn_id: id, .. }
                        if *id == turn_id
                ) && !matches!(event, AgentSessionDomainEvent::QueuePaused { .. })
            })
            .cloned()
            .collect()
    }

    pub fn canonicalize_terminal_queue_pause(
        previous: &[AgentSessionDomainEvent],
        mut candidate: Vec<AgentSessionDomainEvent>,
    ) -> Vec<AgentSessionDomainEvent> {
        let queue_is_paused = previous.iter().fold(false, |paused, event| match event {
            AgentSessionDomainEvent::QueuePaused { .. } => true,
            AgentSessionDomainEvent::QueueResumed { .. } => false,
            _ => paused,
        });
        if queue_is_paused {
            candidate.retain(|event| !matches!(event, AgentSessionDomainEvent::QueuePaused { .. }));
        }
        candidate
    }

    pub fn terminal_requires_queue_pause(events: &[AgentSessionDomainEvent]) -> bool {
        events.iter().any(|event| {
            matches!(event, AgentSessionDomainEvent::TurnInterrupted { .. })
                || matches!(
                    event,
                    AgentSessionDomainEvent::TurnCompleted { exit_code, .. } if *exit_code != 0
                )
        })
    }

    pub fn converge_terminal_events(
        previous: &[AgentSessionDomainEvent],
        supplied: &[AgentSessionDomainEvent],
        project_final_parts: impl FnOnce(
            &[AgentSessionDomainEvent],
            &str,
        )
            -> Vec<crate::domain::agent_session::entities::MessagePart>,
    ) -> Vec<AgentSessionDomainEvent> {
        let (terminal_index, turn_id) = match Self::decide_terminal_events(previous, supplied) {
            TerminalEventApplication::NoTerminal => return supplied.to_vec(),
            TerminalEventApplication::AlreadyApplied { turn_id }
            | TerminalEventApplication::Superseded { turn_id } => {
                return Self::discard_non_current_terminal_events(supplied, turn_id);
            }
            TerminalEventApplication::Current {
                terminal_index,
                turn_id,
            } => (terminal_index, turn_id),
        };

        let AgentSessionDomainEvent::TurnInterrupted {
            reason,
            error,
            exit_code,
            ..
        } = &supplied[terminal_index]
        else {
            return Self::canonicalize_terminal_queue_pause(previous, supplied.to_vec());
        };
        let reason = *reason;
        let error = error.clone();
        let exit_code = *exit_code;

        let mut full = previous.to_vec();
        full.extend_from_slice(&supplied[..terminal_index]);
        let delta_start = previous.len();
        let has_final_parts = full.iter().any(|event| {
            matches!(
                event,
                AgentSessionDomainEvent::FinalPartsRecorded { turn_id: id, .. } if *id == turn_id
            )
        });
        if !has_final_parts {
            let assistant_message_id = full.iter().rev().find_map(|event| match event {
                AgentSessionDomainEvent::TurnStarted {
                    turn_id: id,
                    message_id,
                    assistant_message_id,
                    ..
                } if *id == turn_id => Some(
                    assistant_message_id
                        .clone()
                        .unwrap_or_else(|| format!("{message_id}:agent")),
                ),
                _ => None,
            });
            if let Some(message_id) = assistant_message_id {
                let parts = project_final_parts(&full, &message_id);
                full.push(AgentSessionDomainEvent::FinalPartsRecorded {
                    turn_id,
                    parts,
                    message_id,
                });
            }
        }
        Self::finalize_interrupted_turn(&mut full, turn_id, reason, error, exit_code);
        let mut completed = full.into_iter().skip(delta_start).collect::<Vec<_>>();
        completed.extend_from_slice(&supplied[terminal_index.saturating_add(1)..]);
        Self::canonicalize_terminal_queue_pause(previous, completed)
    }

    pub fn restore(restore: SessionRestore) -> Result<Self, SessionRestoreError> {
        if restore.id.is_empty() {
            return Err(SessionRestoreError::EmptyIdentity);
        }
        if restore.state.is_closed() && restore.current_turn.is_some() {
            return Err(SessionRestoreError::ClosedWithActiveTurn);
        }
        if let Some(turn) = &restore.current_turn {
            if turn.phase() == TurnPhase::Idle {
                return Err(SessionRestoreError::IdlePhaseWithCurrentTurn);
            }
            if turn.permission().is_some() && turn.phase() != TurnPhase::WaitingPermission {
                return Err(SessionRestoreError::PermissionWithoutWaitingTurn);
            }
        }
        Ok(Self {
            id: restore.id,
            revision: restore.revision,
            state: restore.state,
            has_messages: restore.has_messages,
            has_provider_session: restore.has_provider_session,
            current_turn: restore.current_turn,
            last_terminal: restore.last_terminal,
            queue: restore.queue,
            recovery_fact: restore.recovery_fact,
        })
    }

    pub fn new(id: String) -> Result<Self, SessionRestoreError> {
        Self::restore(SessionRestore {
            id,
            revision: 0,
            state: SessionState::Idle,
            has_messages: false,
            has_provider_session: false,
            current_turn: None,
            last_terminal: None,
            queue: QueueState::restore(Vec::new(), false),
            recovery_fact: RecoveryFact::Resolved,
        })
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn state(&self) -> SessionState {
        self.state
    }

    pub fn is_closed(&self) -> bool {
        self.state == SessionState::Closed
    }

    pub fn active_turn_id(&self) -> Option<u64> {
        self.current_turn.as_ref().map(Turn::id)
    }

    pub fn owns_active_turn(&self, turn_id: u64) -> bool {
        self.active_turn_id() == Some(turn_id)
    }

    #[cfg(test)]
    pub fn queue(&self) -> &QueueState {
        &self.queue
    }

    pub fn is_quiescent(&self) -> bool {
        self.current_turn.is_none() && self.queue.is_empty()
    }

    pub fn admit_send(&self) -> Result<SendDispositionDecision, TransitionRejection> {
        self.require_open_and_recovered()?;
        if self.current_turn.is_some() || !self.queue.is_empty() || self.queue.is_paused() {
            Ok(SendDispositionDecision::Queue)
        } else {
            Ok(SendDispositionDecision::StartImmediately)
        }
    }

    pub fn admit_workflow_turn(&self) -> Result<(), TransitionRejection> {
        self.require_open_and_recovered()?;
        if !self.is_quiescent() {
            return Err(TransitionRejection::NotQuiescent);
        }
        Ok(())
    }

    fn admit_stop(&self, turn_id: u64) -> Result<(), TransitionRejection> {
        let Some(turn) = self.current_turn.as_ref() else {
            return Err(TransitionRejection::NoActiveTurn);
        };
        if turn.id() != turn_id {
            return Err(TransitionRejection::StaleTarget);
        }
        Ok(())
    }

    pub fn admit_permission_response(&self, request_id: &str) -> Result<u64, TransitionRejection> {
        let Some(turn) = self.current_turn.as_ref() else {
            return Err(TransitionRejection::NoActiveTurn);
        };
        let Some(permission) = turn.permission() else {
            return Err(TransitionRejection::PermissionNotPending);
        };
        if permission.id != request_id || !permission.is_pending() {
            return Err(TransitionRejection::StaleTarget);
        }
        Ok(turn.id())
    }

    pub fn admit_close(&self) -> Result<(), TransitionRejection> {
        match self.state {
            SessionState::Closed | SessionState::Archived => {
                Err(TransitionRejection::InvalidLifecycle)
            }
            _ => Ok(()),
        }
    }

    pub fn admit_archive_open(&self) -> Result<(), TransitionRejection> {
        if self.state.is_closed() {
            return Err(TransitionRejection::InvalidLifecycle);
        }
        Ok(())
    }

    pub fn admit_archive_closed(&self) -> Result<(), TransitionRejection> {
        match self.state {
            SessionState::Closed => Ok(()),
            _ => Err(TransitionRejection::InvalidLifecycle),
        }
    }

    pub fn admit_backend_switch(&self) -> Result<(), TransitionRejection> {
        if self.state.is_closed() {
            return Err(TransitionRejection::SessionClosed);
        }
        if self.current_turn.is_some() {
            return Err(TransitionRejection::NotQuiescent);
        }
        if !self.queue.is_empty() {
            return Err(TransitionRejection::QueueNotEmpty);
        }
        if self.recovery_fact.is_unresolved() {
            return Err(TransitionRejection::UnresolvedRecovery);
        }
        Ok(())
    }

    /// Admit changing the provider selected for a session that has not yet
    /// established provider-owned history.
    ///
    /// This is intentionally stricter than the explicit lifecycle
    /// `SwitchBackend` action, which is allowed to discard completed history.
    pub fn admit_backend_selection_change(&self) -> Result<(), TransitionRejection> {
        self.admit_backend_switch()?;
        if self.has_messages || self.has_provider_session {
            return Err(TransitionRejection::NotQuiescent);
        }
        Ok(())
    }

    pub fn start_turn(&mut self, turn: Turn) -> TransitionOutcome {
        if let Err(rejection) = self.require_open_and_recovered() {
            return TransitionOutcome::Rejected(rejection);
        }
        if let Some(current) = self.current_turn.as_ref() {
            return if current.id() == turn.id() {
                TransitionOutcome::AlreadyApplied
            } else {
                TransitionOutcome::Rejected(TransitionRejection::NotQuiescent)
            };
        }
        if !self.queue.is_empty() {
            return TransitionOutcome::Rejected(TransitionRejection::QueueNotEmpty);
        }
        if turn.phase() == TurnPhase::Idle {
            return TransitionOutcome::Rejected(TransitionRejection::InvalidLifecycle);
        }
        self.current_turn = Some(turn);
        self.has_messages = true;
        self.state = SessionState::Active;
        self.bump_revision();
        TransitionOutcome::Applied
    }

    /// Apply a turn-start fact that has already crossed command admission.
    ///
    /// A newer durable fact supersedes a stale bounded current-turn fact. It
    /// must still not bypass the canonical queue, because that queue is part
    /// of the same aggregate authority.
    pub fn apply_observed_turn_start(&mut self, turn: Turn) -> TransitionOutcome {
        if !self.queue.is_empty() {
            return TransitionOutcome::Rejected(TransitionRejection::QueueNotEmpty);
        }
        if turn.phase() == TurnPhase::Idle {
            return TransitionOutcome::Rejected(TransitionRejection::InvalidLifecycle);
        }
        if self
            .current_turn
            .as_ref()
            .is_some_and(|current| current.id() == turn.id())
        {
            return TransitionOutcome::AlreadyApplied;
        }
        self.current_turn = Some(turn);
        self.has_messages = true;
        self.state = SessionState::Active;
        self.bump_revision();
        TransitionOutcome::Applied
    }

    pub fn enqueue(&mut self, item: QueueItem) -> TransitionOutcome {
        if let Err(rejection) = self.require_open_and_recovered() {
            return TransitionOutcome::Rejected(rejection);
        }
        let outcome = self.queue.enqueue(item);
        if outcome == TransitionOutcome::Applied {
            self.has_messages = true;
            self.bump_revision();
        }
        outcome
    }

    pub fn apply_send(
        &mut self,
        command: SessionSendCommand,
    ) -> Result<SessionSendTransition, SendCommandRejection> {
        if self.id != command.expected_session_id {
            return Err(SendCommandRejection::IdentityMismatch);
        }
        let queue = if command.workflow_turn {
            self.admit_workflow_turn()
                .map_err(SendCommandRejection::Transition)?;
            false
        } else {
            match self
                .admit_send()
                .map_err(SendCommandRejection::Transition)?
            {
                SendDispositionDecision::StartImmediately => false,
                SendDispositionDecision::Queue => true,
            }
        };
        let turn_id = command
            .reserved_turn_id
            .as_deref()
            .or(match &command.disposition {
                SendDisposition::StartedTurn { turn_id } => Some(turn_id.as_str()),
                SendDisposition::Queued { .. } => None,
            })
            .ok_or(SendCommandRejection::TurnIdentityUnavailable)?
            .parse::<u64>()
            .map_err(|_| SendCommandRejection::InvalidTurnIdentity)?;
        let queue_item_id = match &command.disposition {
            SendDisposition::Queued { queue_item_id } => queue_item_id.clone(),
            SendDisposition::StartedTurn { .. } => {
                format!("queue-{}", command.human_message_id)
            }
        };
        let transition = if queue {
            self.enqueue(QueueItem {
                id: queue_item_id.clone(),
                operation_id: command.input_ref,
                reserved_turn_id: Some(turn_id.to_string()),
                human_message_id: Some(command.human_message_id.clone()),
            })
        } else {
            self.start_turn(Turn::start(turn_id))
        };
        if transition != TransitionOutcome::Applied {
            return Err(SendCommandRejection::TransitionNotApplied);
        }
        if queue {
            Ok(SessionSendTransition {
                disposition: SendDisposition::Queued { queue_item_id },
                reserved_turn_id: Some(turn_id.to_string()),
            })
        } else {
            Ok(SessionSendTransition {
                disposition: SendDisposition::StartedTurn {
                    turn_id: turn_id.to_string(),
                },
                reserved_turn_id: None,
            })
        }
    }

    pub fn start_queue_head(&mut self, expected_id: &str, turn: Turn) -> TransitionOutcome {
        if let Err(rejection) = self.require_open_and_recovered() {
            return TransitionOutcome::Rejected(rejection);
        }
        if self.queue.is_paused() {
            return TransitionOutcome::Rejected(TransitionRejection::QueuePaused);
        }
        if self.current_turn.is_some() {
            return TransitionOutcome::Rejected(TransitionRejection::NotQuiescent);
        }
        let pop = self.queue.pop_head(expected_id);
        if pop != TransitionOutcome::Applied {
            return pop;
        }
        self.current_turn = Some(turn);
        self.state = SessionState::Active;
        self.bump_revision();
        TransitionOutcome::Applied
    }

    pub fn apply_queue_start(
        &mut self,
        expected_id: &str,
        human_message_id: &str,
        turn: Turn,
    ) -> Result<QueueStartTransition, QueueStartRejection> {
        let head = self
            .queue
            .items
            .front()
            .ok_or(QueueStartRejection::Transition(
                TransitionRejection::StaleTarget,
            ))?;
        if head.id != expected_id || head.human_message_id.as_deref() != Some(human_message_id) {
            return Err(QueueStartRejection::IdentityMismatch);
        }
        let reserved_turn_id = head
            .reserved_turn_id
            .as_deref()
            .ok_or(QueueStartRejection::InvalidReservedTurnIdentity)?
            .parse::<u64>()
            .map_err(|_| QueueStartRejection::InvalidReservedTurnIdentity)?;
        if reserved_turn_id != turn.id() {
            return Err(QueueStartRejection::IdentityMismatch);
        }
        match self.start_queue_head(expected_id, turn) {
            TransitionOutcome::Applied => Ok(QueueStartTransition {
                consumed_queue_item_id: expected_id.to_string(),
            }),
            TransitionOutcome::AlreadyApplied
            | TransitionOutcome::NotApplicable
            | TransitionOutcome::Rejected(TransitionRejection::StaleTarget) => {
                Err(QueueStartRejection::IdentityMismatch)
            }
            TransitionOutcome::Rejected(rejection) => {
                Err(QueueStartRejection::Transition(rejection))
            }
        }
    }

    pub fn request_permission(
        &mut self,
        turn_id: u64,
        request: PermissionRequest,
    ) -> TransitionOutcome {
        let Some(turn) = self.current_turn.as_mut() else {
            return TransitionOutcome::Rejected(TransitionRejection::NoActiveTurn);
        };
        if turn.id() != turn_id {
            return TransitionOutcome::Rejected(TransitionRejection::StaleTarget);
        }
        let outcome = turn.request_permission(request);
        if outcome == TransitionOutcome::Applied {
            self.bump_revision();
        }
        outcome
    }

    pub fn resolve_permission(
        &mut self,
        turn_id: u64,
        response: &PermissionResponse,
    ) -> TransitionOutcome {
        let Some(turn) = self.current_turn.as_mut() else {
            return TransitionOutcome::Rejected(TransitionRejection::NoActiveTurn);
        };
        if turn.id() != turn_id {
            return TransitionOutcome::Rejected(TransitionRejection::StaleTarget);
        }
        let outcome = turn.resolve_permission(response);
        if outcome == TransitionOutcome::Applied {
            self.bump_revision();
        }
        outcome
    }

    pub fn apply_accepted_permission_result(
        &mut self,
        turn_id: u64,
        response: &PermissionResponse,
    ) -> PermissionEffectCompletion {
        match self.resolve_permission(turn_id, response) {
            TransitionOutcome::Applied => PermissionEffectCompletion::ProjectResolution,
            TransitionOutcome::AlreadyApplied => PermissionEffectCompletion::AlreadySettled,
            TransitionOutcome::Rejected(
                TransitionRejection::NoActiveTurn
                | TransitionRejection::PermissionNotPending
                | TransitionRejection::StaleTarget,
            ) => PermissionEffectCompletion::Superseded,
            TransitionOutcome::NotApplicable => {
                PermissionEffectCompletion::Rejected(TransitionRejection::InvalidLifecycle)
            }
            TransitionOutcome::Rejected(rejection) => {
                PermissionEffectCompletion::Rejected(rejection)
            }
        }
    }

    pub fn apply_terminal(&mut self, turn_id: u64, result: TurnResult) -> TerminalDecision {
        let Some(turn) = self.current_turn.as_ref() else {
            let application = if self.last_terminal.as_ref() == Some(&result) {
                TerminalApplication::AlreadyApplied
            } else {
                TerminalApplication::Superseded
            };
            return TerminalDecision {
                application,
                result,
                pause_queue: false,
            };
        };
        if turn.id() != turn_id {
            return TerminalDecision {
                application: TerminalApplication::Superseded,
                result,
                pause_queue: false,
            };
        }
        let outcome = Self::terminal_outcome(&result);
        self.state = outcome.session_state;
        if outcome.pause_queue {
            self.queue.pause();
        }
        self.current_turn = None;
        self.last_terminal = Some(result.clone());
        self.bump_revision();
        TerminalDecision {
            application: TerminalApplication::Current,
            result,
            pause_queue: outcome.pause_queue,
        }
    }

    pub fn pause_queue(&mut self) -> TransitionOutcome {
        let outcome = self.queue.pause();
        if outcome == TransitionOutcome::Applied {
            self.bump_revision();
        }
        outcome
    }

    pub fn apply_stop(
        &mut self,
        expected_revision: u64,
        turn_id: u64,
    ) -> Result<StopTransition, TransitionRejection> {
        if self.revision != expected_revision {
            return Err(TransitionRejection::StaleTarget);
        }
        self.admit_stop(turn_id)?;
        let queue_was_paused = self.queue.is_paused();
        match self.pause_queue() {
            TransitionOutcome::Applied | TransitionOutcome::AlreadyApplied => Ok(StopTransition {
                turn_id,
                queue_was_paused,
            }),
            TransitionOutcome::NotApplicable | TransitionOutcome::Rejected(_) => {
                Err(TransitionRejection::StaleTarget)
            }
        }
    }

    pub fn apply_stop_command(
        &mut self,
        expected_revision: u64,
        turn_id: &str,
    ) -> Result<StopTransition, StopCommandRejection> {
        let turn_id = turn_id
            .parse::<u64>()
            .map_err(|_| StopCommandRejection::InvalidTurnIdentity)?;
        self.apply_stop(expected_revision, turn_id)
            .map_err(StopCommandRejection::Transition)
    }

    pub fn resume_queue(&mut self) -> TransitionOutcome {
        if self.recovery_fact.is_unresolved() {
            return TransitionOutcome::Rejected(TransitionRejection::UnresolvedRecovery);
        }
        let outcome = self.queue.resume();
        if outcome == TransitionOutcome::Applied {
            self.bump_revision();
        }
        outcome
    }

    /// Applies one accepted lifecycle command and returns the complete
    /// persistence/effect plan. Callers choose ordering; they do not
    /// reinterpret which terminal, queue, or runtime consequences belong to
    /// the command.
    pub fn apply_lifecycle(
        &mut self,
        command: SessionLifecycleCommand,
    ) -> Result<SessionLifecycleTransition, TransitionRejection> {
        match &command {
            SessionLifecycleCommand::Close => self.admit_close()?,
            SessionLifecycleCommand::ArchiveOpen => self.admit_archive_open()?,
            SessionLifecycleCommand::ArchiveClosed => self.admit_archive_closed()?,
            SessionLifecycleCommand::SwitchBackend { .. } => self.admit_backend_switch()?,
        }

        let queue_was_paused = self.queue.is_paused();
        let active_turn_id = self.active_turn_id();
        let (pauses_queue, emits_session_closed, runtime_effect_if_present, backend_id) =
            match &command {
                SessionLifecycleCommand::Close | SessionLifecycleCommand::ArchiveOpen => {
                    (true, true, true, None)
                }
                SessionLifecycleCommand::ArchiveClosed => (false, false, false, None),
                SessionLifecycleCommand::SwitchBackend { backend_id } => {
                    (true, false, true, Some(backend_id.clone()))
                }
            };

        if pauses_queue {
            self.queue.pause();
        }
        match command {
            SessionLifecycleCommand::Close => {
                self.current_turn = None;
                self.state = SessionState::Closed;
            }
            SessionLifecycleCommand::ArchiveOpen | SessionLifecycleCommand::ArchiveClosed => {
                self.current_turn = None;
                self.state = SessionState::Archived;
            }
            SessionLifecycleCommand::SwitchBackend { .. } => {
                self.state = SessionState::Idle;
                self.has_provider_session = false;
            }
        }
        self.bump_revision();

        let terminal = active_turn_id.map(|turn_id| {
            (
                turn_id,
                TurnResult::Interrupted {
                    reason: InterruptReason::SessionClosed,
                    error: None,
                },
            )
        });
        Ok(SessionLifecycleTransition {
            terminal,
            queue_was_paused,
            pauses_queue,
            emits_session_closed,
            backend_id,
            runtime_effect_if_present,
        })
    }

    pub fn apply_lifecycle_at_revision(
        &mut self,
        expected_revision: i64,
        command: SessionLifecycleCommand,
    ) -> Result<SessionLifecycleTransition, LifecycleCommandRejection> {
        let current_revision = i64::try_from(self.revision)
            .map_err(|_| LifecycleCommandRejection::RevisionUnrepresentable)?;
        if current_revision != expected_revision {
            return Err(LifecycleCommandRejection::RevisionConflict { current_revision });
        }
        self.apply_lifecycle(command)
            .map_err(LifecycleCommandRejection::Transition)
    }

    fn require_open_and_recovered(&self) -> Result<(), TransitionRejection> {
        if self.state.is_closed() {
            return Err(TransitionRejection::SessionClosed);
        }
        if self.recovery_fact.is_unresolved() {
            return Err(TransitionRejection::UnresolvedRecovery);
        }
        Ok(())
    }

    fn bump_revision(&mut self) {
        self.revision = self.revision.saturating_add(1);
    }
}

#[cfg(test)]
mod tests;
