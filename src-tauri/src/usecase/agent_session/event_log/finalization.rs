use super::events::{AgentSessionEvent, InterruptReason, TurnId};
use crate::domain::agent_session::aggregates::session::Session as SessionAggregate;
use crate::domain::agent_session::entities::PermissionRequest;

pub fn finalize_turn(
    events: &mut Vec<AgentSessionEvent>,
    turn_id: TurnId,
    reason: InterruptReason,
    error: Option<String>,
    exit_code: i64,
) {
    SessionAggregate::finalize_interrupted_turn(events, turn_id, reason, error, exit_code);
}

#[derive(Debug, Clone)]
pub(crate) struct UnresolvedPermissionRequest {
    pub turn_id: TurnId,
    pub request: PermissionRequest,
}

pub(crate) fn latest_unresolved_permission_request(
    events: &[AgentSessionEvent],
) -> Option<UnresolvedPermissionRequest> {
    SessionAggregate::latest_unresolved_permission(events).map(|permission| {
        UnresolvedPermissionRequest {
            turn_id: permission.turn_id,
            request: permission.request,
        }
    })
}
