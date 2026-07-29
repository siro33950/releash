use crate::domain::agent_session::value_objects::PermissionMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionToolKind {
    Interactive,
    NonInteractive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderPermissionDecision {
    AutoAllow,
    Prompt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PermissionResponseRuntimeDecision {
    pub patch_cached_projection: bool,
    pub resume_streaming: bool,
}

pub fn decide_permission_response_runtime_completion(
    has_cached_request: bool,
    cached_request_matches: bool,
    response_came_from_runtime: bool,
) -> PermissionResponseRuntimeDecision {
    PermissionResponseRuntimeDecision {
        patch_cached_projection: cached_request_matches,
        resume_streaming: has_cached_request && cached_request_matches
            || !response_came_from_runtime,
    }
}

pub fn permission_response_turn_matches(
    pending_turn_id: Option<u64>,
    expected_turn_id: u64,
) -> bool {
    pending_turn_id == Some(expected_turn_id)
}

pub fn runtime_permission_effect_is_owned(has_runtime: bool, owns_pending_request: bool) -> bool {
    has_runtime && owns_pending_request
}

pub fn permission_request_identity_matches(observed: &str, expected: &str) -> bool {
    observed == expected
}

pub fn decide_provider_permission(
    mode: PermissionMode,
    plan_mode: bool,
    tool_kind: PermissionToolKind,
) -> ProviderPermissionDecision {
    let interactive = tool_kind == PermissionToolKind::Interactive;
    if plan_mode {
        return if interactive {
            ProviderPermissionDecision::Prompt
        } else {
            ProviderPermissionDecision::AutoAllow
        };
    }
    match mode {
        PermissionMode::Full => ProviderPermissionDecision::AutoAllow,
        PermissionMode::Edit if !interactive => ProviderPermissionDecision::AutoAllow,
        PermissionMode::Ask | PermissionMode::Edit => ProviderPermissionDecision::Prompt,
    }
}

pub fn classify_permission_tool(tool_name: &str) -> PermissionToolKind {
    match tool_name {
        "AskUserQuestion" | "EnterPlanMode" | "ExitPlanMode" => PermissionToolKind::Interactive,
        _ => PermissionToolKind::NonInteractive,
    }
}

pub fn decide_provider_permission_for_tool(
    mode: PermissionMode,
    plan_mode: bool,
    tool_name: &str,
) -> ProviderPermissionDecision {
    decide_provider_permission(mode, plan_mode, classify_permission_tool(tool_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_permission_policy_owns_auto_allow_admission() {
        assert_eq!(
            decide_provider_permission(
                PermissionMode::Full,
                false,
                PermissionToolKind::Interactive
            ),
            ProviderPermissionDecision::AutoAllow
        );
        assert_eq!(
            decide_provider_permission(
                PermissionMode::Edit,
                false,
                PermissionToolKind::NonInteractive
            ),
            ProviderPermissionDecision::AutoAllow
        );
        assert_eq!(
            decide_provider_permission(PermissionMode::Edit, true, PermissionToolKind::Interactive),
            ProviderPermissionDecision::Prompt
        );
        assert_eq!(
            decide_permission_response_runtime_completion(true, true, true),
            PermissionResponseRuntimeDecision {
                patch_cached_projection: true,
                resume_streaming: true,
            }
        );
        assert!(
            decide_permission_response_runtime_completion(false, false, false).resume_streaming
        );
        assert!(permission_response_turn_matches(Some(7), 7));
        assert_eq!(
            classify_permission_tool("AskUserQuestion"),
            PermissionToolKind::Interactive
        );
        assert_eq!(
            classify_permission_tool("Read"),
            PermissionToolKind::NonInteractive
        );
    }
}
