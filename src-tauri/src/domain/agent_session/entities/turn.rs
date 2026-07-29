pub type TurnId = u64;

use crate::domain::agent_session::aggregates::session::{TransitionOutcome, TransitionRejection};
use crate::domain::agent_session::entities::{
    PermissionDecision, PermissionRequest, PermissionRequestStatus, PermissionResponse,
    PermissionResponseDecision,
};
use crate::domain::agent_session::value_objects::TurnPhase;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    id: TurnId,
    phase: TurnPhase,
    permission: Option<PermissionRequest>,
}

impl Turn {
    pub fn start(id: TurnId) -> Self {
        Self {
            id,
            phase: TurnPhase::Streaming,
            permission: None,
        }
    }

    pub fn restore(id: TurnId, phase: TurnPhase, permission: Option<PermissionRequest>) -> Self {
        Self {
            id,
            phase,
            permission,
        }
    }

    pub fn id(&self) -> TurnId {
        self.id
    }

    pub fn phase(&self) -> TurnPhase {
        self.phase
    }

    pub fn permission(&self) -> Option<&PermissionRequest> {
        self.permission.as_ref()
    }

    pub fn request_permission(&mut self, request: PermissionRequest) -> TransitionOutcome {
        if let Some(current) = self.permission.as_ref() {
            return if current.id == request.id {
                TransitionOutcome::AlreadyApplied
            } else {
                TransitionOutcome::Rejected(TransitionRejection::NotQuiescent)
            };
        }
        if !request.is_pending() {
            return TransitionOutcome::Rejected(TransitionRejection::InvalidLifecycle);
        }
        self.permission = Some(request);
        self.phase = TurnPhase::WaitingPermission;
        TransitionOutcome::Applied
    }

    pub fn resolve_permission(&mut self, response: &PermissionResponse) -> TransitionOutcome {
        let Some(permission) = self.permission.as_mut() else {
            return TransitionOutcome::Rejected(TransitionRejection::PermissionNotPending);
        };
        if permission.id != response.request_id {
            return TransitionOutcome::Rejected(TransitionRejection::StaleTarget);
        }
        if !permission.is_pending() {
            return TransitionOutcome::AlreadyApplied;
        }
        let (decision, answers) = match &response.decision {
            PermissionResponseDecision::Allow { answers, .. } => {
                (PermissionDecision::Allowed, answers.clone())
            }
            PermissionResponseDecision::Deny { .. } => (PermissionDecision::Denied, None),
        };
        permission.status = PermissionRequestStatus::Resolved { decision, answers };
        self.phase = TurnPhase::Streaming;
        TransitionOutcome::Applied
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnResult {
    Completed {
        stop_reason: Option<TurnStopReason>,
        token_usage: Option<TokenUsage>,
    },
    Failed {
        error: String,
        token_usage: Option<TokenUsage>,
    },
    Interrupted {
        reason: InterruptReason,
        error: Option<String>,
    },
}

impl TurnResult {
    pub fn requires_crash_snapshot(&self) -> bool {
        matches!(
            self,
            Self::Interrupted {
                reason: InterruptReason::Crash,
                ..
            }
        )
    }

    pub fn trailing_fatal_message(&self) -> Option<&str> {
        match self {
            Self::Interrupted {
                reason: InterruptReason::Crash,
                error: Some(message),
            } => Some(message),
            _ => None,
        }
    }

    pub fn token_usage(&self) -> Option<TokenUsage> {
        match self {
            Self::Completed { token_usage, .. } | Self::Failed { token_usage, .. } => *token_usage,
            Self::Interrupted { .. } => None,
        }
    }

    pub fn terminal_stream_sequence(&self, current: u64) -> u64 {
        if self.requires_crash_snapshot() {
            current.saturating_add(1)
        } else {
            current
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnStopReason {
    #[allow(dead_code)]
    // issues-1301 G-1: refusal is emitted by backend conversion fixtures and workflow failure projection, not every production path.
    Refusal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterruptReason {
    Abort,
    #[allow(dead_code)]
    // Stale watchdog no longer synthesizes timeout interrupts, but the domain boundary keeps the explicit backend/tool timeout vocabulary.
    Timeout,
    Crash,
    SessionClosed,
}

impl InterruptReason {
    pub fn label(self) -> &'static str {
        match self {
            Self::Abort => "abort",
            Self::Timeout => "timeout",
            Self::Crash => "crash",
            Self::SessionClosed => "session_closed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: Option<u64>,
    pub context_window_tokens: Option<u64>,
}
