pub(crate) mod context_replacement;
mod durable_counter;
pub mod operation_recovery_policy;
mod permission_policy;
mod recovery_inventory_policy;
mod runtime_event_policy;
pub mod send_operation_policy;
mod session_metadata_policy;
pub(crate) mod skill_frontmatter;
mod streaming_policy;

use super::SkillEntry;

pub(crate) use context_replacement::{
    dedup_instructions, latest_revisions_by_kind, next_epoch_for_identity,
    normalize_path_components, replacement_action, snapshot_is_stale,
};
pub use durable_counter::{add_durable_count, advance_durable_counter};
pub use operation_recovery_policy::{
    admit_backend_recovery_completion, admit_backend_recovery_failure,
    admit_backend_recovery_start, backend_recovery_effect_identity_matches,
    backend_recovery_error_digest, backend_recovery_failure_message_id,
    backend_recovery_obligation_id, backend_recovery_provider_observation_id,
    backend_recovery_reservation, classify_legacy_provider_establish,
    classify_permission_response_recovery, classify_recovery_readback,
    decide_backend_recovery_durable_completion, decide_recovery_action, next_recovery_retry_delay,
    recovery_classification_is_allowed, recovery_handoff_matches,
    recovery_publication_obligation_id, recovery_result_outcome, runtime_stop_request_id,
    session_close_readback_obligation_id, stop_readback_obligation_id,
    BackendRecoveryDurableCompletionDecision, BackendRecoveryDurableCompletionFacts,
    BackendRecoveryDurableCompletionRejection, BackendRecoveryReservationDecision,
    BackendRecoveryReservationRejection, LegacyProviderEstablishRecovery,
    PermissionResponseRecoveryObservation, RecoveryActionDecision, RecoveryActionRejection,
    RecoveryReadbackTarget,
};
pub use permission_policy::{
    decide_permission_response_runtime_completion, decide_provider_permission_for_tool,
    permission_request_identity_matches, permission_response_turn_matches,
    runtime_permission_effect_is_owned, ProviderPermissionDecision,
};
pub use recovery_inventory_policy::{
    bounded_recovery_owner_component, decide_recovery_capabilities, pending_recovery_descriptor,
    workflow_node_recovery_owner_target, PendingRecoveryCategory, PendingRecoveryKnownStatus,
    PendingRecoveryOwnerTarget, RecoveryActionIdentity, RecoveryCapabilities,
    RecoveryObservationFact, RecoveryResourceState,
};
pub use runtime_event_policy::{
    context_carry_for_established_resume, decide_runtime_event_admission,
    decide_session_established_event, require_terminal_commit_identity, runtime_error_message_id,
    runtime_event_recovery_id, runtime_provider_session_observation_id, RuntimeEventAdmission,
    RuntimeEventAdmissionFacts, SessionEstablishedEventDecision,
};
pub use send_operation_policy::{
    accepted_effect_execution_matches, accepted_effect_has_durable_execution_identity,
    accepted_effect_is_process_owned, accepted_prompt_matches,
    accepted_queued_effect_has_durable_identity, accepted_queued_effect_identity_is_consistent,
    accepted_queued_effect_matches, accepted_queued_effect_reservation_conflicts,
    accepted_queued_effect_should_retain, accepted_send_artifact_digest, accepted_send_retry_delay,
    accepted_send_target_matches, accepted_worktree_matches, admit_workflow_send_target,
    allocate_next_turn_identity, decide_accepted_queued_effect_queue, decide_runtime_turn_recovery,
    durable_workflow_turn_operation_id, queue_item_identity_matches,
    queued_effect_remains_unstarted, turn_identity_advances, turn_preclaim_failure_disposition,
    validate_accepted_effect_runtime_identity, workflow_send_receipt_matches,
    workflow_send_should_retry, workflow_turn_principal_is_authorized,
    AcceptedEffectExecutionIdentity, AcceptedEffectIdentityRejection, AcceptedQueuedEffectIdentity,
    AcceptedQueuedEffectQueueDecision, AcceptedSendTarget, CanonicalQueuedEffectIdentity,
    ReservedTurnIdentity, RuntimeTurnRecoveryDecision, TurnIdentityAllocationError,
    TurnPreclaimFailureDisposition, WorkflowSendTargetRejection,
    INTERNAL_WORKFLOW_OPERATION_PRINCIPAL, WORKFLOW_SEND_RETRY_ATTEMPTS,
};
#[cfg(test)]
pub use session_metadata_policy::should_apply_session_configuration;
pub use session_metadata_policy::{
    admit_user_session_metadata_action, backend_selection_changes, compact_session_title,
    decide_session_fork, is_workflow_node_session, normalize_permission_profile_id,
    UserSessionMetadataAction,
};
pub(crate) use skill_frontmatter::parse_skill_frontmatter;
pub use streaming_policy::{
    add_streaming_byte_size, next_stream_sequence, part_needs_event_history,
    part_records_durable_event, parts_can_stream_as_append_delta, patch_permission_response,
    should_persist_streaming_snapshot, stream_target_is_current,
    streaming_flush_decision_for_apply, streaming_parts_byte_size, StreamingFlushDecision,
    STREAMING_EMIT_INTERVAL,
};

/// Maximum image size in bytes (5 MiB).
/// Anthropic Messages API limits base64-encoded images to roughly 5 MB.
pub const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;

pub const MAX_TOOL_OUTPUT_BYTES: usize = 30 * 1024;
pub const MAX_TOOL_OUTPUT_LINES: usize = 1000;
pub const TOOL_OUTPUT_PREVIEW_BYTES: usize = MAX_TOOL_OUTPUT_BYTES;
pub const TOOL_OUTPUT_PREVIEW_LINES: usize = MAX_TOOL_OUTPUT_LINES;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOutputSummaryProjection {
    pub line_count: u64,
    pub byte_size: u64,
    pub is_error: bool,
    pub truncated: bool,
}

pub trait ToolOutputExternalizationPolicy {
    fn should_externalize_tool_output(&self, content: &str) -> bool;
    fn tool_output_preview(&self, content: &str) -> String;
    fn tool_output_summary(
        &self,
        content: &str,
        is_error: bool,
        truncated: bool,
    ) -> ToolOutputSummaryProjection;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultToolOutputExternalizationPolicy;

impl ToolOutputExternalizationPolicy for DefaultToolOutputExternalizationPolicy {
    fn should_externalize_tool_output(&self, content: &str) -> bool {
        tool_output_line_count(content) > MAX_TOOL_OUTPUT_LINES as u64
            || content.len() > MAX_TOOL_OUTPUT_BYTES
    }

    fn tool_output_preview(&self, content: &str) -> String {
        let line_limited = if tool_output_line_count(content) <= TOOL_OUTPUT_PREVIEW_LINES as u64 {
            content
        } else {
            let mut end = 0;
            for (line_index, chunk) in content.split_inclusive('\n').enumerate() {
                if line_index >= TOOL_OUTPUT_PREVIEW_LINES {
                    break;
                }
                end += chunk.len();
            }
            &content[..end]
        };
        truncate_to_char_boundary(line_limited, TOOL_OUTPUT_PREVIEW_BYTES).to_string()
    }

    fn tool_output_summary(
        &self,
        content: &str,
        is_error: bool,
        truncated: bool,
    ) -> ToolOutputSummaryProjection {
        ToolOutputSummaryProjection {
            line_count: tool_output_line_count(content),
            byte_size: content.len() as u64,
            is_error,
            truncated,
        }
    }
}

pub fn tool_output_line_count(content: &str) -> u64 {
    content.lines().count() as u64
}

fn truncate_to_char_boundary(content: &str, max_bytes: usize) -> &str {
    if content.len() <= max_bytes {
        return content;
    }
    let mut end = max_bytes;
    while end > 0 && !content.is_char_boundary(end) {
        end -= 1;
    }
    &content[..end]
}

pub trait AttachmentExternalizationPolicy {
    #[cfg(test)]
    fn reject_oversized_base64_image(&self, data: &str) -> Result<(), String>;
    fn validate_image_bytes(&self, bytes: &[u8]) -> Result<&'static str, String>;
    #[cfg(test)]
    fn validate_image_bytes_for_media_type(
        &self,
        bytes: &[u8],
        media_type: &str,
    ) -> Result<&'static str, String>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultAttachmentExternalizationPolicy;

impl AttachmentExternalizationPolicy for DefaultAttachmentExternalizationPolicy {
    #[cfg(test)]
    fn reject_oversized_base64_image(&self, data: &str) -> Result<(), String> {
        let max_encoded_len = max_base64_image_len();
        if data.len() > max_encoded_len {
            return Err(format!(
                "Image too large: encoded length {} exceeds max encoded length {}",
                data.len(),
                max_encoded_len
            ));
        }
        Ok(())
    }

    fn validate_image_bytes(&self, bytes: &[u8]) -> Result<&'static str, String> {
        if bytes.len() > MAX_IMAGE_BYTES {
            return Err(format!(
                "Image too large: {} bytes (max {} bytes)",
                bytes.len(),
                MAX_IMAGE_BYTES
            ));
        }

        detect_image_mime(bytes).ok_or_else(|| "Unsupported image format".to_string())
    }

    #[cfg(test)]
    fn validate_image_bytes_for_media_type(
        &self,
        bytes: &[u8],
        media_type: &str,
    ) -> Result<&'static str, String> {
        let detected = self.validate_image_bytes(bytes)?;
        if detected != media_type {
            return Err(format!(
                "Image media type mismatch: declared {media_type}, detected {detected}"
            ));
        }
        Ok(detected)
    }
}

#[cfg(test)]
pub fn max_base64_image_len() -> usize {
    MAX_IMAGE_BYTES.div_ceil(3) * 4
}

/// Detect MIME type from magic bytes.
pub fn detect_image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() < 4 {
        return None;
    }
    if bytes[0] == 0xFF && bytes[1] == 0xD8 && bytes[2] == 0xFF {
        return Some("image/jpeg");
    }
    if bytes[0] == 0x89 && bytes[1] == 0x50 && bytes[2] == 0x4E && bytes[3] == 0x47 {
        return Some("image/png");
    }
    if bytes[0] == 0x47 && bytes[1] == 0x49 && bytes[2] == 0x46 && bytes[3] == 0x38 {
        return Some("image/gif");
    }
    if bytes.len() >= 12
        && bytes[0] == 0x52
        && bytes[1] == 0x49
        && bytes[2] == 0x46
        && bytes[3] == 0x46
        && bytes[8] == 0x57
        && bytes[9] == 0x45
        && bytes[10] == 0x42
        && bytes[11] == 0x50
    {
        return Some("image/webp");
    }
    None
}

pub(crate) fn filter_agent_skills_for_query(
    skills: Vec<SkillEntry>,
    query: Option<&str>,
    limit: Option<usize>,
) -> Vec<SkillEntry> {
    let needle = query.unwrap_or_default().trim().to_lowercase();
    let max_results = limit.unwrap_or(usize::MAX);
    skills
        .into_iter()
        .filter(|skill| {
            needle.is_empty()
                || skill.name.to_lowercase().contains(&needle)
                || skill.description.to_lowercase().contains(&needle)
        })
        .take(max_results)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_agent_skills_filters_by_query_and_limit() {
        let skills = vec![
            SkillEntry {
                name: "review".to_string(),
                description: "Review code changes".to_string(),
                scope: "project".to_string(),
            },
            SkillEntry {
                name: "docs".to_string(),
                description: "Write documentation".to_string(),
                scope: "personal".to_string(),
            },
            SkillEntry {
                name: "diagram".to_string(),
                description: "Document architecture diagrams".to_string(),
                scope: "project".to_string(),
            },
        ];

        let result = filter_agent_skills_for_query(skills.clone(), Some("doc"), Some(1));

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "docs");

        let result = filter_agent_skills_for_query(skills, Some("review"), Some(20));

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "review");
    }
}
mod context_restore_policy;
mod recovery_fact;
mod stale_policy;
mod status_classification;

pub use context_restore_policy::{
    context_restore_completion_is_settled, decide_context_restore_completion,
    decide_context_restore_preparation, ContextCarryChange, ContextRestoreCompletionCommand,
    ContextRestoreCompletionDecision, ContextRestoreCompletionFacts,
    ContextRestoreCompletionRejection, ContextRestorePreparationDecision,
};
pub use recovery_fact::{
    admit_backend_recovery_sensitive_operation, backend_recovery_may_be_incomplete,
    classify_backend_recovery, classify_recovery_fact, decide_backend_recovery_readback,
    decide_recovery_publication, decide_recovery_publication_commit,
    project_durable_backend_recovery, BackendRecoveryObservation,
    BackendRecoveryOperationRejection, BackendRecoveryReadbackDecision, DurableBackendRecovery,
    RecoveryPublicationCommitDecision, RecoveryPublicationCommitFacts,
    RecoveryPublicationCommitRejection, RecoveryPublicationDecision,
    RecoveryPublicationListDecision,
};
pub use stale_policy::{
    effective_stale_timeout, has_in_flight_tool_use, provider_startup_retries,
    provider_startup_timeout, recovery_cap_reached, remaining_until_stale, stale_timeout_from_secs,
    stale_watchdog_should_continue_waiting, stall_cap_reached, turn_is_stale,
};
#[cfg(test)]
pub use stale_policy::{MAX_STALL_RECOVERY_ATTEMPTS, MAX_STALL_SIGNALS};
#[cfg(test)]
pub use status_classification::{
    backend_selection_change_is_admitted, BackendSelectionChangeFacts,
};
pub use status_classification::{
    backend_selection_is_presented_as_changeable, classify_resume_outcome,
    classify_session_activity, project_runtime_turn_phase, SessionActivity,
};
