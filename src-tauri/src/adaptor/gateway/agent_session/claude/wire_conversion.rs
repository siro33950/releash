use crate::domain::agent_session::value_objects::PermissionMode;
use crate::infrastructure::agent_session::claude::wire::ClaudeWireMode;

pub(crate) fn claude_wire_mode(mode: PermissionMode, plan_mode: bool) -> ClaudeWireMode {
    if plan_mode {
        return ClaudeWireMode::Plan;
    }
    match mode {
        PermissionMode::Ask => ClaudeWireMode::Default,
        PermissionMode::Edit => ClaudeWireMode::AcceptEdits,
        PermissionMode::Full => ClaudeWireMode::BypassPermissions,
    }
}

pub(crate) fn permission_mode_from_wire(mode: &str) -> Option<PermissionMode> {
    match mode {
        "default" => Some(PermissionMode::Ask),
        "acceptEdits" => Some(PermissionMode::Edit),
        "bypassPermissions" => Some(PermissionMode::Full),
        "plan" => None,
        _ => None,
    }
}

pub(crate) fn permission_mode_and_plan_from_wire(mode: ClaudeWireMode) -> (PermissionMode, bool) {
    match mode {
        ClaudeWireMode::Default => (PermissionMode::Ask, false),
        ClaudeWireMode::AcceptEdits => (PermissionMode::Edit, false),
        ClaudeWireMode::BypassPermissions => (PermissionMode::Full, false),
        ClaudeWireMode::Plan => (PermissionMode::Edit, true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_mode_has_wire_precedence() {
        assert_eq!(
            claude_wire_mode(PermissionMode::Full, true),
            ClaudeWireMode::Plan
        );
    }
}
