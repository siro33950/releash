use crate::domain::agent_session::value_objects::{ContextCarryState, SessionState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserSessionMetadataAction {
    Rename,
    Fork,
    #[cfg(test)]
    ArchiveOpen,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkflowOwnedSession;

pub fn admit_user_session_metadata_action(
    workflow_node_session: bool,
    _action: UserSessionMetadataAction,
) -> Result<(), WorkflowOwnedSession> {
    if workflow_node_session {
        Err(WorkflowOwnedSession)
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionForkDecision {
    pub state: SessionState,
    pub error_reason: Option<String>,
    pub provider_session_id: Option<String>,
    pub provider_session_generation: u64,
    pub provider_session_observation_id: Option<String>,
    pub context_reinjection_generation: Option<u64>,
    pub context_carry: Option<ContextCarryState>,
    pub clear_recovery_publication: bool,
    pub clear_last_turn_interruption: bool,
    pub last_turn_id: Option<u64>,
    pub workflow_node_session: bool,
}

pub fn decide_session_fork(
    workflow_node_session: bool,
) -> Result<SessionForkDecision, WorkflowOwnedSession> {
    admit_user_session_metadata_action(workflow_node_session, UserSessionMetadataAction::Fork)?;
    Ok(SessionForkDecision {
        state: SessionState::Idle,
        error_reason: None,
        provider_session_id: None,
        provider_session_generation: 0,
        provider_session_observation_id: None,
        context_reinjection_generation: None,
        context_carry: None,
        clear_recovery_publication: true,
        clear_last_turn_interruption: true,
        last_turn_id: Some(0),
        workflow_node_session: false,
    })
}

pub fn is_workflow_node_session(
    workflow_node_session: bool,
    has_workflow_node_context: bool,
) -> bool {
    workflow_node_session || has_workflow_node_context
}

pub fn backend_selection_changes(current: Option<&str>, requested: &str) -> bool {
    current != Some(requested)
}

#[cfg(test)]
pub fn should_apply_session_configuration<T: PartialEq>(
    recovery_in_progress: bool,
    current: &T,
    requested: &T,
) -> bool {
    !recovery_in_progress && current != requested
}

pub fn compact_session_title(title: &str) -> String {
    let compact = title.split_whitespace().collect::<Vec<_>>().join(" ");
    match compact.char_indices().nth(100) {
        Some((byte_pos, _)) => format!("{}…", &compact[..byte_pos]),
        None => compact,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidPermissionProfileIdentity;

pub fn normalize_permission_profile_id(
    permission_profile_id: Option<&str>,
) -> Result<Option<String>, InvalidPermissionProfileIdentity> {
    permission_profile_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            if value.chars().any(char::is_control) {
                Err(InvalidPermissionProfileIdentity)
            } else {
                Ok(value.to_string())
            }
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_owned_metadata_is_not_user_mutable() {
        assert_eq!(
            admit_user_session_metadata_action(true, UserSessionMetadataAction::Rename),
            Err(WorkflowOwnedSession)
        );
        assert!(admit_user_session_metadata_action(false, UserSessionMetadataAction::Fork).is_ok());
        assert_eq!(
            admit_user_session_metadata_action(true, UserSessionMetadataAction::ArchiveOpen),
            Err(WorkflowOwnedSession)
        );
        assert_eq!(decide_session_fork(true), Err(WorkflowOwnedSession));
        assert_eq!(
            decide_session_fork(false),
            Ok(SessionForkDecision {
                state: SessionState::Idle,
                error_reason: None,
                provider_session_id: None,
                provider_session_generation: 0,
                provider_session_observation_id: None,
                context_reinjection_generation: None,
                context_carry: None,
                clear_recovery_publication: true,
                clear_last_turn_interruption: true,
                last_turn_id: Some(0),
                workflow_node_session: false,
            })
        );
    }

    #[test]
    fn title_compaction_is_bounded_by_characters() {
        assert_eq!(compact_session_title("  one   two  "), "one two");
        assert_eq!(
            compact_session_title(&"あ".repeat(101)),
            format!("{}…", "あ".repeat(100))
        );
        assert_eq!(normalize_permission_profile_id(Some("  ")), Ok(None));
        assert_eq!(
            normalize_permission_profile_id(Some("profile")),
            Ok(Some("profile".into()))
        );
        assert_eq!(
            normalize_permission_profile_id(Some("bad\nprofile")),
            Err(InvalidPermissionProfileIdentity)
        );
    }
}
