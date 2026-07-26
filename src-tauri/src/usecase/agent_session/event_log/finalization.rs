use std::collections::HashSet;

use super::events::{AgentSessionEvent, InterruptReason, PermissionDecision, TurnId};
use super::part_events::permission_request_id;
use crate::domain::agent_session::entities::PermissionRequest;
use crate::usecase::agent_session::session::PermissionRequestMsg;

pub fn finalize_turn(
    events: &mut Vec<AgentSessionEvent>,
    turn_id: TurnId,
    reason: InterruptReason,
    error: Option<String>,
    exit_code: i64,
) {
    if has_turn_terminal(events, turn_id) {
        return;
    }

    let interrupted_content = interruption_content(reason, error.as_deref());
    for tool_use_id in unfinished_tool_calls(events, turn_id) {
        events.push(AgentSessionEvent::ToolCallFailed {
            turn_id,
            tool_use_id,
            content: interrupted_content.clone(),
            content_ref: None,
            summary: None,
        });
    }

    for permission in unresolved_permissions_for_turn(events, turn_id) {
        events.push(AgentSessionEvent::PermissionResolved {
            turn_id,
            tool_use_id: permission.tool_use_id,
            request_id: permission.request_id,
            decision: PermissionDecision::Cancelled,
            answers: None,
        });
    }

    events.push(AgentSessionEvent::TurnInterrupted {
        turn_id,
        reason,
        exit_code,
        error,
    });
}

pub(super) fn has_unresolved_permissions(events: &[AgentSessionEvent], turn_id: TurnId) -> bool {
    !unresolved_permissions_for_turn(events, turn_id).is_empty()
}

#[derive(Debug, Clone)]
pub(crate) struct UnresolvedPermissionRequest {
    pub turn_id: TurnId,
    pub request: PermissionRequestMsg,
}

pub(crate) fn latest_unresolved_permission_request(
    events: &[AgentSessionEvent],
) -> Option<UnresolvedPermissionRequest> {
    let turn_id = events.iter().rev().find_map(|event| match event {
        AgentSessionEvent::TurnStarted { turn_id, .. } => Some(*turn_id),
        _ => None,
    })?;
    if has_turn_terminal(events, turn_id) {
        return None;
    }

    unresolved_permissions_for_turn(events, turn_id)
        .pop()
        .map(|permission| UnresolvedPermissionRequest {
            turn_id: permission.turn_id,
            request: crate::usecase::agent_session::runtime::event_apply::permission_request_msg(
                &permission.request,
            ),
        })
}

fn interruption_content(reason: InterruptReason, error: Option<&str>) -> String {
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

fn has_turn_terminal(events: &[AgentSessionEvent], turn_id: TurnId) -> bool {
    events.iter().any(|event| {
        matches!(
            event,
            AgentSessionEvent::TurnCompleted { turn_id: id, .. }
                | AgentSessionEvent::TurnInterrupted { turn_id: id, .. } if *id == turn_id
        )
    })
}

fn unfinished_tool_calls(events: &[AgentSessionEvent], turn_id: TurnId) -> Vec<String> {
    let mut started = Vec::new();
    let mut finished = HashSet::new();
    for event in events {
        match event {
            AgentSessionEvent::ToolCallStarted {
                turn_id: id,
                tool_use_id,
                ..
            } if *id == turn_id => started.push(tool_use_id.clone()),
            AgentSessionEvent::ToolCallSucceeded {
                turn_id: id,
                tool_use_id,
                ..
            }
            | AgentSessionEvent::ToolCallFailed {
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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct PermissionKey {
    tool_use_id: Option<String>,
    request_id: Option<String>,
}

#[derive(Debug, Clone)]
struct UnresolvedPermission {
    turn_id: TurnId,
    tool_use_id: Option<String>,
    request_id: Option<String>,
    request: PermissionRequest,
}

impl UnresolvedPermission {
    fn key(&self) -> PermissionKey {
        PermissionKey {
            tool_use_id: self.tool_use_id.clone(),
            request_id: self.request_id.clone(),
        }
    }
}

fn unresolved_permissions_for_turn(
    events: &[AgentSessionEvent],
    turn_id: TurnId,
) -> Vec<UnresolvedPermission> {
    let mut requested = Vec::new();
    let mut resolved = HashSet::new();
    for event in events {
        match event {
            AgentSessionEvent::PermissionRequested {
                turn_id: id,
                tool_use_id,
                request,
            } if *id == turn_id => requested.push(UnresolvedPermission {
                turn_id: *id,
                tool_use_id: tool_use_id.clone(),
                request_id: permission_request_id(request),
                request: request.clone(),
            }),
            AgentSessionEvent::PermissionResolved {
                turn_id: id,
                tool_use_id,
                request_id,
                ..
            } if *id == turn_id => {
                resolved.insert(PermissionKey {
                    tool_use_id: tool_use_id.clone(),
                    request_id: request_id.clone(),
                });
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
                    key.request_id.is_some() && key.request_id == resolved_key.request_id
                        || key.tool_use_id.is_some() && key.tool_use_id == resolved_key.tool_use_id
                })
        })
        .collect()
}
