//! Gateway-owned Stored V1 codecs for local-state projection rows.
//!
//! Repository ports exchange closed semantic records.  JSON schema labels,
//! raw payload preservation, row namespaces and the 16 MiB stored-record
//! bound stay on this side of the boundary.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::adaptor::gateway::agent_session::session_storage::{
    decode_agent_content_blob_record_v1, decode_agent_message_projection_record_v1,
    decode_agent_session_projection_record_v1, encode_agent_content_blob_record_v1,
    encode_agent_message_projection_record_v1, encode_agent_session_projection_record_v1,
};
use crate::adaptor::gateway::workflow::execution_store::{
    decode_workflow_execution_projection_record_v1, decode_workflow_worktree_owner_record_v1,
    encode_workflow_execution_projection_record_v1, encode_workflow_worktree_owner_record_v1,
};
use crate::domain::local_event::{
    AgentContentBlobRecord, LocalStateMutation, MessageProjectionRecord,
    ProviderAgentSessionLifecycleRecord, ProviderAgentSessionOriginRecord,
    ProviderAgentSessionProjectionRecord, ProviderAgentSessionProviderRecord,
    ProviderHookHealthProjectionRecord, ProviderSessionOwnershipProjectionRecord, RevisionGuard,
    SessionProjectionRecord, WorkflowExecutionProjectionRecord,
};

use super::indexed_projection_codec::encode_workflow_execution_node_v1;
use super::state_record_codec::{StoredOperationReceiptV1, StoredOperationStatusV1};

pub(crate) const PROJECTION_RECORD_MAX_BYTES: usize = 16 * 1024 * 1024;
pub(crate) const PROVIDER_AGENT_SESSION_STORAGE_PREFIX: &str = "provider-agent-session:";
pub(crate) const PROVIDER_SESSION_OWNERSHIP_STORAGE_PREFIX: &str = "provider-session-ownership:";
pub(crate) const PROVIDER_HOOK_HEALTH_STORAGE_PREFIX: &str = "provider-hook-health:";

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredProviderAgentSessionProjectionV1 {
    schema: String,
    id: String,
    workspace_identity: String,
    worktree_path: String,
    provider: String,
    origin: StoredProviderAgentSessionOriginV1,
    lifecycle: String,
    provider_session_id: Option<String>,
    transcript_ref: Option<String>,
    initial_instruction_admitted: bool,
    #[serde(default)]
    last_exit_abnormal: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum StoredProviderAgentSessionOriginV1 {
    Standalone,
    WorkflowNode {
        workflow_execution_id: String,
        node_execution_id: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredProviderSessionOwnershipProjectionV1 {
    schema: String,
    provider: String,
    provider_session_id: String,
    agent_session_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredProviderHookHealthProjectionV1 {
    schema: String,
    provider: String,
    latest_launch_id: String,
    latest_launch_session_started: bool,
    warning_launch_id: Option<String>,
    warning_reason: Option<String>,
}

fn encode_provider_hook_health_projection_record_v1(
    record: &ProviderHookHealthProjectionRecord,
) -> Result<String, String> {
    validate_provider_hook_health_projection(record)?;
    serde_json::to_string(&StoredProviderHookHealthProjectionV1 {
        schema: "provider_hook_health_projection_v1".to_string(),
        provider: provider_record_label(record.provider).to_string(),
        latest_launch_id: record.latest_launch_id.clone(),
        latest_launch_session_started: record.latest_launch_session_started,
        warning_launch_id: record.warning_launch_id.clone(),
        warning_reason: record.warning_reason.clone(),
    })
    .map_err(|_| "Provider Hook health encode failed".to_string())
}

fn decode_provider_hook_health_projection_record_v1(
    raw: &str,
) -> Result<ProviderHookHealthProjectionRecord, String> {
    let stored: StoredProviderHookHealthProjectionV1 = serde_json::from_str(raw)
        .map_err(|_| "Provider Hook health projection is malformed".to_string())?;
    if stored.schema != "provider_hook_health_projection_v1" {
        return Err("Provider Hook health projection schema is unsupported".to_string());
    }
    let record = ProviderHookHealthProjectionRecord {
        provider: decode_provider_record(&stored.provider)?,
        latest_launch_id: stored.latest_launch_id,
        latest_launch_session_started: stored.latest_launch_session_started,
        warning_launch_id: stored.warning_launch_id,
        warning_reason: stored.warning_reason,
    };
    validate_provider_hook_health_projection(&record)?;
    Ok(record)
}

fn validate_provider_hook_health_projection(
    record: &ProviderHookHealthProjectionRecord,
) -> Result<(), String> {
    if record.latest_launch_id.trim().is_empty()
        || record.warning_launch_id.is_some() != record.warning_reason.is_some()
        || record
            .warning_launch_id
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
        || record
            .warning_reason
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
    {
        return Err("Provider Hook health projection is invalid".to_string());
    }
    Ok(())
}

fn encode_provider_session_ownership_projection_record_v1(
    record: &ProviderSessionOwnershipProjectionRecord,
) -> Result<String, String> {
    validate_provider_session_ownership_projection(record)?;
    serde_json::to_string(&StoredProviderSessionOwnershipProjectionV1 {
        schema: "provider_session_ownership_projection_v1".to_string(),
        provider: provider_record_label(record.provider).to_string(),
        provider_session_id: record.provider_session_id.clone(),
        agent_session_id: record.agent_session_id.clone(),
    })
    .map_err(|_| "provider Session ownership encode failed".to_string())
}

fn decode_provider_session_ownership_projection_record_v1(
    raw: &str,
) -> Result<ProviderSessionOwnershipProjectionRecord, String> {
    let stored: StoredProviderSessionOwnershipProjectionV1 = serde_json::from_str(raw)
        .map_err(|_| "provider Session ownership projection is malformed".to_string())?;
    if stored.schema != "provider_session_ownership_projection_v1" {
        return Err("provider Session ownership projection schema is unsupported".to_string());
    }
    let record = ProviderSessionOwnershipProjectionRecord {
        provider: decode_provider_record(&stored.provider)?,
        provider_session_id: stored.provider_session_id,
        agent_session_id: stored.agent_session_id,
    };
    validate_provider_session_ownership_projection(&record)?;
    Ok(record)
}

fn validate_provider_session_ownership_projection(
    record: &ProviderSessionOwnershipProjectionRecord,
) -> Result<(), String> {
    if record.provider_session_id.trim().is_empty()
        || record
            .agent_session_id
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
    {
        return Err("provider Session ownership projection is invalid".to_string());
    }
    Ok(())
}

fn provider_record_label(provider: ProviderAgentSessionProviderRecord) -> &'static str {
    match provider {
        ProviderAgentSessionProviderRecord::Claude => "claude",
        ProviderAgentSessionProviderRecord::Codex => "codex",
    }
}

fn decode_provider_record(value: &str) -> Result<ProviderAgentSessionProviderRecord, String> {
    match value {
        "claude" => Ok(ProviderAgentSessionProviderRecord::Claude),
        "codex" => Ok(ProviderAgentSessionProviderRecord::Codex),
        _ => Err("provider Session ownership provider is invalid".to_string()),
    }
}

fn encode_provider_agent_session_projection_record_v1(
    record: &ProviderAgentSessionProjectionRecord,
) -> Result<String, String> {
    validate_provider_agent_session_projection(record)?;
    let stored = StoredProviderAgentSessionProjectionV1 {
        schema: "provider_agent_session_projection_v1".to_string(),
        id: record.id.clone(),
        workspace_identity: record.workspace_identity.clone(),
        worktree_path: record.worktree_path.clone(),
        provider: provider_record_label(record.provider).to_string(),
        origin: match &record.origin {
            ProviderAgentSessionOriginRecord::Standalone => {
                StoredProviderAgentSessionOriginV1::Standalone
            }
            ProviderAgentSessionOriginRecord::WorkflowNode {
                workflow_execution_id,
                node_execution_id,
            } => StoredProviderAgentSessionOriginV1::WorkflowNode {
                workflow_execution_id: workflow_execution_id.clone(),
                node_execution_id: node_execution_id.clone(),
            },
        },
        lifecycle: match record.lifecycle {
            ProviderAgentSessionLifecycleRecord::Open => "open",
            ProviderAgentSessionLifecycleRecord::Paused => "paused",
            ProviderAgentSessionLifecycleRecord::Archived => "archived",
        }
        .to_string(),
        provider_session_id: record.provider_session_id.clone(),
        transcript_ref: record.transcript_ref.clone(),
        initial_instruction_admitted: record.initial_instruction_admitted,
        last_exit_abnormal: record.last_exit_abnormal,
    };
    serde_json::to_string(&stored).map_err(|_| "provider AgentSession encode failed".to_string())
}

fn decode_provider_agent_session_projection_record_v1(
    raw: &str,
) -> Result<ProviderAgentSessionProjectionRecord, String> {
    let stored: StoredProviderAgentSessionProjectionV1 = serde_json::from_str(raw)
        .map_err(|_| "provider AgentSession projection is malformed".to_string())?;
    if stored.schema != "provider_agent_session_projection_v1" {
        return Err("provider AgentSession projection schema is unsupported".to_string());
    }
    let record = ProviderAgentSessionProjectionRecord {
        id: stored.id,
        workspace_identity: stored.workspace_identity,
        worktree_path: stored.worktree_path,
        provider: decode_provider_record(&stored.provider)
            .map_err(|_| "provider AgentSession provider is invalid".to_string())?,
        origin: match stored.origin {
            StoredProviderAgentSessionOriginV1::Standalone => {
                ProviderAgentSessionOriginRecord::Standalone
            }
            StoredProviderAgentSessionOriginV1::WorkflowNode {
                workflow_execution_id,
                node_execution_id,
            } => ProviderAgentSessionOriginRecord::WorkflowNode {
                workflow_execution_id,
                node_execution_id,
            },
        },
        lifecycle: match stored.lifecycle.as_str() {
            "open" => ProviderAgentSessionLifecycleRecord::Open,
            "paused" => ProviderAgentSessionLifecycleRecord::Paused,
            "archived" => ProviderAgentSessionLifecycleRecord::Archived,
            _ => return Err("provider AgentSession lifecycle is invalid".to_string()),
        },
        provider_session_id: stored.provider_session_id,
        transcript_ref: stored.transcript_ref,
        initial_instruction_admitted: stored.initial_instruction_admitted,
        last_exit_abnormal: stored.last_exit_abnormal,
    };
    validate_provider_agent_session_projection(&record)?;
    Ok(record)
}

fn validate_provider_agent_session_projection(
    record: &ProviderAgentSessionProjectionRecord,
) -> Result<(), String> {
    if record.id.trim().is_empty()
        || record.workspace_identity.trim().is_empty()
        || record.worktree_path.trim().is_empty()
        || crate::domain::repository::normalize_repo_path(&record.workspace_identity)
            != record.workspace_identity
        || crate::domain::repository::normalize_repo_path(&record.worktree_path)
            != record.worktree_path
        || record
            .provider_session_id
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
        || record
            .transcript_ref
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
    {
        return Err("provider AgentSession projection is invalid".to_string());
    }
    if let ProviderAgentSessionOriginRecord::WorkflowNode {
        workflow_execution_id,
        node_execution_id,
    } = &record.origin
    {
        if workflow_execution_id.trim().is_empty() || node_execution_id.trim().is_empty() {
            return Err("provider AgentSession workflow ownership is invalid".to_string());
        }
    }
    Ok(())
}

pub(crate) fn canonical_mutation_identity_v1(
    mutation: &LocalStateMutation,
) -> Result<Vec<u8>, String> {
    fn field(bytes: &mut Vec<u8>, value: &[u8]) {
        bytes.extend_from_slice(&(value.len() as u64).to_be_bytes());
        bytes.extend_from_slice(value);
    }
    fn text(bytes: &mut Vec<u8>, value: &str) {
        field(bytes, value.as_bytes());
    }
    fn revision_guard(bytes: &mut Vec<u8>, guard: RevisionGuard) {
        match guard {
            RevisionGuard::Absent => bytes.push(0),
            RevisionGuard::Expected(revision) => {
                bytes.push(1);
                bytes.extend_from_slice(&revision.value().to_be_bytes());
            }
        }
    }

    let mut bytes = b"local_state_mutation_identity_v1".to_vec();
    match mutation {
        LocalStateMutation::SessionProjection(mutation) => {
            text(&mut bytes, "session_projection");
            text(&mut bytes, &mutation.session_id);
            text(
                &mut bytes,
                &encode_session_projection_record_v1(&mutation.projection)?,
            );
            revision_guard(&mut bytes, mutation.expected);
            bytes.extend_from_slice(&mutation.revision.value().to_be_bytes());
            Ok(bytes)
        }
        LocalStateMutation::MessageProjection(mutation) => {
            text(&mut bytes, "message_projection");
            text(&mut bytes, &mutation.session_id);
            text(&mut bytes, &mutation.message_id);
            text(
                &mut bytes,
                &encode_message_projection_record_v1(&mutation.projection)?,
            );
            revision_guard(&mut bytes, mutation.expected);
            bytes.extend_from_slice(&mutation.revision.value().to_be_bytes());
            Ok(bytes)
        }
        LocalStateMutation::WorkflowExecutionProjection(mutation) => {
            text(&mut bytes, "workflow_execution_projection");
            text(
                &mut bytes,
                &encode_workflow_execution_projection_record_v1(&mutation.projection)?,
            );
            revision_guard(&mut bytes, mutation.expected);
            bytes.extend_from_slice(&mutation.revision.value().to_be_bytes());
            Ok(bytes)
        }
        LocalStateMutation::WorkflowExecutionNodeProjection(mutation) => {
            text(&mut bytes, "workflow_execution_node_projection");
            text(&mut bytes, &mutation.execution_id);
            for node in &mutation.nodes {
                let (tree, detail) = encode_workflow_execution_node_v1(node)?;
                text(&mut bytes, &tree);
                text(&mut bytes, &detail);
            }
            revision_guard(&mut bytes, mutation.expected);
            bytes.extend_from_slice(&mutation.revision.value().to_be_bytes());
            Ok(bytes)
        }
        LocalStateMutation::OperationRecord(mutation) => {
            text(&mut bytes, "operation_record");
            text(&mut bytes, mutation.kind.label());
            text(&mut bytes, &mutation.operation_id);
            text(
                &mut bytes,
                &StoredOperationReceiptV1::encode_new(&mutation.receipt)
                    .map_err(|error| format!("operation receipt identity failed: {error:?}"))?,
            );
            text(
                &mut bytes,
                &StoredOperationStatusV1::encode_new(&mutation.latest_status)
                    .map_err(|error| format!("operation status identity failed: {error:?}"))?,
            );
            revision_guard(&mut bytes, mutation.expected);
            bytes.extend_from_slice(&mutation.revision.value().to_be_bytes());
            Ok(bytes)
        }
        _ => mutation.canonical_identity_v1().map_err(str::to_string),
    }
}

pub(crate) fn encode_session_projection_record_v1(
    record: &SessionProjectionRecord,
) -> Result<String, String> {
    let raw = match record {
        SessionProjectionRecord::AgentSession(projection) => {
            encode_agent_session_projection_record_v1(projection)?
        }
        SessionProjectionRecord::ProviderAgentSession(projection) => {
            encode_provider_agent_session_projection_record_v1(projection)?
        }
        SessionProjectionRecord::ProviderSessionOwnership(projection) => {
            encode_provider_session_ownership_projection_record_v1(projection)?
        }
        SessionProjectionRecord::ProviderHookHealth(projection) => {
            encode_provider_hook_health_projection_record_v1(projection)?
        }
        SessionProjectionRecord::WorkflowExecution(projection) => {
            encode_workflow_execution_projection_record_v1(projection)?
        }
        SessionProjectionRecord::WorkflowWorktreeOwner(owner) => {
            encode_workflow_worktree_owner_record_v1(owner)?
        }
    };
    validate_stored_bound(&raw)?;
    Ok(raw)
}

pub(crate) fn decode_session_projection_record_v1(
    raw: &str,
    session_id: &str,
) -> Result<SessionProjectionRecord, String> {
    validate_stored_bound(raw)?;
    let record = if session_id.starts_with(PROVIDER_HOOK_HEALTH_STORAGE_PREFIX) {
        SessionProjectionRecord::ProviderHookHealth(
            decode_provider_hook_health_projection_record_v1(raw)?,
        )
    } else if session_id.starts_with(PROVIDER_SESSION_OWNERSHIP_STORAGE_PREFIX) {
        SessionProjectionRecord::ProviderSessionOwnership(
            decode_provider_session_ownership_projection_record_v1(raw)?,
        )
    } else if session_id.starts_with(PROVIDER_AGENT_SESSION_STORAGE_PREFIX) {
        SessionProjectionRecord::ProviderAgentSession(
            decode_provider_agent_session_projection_record_v1(raw)?,
        )
    } else if session_id.starts_with("workflow-worktree:") {
        SessionProjectionRecord::WorkflowWorktreeOwner(decode_workflow_worktree_owner_record_v1(
            raw,
        )?)
    } else if session_id.starts_with("workflow:") {
        SessionProjectionRecord::WorkflowExecution(decode_workflow_execution_projection_record_v1(
            raw,
        )?)
    } else {
        SessionProjectionRecord::AgentSession(Box::new(decode_agent_session_projection_record_v1(
            raw,
        )?))
    };
    validate_session_projection_key(session_id, &record)?;
    Ok(record)
}

pub(crate) fn encode_message_projection_record_v1(
    record: &MessageProjectionRecord,
) -> Result<String, String> {
    let raw = match record {
        MessageProjectionRecord::AgentMessage(message) => {
            encode_agent_message_projection_record_v1(message)?
        }
        MessageProjectionRecord::AgentContentBlob(blob) => {
            encode_agent_content_blob_record_v1(blob)?
        }
    };
    validate_stored_bound(&raw)?;
    Ok(raw)
}

pub(crate) fn decode_message_projection_record_v1(
    raw: &str,
    session_id: &str,
    message_id: &str,
) -> Result<MessageProjectionRecord, String> {
    validate_stored_bound(raw)?;
    let record = if session_id.starts_with("blob:") {
        MessageProjectionRecord::AgentContentBlob(decode_agent_content_blob_record_v1(raw)?)
    } else {
        MessageProjectionRecord::AgentMessage(decode_agent_message_projection_record_v1(raw)?)
    };
    validate_message_projection_key(session_id, message_id, &record)?;
    Ok(record)
}

pub(crate) fn encode_session_projection_update_v1(
    previous_raw: Option<&str>,
    record: &SessionProjectionRecord,
    session_id: &str,
) -> Result<String, String> {
    validate_session_projection_key(session_id, record)?;
    let canonical = encode_session_projection_record_v1(record)?;
    let Some(previous_raw) = previous_raw else {
        return Ok(canonical);
    };
    let previous = decode_session_projection_record_v1(previous_raw, session_id)?;
    if previous == *record {
        return Ok(previous_raw.to_string());
    }
    let previous_canonical = encode_session_projection_record_v1(&previous)?;
    merge_additive_payload(previous_raw, &previous_canonical, &canonical)
}

pub(crate) fn encode_message_projection_update_v1(
    previous_raw: Option<&str>,
    record: &MessageProjectionRecord,
    session_id: &str,
    message_id: &str,
) -> Result<String, String> {
    validate_message_projection_key(session_id, message_id, record)?;
    let canonical = encode_message_projection_record_v1(record)?;
    let Some(previous_raw) = previous_raw else {
        return Ok(canonical);
    };
    let previous = decode_message_projection_record_v1(previous_raw, session_id, message_id)?;
    if previous == *record {
        return Ok(previous_raw.to_string());
    }
    let previous_canonical = encode_message_projection_record_v1(&previous)?;
    merge_additive_payload(previous_raw, &previous_canonical, &canonical)
}

fn validate_session_projection_key(
    session_id: &str,
    record: &SessionProjectionRecord,
) -> Result<(), String> {
    match record {
        SessionProjectionRecord::AgentSession(projection)
            if !session_id.starts_with("workflow:")
                && !session_id.starts_with("workflow-worktree:")
                && !session_id.starts_with(PROVIDER_AGENT_SESSION_STORAGE_PREFIX)
                && !session_id.starts_with(PROVIDER_SESSION_OWNERSHIP_STORAGE_PREFIX)
                && !session_id.starts_with(PROVIDER_HOOK_HEALTH_STORAGE_PREFIX)
                && projection.meta.id == session_id =>
        {
            Ok(())
        }
        SessionProjectionRecord::ProviderAgentSession(projection)
            if session_id
                .strip_prefix(PROVIDER_AGENT_SESSION_STORAGE_PREFIX)
                .is_some_and(|id| id == projection.id) =>
        {
            Ok(())
        }
        SessionProjectionRecord::ProviderSessionOwnership(projection)
            if session_id == provider_session_ownership_storage_key(projection) =>
        {
            Ok(())
        }
        SessionProjectionRecord::ProviderHookHealth(projection)
            if session_id
                == format!(
                    "{PROVIDER_HOOK_HEALTH_STORAGE_PREFIX}{}",
                    provider_record_label(projection.provider)
                ) =>
        {
            Ok(())
        }
        SessionProjectionRecord::WorkflowExecution(WorkflowExecutionProjectionRecord::Present(
            execution,
        )) if session_id
            .strip_prefix("workflow:")
            .is_some_and(|id| id == execution.execution_id) =>
        {
            Ok(())
        }
        SessionProjectionRecord::WorkflowExecution(
            WorkflowExecutionProjectionRecord::Deleted { execution_id },
        ) if session_id
            .strip_prefix("workflow:")
            .is_some_and(|id| id == execution_id) =>
        {
            Ok(())
        }
        SessionProjectionRecord::WorkflowWorktreeOwner(owner)
            if session_id == workflow_worktree_storage_key(&owner.worktree_path)
                && !owner.execution_id.is_empty() =>
        {
            Ok(())
        }
        _ => Err("projection row identity does not match its semantic record".to_string()),
    }
}

fn validate_message_projection_key(
    session_id: &str,
    message_id: &str,
    record: &MessageProjectionRecord,
) -> Result<(), String> {
    match record {
        MessageProjectionRecord::AgentMessage(message)
            if !session_id.starts_with("blob:") && message.id == message_id =>
        {
            Ok(())
        }
        MessageProjectionRecord::AgentContentBlob(AgentContentBlobRecord::Attachment {
            id,
            ..
        }) if session_id.starts_with("blob:") && message_id == format!("attachment:{id}") => Ok(()),
        MessageProjectionRecord::AgentContentBlob(AgentContentBlobRecord::ToolOutput {
            id,
            ..
        }) if session_id.starts_with("blob:") && message_id == format!("tool_output:{id}") => {
            Ok(())
        }
        _ => Err("message projection row identity does not match its semantic record".to_string()),
    }
}

fn workflow_worktree_storage_key(worktree_path: &str) -> String {
    let digest = Sha256::digest(worktree_path.as_bytes());
    format!("workflow-worktree:{}", hex::encode(digest))
}

fn provider_session_ownership_storage_key(
    projection: &ProviderSessionOwnershipProjectionRecord,
) -> String {
    let digest = Sha256::digest(projection.provider_session_id.as_bytes());
    format!(
        "{PROVIDER_SESSION_OWNERSHIP_STORAGE_PREFIX}{}:{}",
        provider_record_label(projection.provider),
        hex::encode(digest)
    )
}

fn validate_stored_bound(raw: &str) -> Result<(), String> {
    if raw.is_empty() || raw.len() > PROJECTION_RECORD_MAX_BYTES {
        return Err("projection record is empty or exceeds its bound".to_string());
    }
    Ok(())
}

fn merge_additive_payload(
    previous_raw: &str,
    previous_canonical: &str,
    next_canonical: &str,
) -> Result<String, String> {
    let previous_raw: Value = serde_json::from_str(previous_raw)
        .map_err(|_| "stored projection payload is invalid".to_string())?;
    let previous_canonical: Value = serde_json::from_str(previous_canonical)
        .map_err(|_| "canonical projection payload is invalid".to_string())?;
    let mut next: Value = serde_json::from_str(next_canonical)
        .map_err(|_| "next projection payload is invalid".to_string())?;
    preserve_additive_fields(&previous_raw, &previous_canonical, &mut next);
    let merged =
        serde_json::to_string(&next).map_err(|_| "projection payload merge failed".to_string())?;
    validate_stored_bound(&merged)?;
    Ok(merged)
}

fn preserve_additive_fields(raw: &Value, known: &Value, next: &mut Value) {
    match (raw, known, next) {
        (Value::Object(raw), Value::Object(known), Value::Object(next)) => {
            for (key, raw_value) in raw {
                match (known.get(key), next.get_mut(key)) {
                    (None, None) => {
                        next.insert(key.clone(), raw_value.clone());
                    }
                    (Some(known_value), Some(next_value)) => {
                        preserve_additive_fields(raw_value, known_value, next_value);
                    }
                    _ => {}
                }
            }
        }
        (Value::Array(raw), Value::Array(known), Value::Array(next)) => {
            let known_keys = unique_array_keys(known);
            let next_keys = unique_array_keys(next);
            for (next_index, next_value) in next.iter_mut().enumerate() {
                let previous_index = known_keys.as_ref().zip(next_keys.as_ref()).and_then(
                    |(known_keys, next_keys)| {
                        known_keys
                            .iter()
                            .position(|candidate| candidate == &next_keys[next_index])
                    },
                );
                if let Some(previous_index) = previous_index {
                    if let (Some(raw_value), Some(known_value)) =
                        (raw.get(previous_index), known.get(previous_index))
                    {
                        preserve_additive_fields(raw_value, known_value, next_value);
                    }
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
#[path = "agent_session_lifecycle_projection_codec_test.rs"]
mod agent_session_lifecycle_projection_codec_tests;

fn unique_array_keys(values: &[Value]) -> Option<Vec<String>> {
    let keys = values
        .iter()
        .map(semantic_array_key)
        .collect::<Option<Vec<_>>>()?;
    let unique = keys.iter().collect::<std::collections::HashSet<_>>();
    (unique.len() == keys.len()).then_some(keys)
}

fn semantic_array_key(value: &Value) -> Option<String> {
    let object = value.as_object()?;
    let mut identity = String::new();
    for key in [
        "queue_item_id",
        "action_id",
        "obligation_id",
        "request_id",
        "message_id",
        "tool_use_id",
        "recovery_id",
        "turn_id",
        "id",
        "kind",
        "type",
    ] {
        let Some(value) = object.get(key) else {
            continue;
        };
        if let Some(value) = value.as_str() {
            identity.push_str(key);
            identity.push('\0');
            identity.push_str(value);
            identity.push('\0');
        } else if let Some(value) = value.as_u64() {
            identity.push_str(key);
            identity.push('\0');
            identity.push_str(&value.to_string());
            identity.push('\0');
        }
    }
    (!identity.is_empty()).then_some(identity)
}

#[cfg(test)]
mod tests {
    use super::{
        canonical_mutation_identity_v1, decode_message_projection_record_v1,
        decode_provider_hook_health_projection_record_v1, decode_session_projection_record_v1,
        encode_message_projection_record_v1, encode_provider_hook_health_projection_record_v1,
        encode_session_projection_record_v1, encode_session_projection_update_v1,
        merge_additive_payload,
    };
    use crate::adaptor::gateway::agent_session::session_storage::{
        encode_agent_message_projection_record_v1, AgentSessionProjectionCodecV1,
    };
    use crate::domain::local_event::{
        AgentMessageProjectionRecord, AgentMessageRoleRecord, LocalStateMutation,
        MessageProjectionRecord, OperationKind, OperationReceiptRecord, OperationRecordMutation,
        OperationStatusRecord, OperationStatusValue, ProviderAgentSessionProviderRecord,
        ProviderHookHealthProjectionRecord, RecordAuthentication, Revision, RevisionGuard,
        SessionProjectionRecord,
    };
    use crate::usecase::agent_session::session::{
        AgentSessionProjectionCodec, CanonicalAgentSessionProjection, CanonicalQueuedSend,
        SessionMeta, SessionState,
    };

    fn agent_session_projection(session_id: &str) -> SessionProjectionRecord {
        let mut session = crate::usecase::agent_session::session::build_new_session_with_id(
            session_id.to_string(),
            "/tmp/f06-projection-codec",
            Some("codex".to_string()),
            crate::domain::agent_session::PermissionMode::Ask,
            Some("gpt-5.6-sol".to_string()),
            false,
            false,
            None,
        );
        session.state = SessionState::Idle;
        AgentSessionProjectionCodecV1
            .encode(&CanonicalAgentSessionProjection {
                meta: SessionMeta::from_session(&session),
                title: Some("before".to_string()),
                messages: Vec::new(),
                reducer_events: Vec::new(),
                queue_paused_at: None,
                latest_token_usage: None,
                pending_send_queue: vec![CanonicalQueuedSend {
                    queue_item_id: "queue-1".to_string(),
                    human_message_id: "human-1".to_string(),
                    reserved_turn_id: "turn-1".to_string(),
                    input_ref: "input-before".to_string(),
                }],
            })
            .expect("semantic agent-session fixture")
    }

    fn agent_message_projection(message_id: &str) -> MessageProjectionRecord {
        MessageProjectionRecord::AgentMessage(AgentMessageProjectionRecord {
            id: message_id.to_string(),
            role: AgentMessageRoleRecord::Agent,
            content: "hello".to_string(),
            thinking: None,
            activities: None,
            parts: None,
            streaming_final_seq: 7,
            timestamp_bits: 1.5_f64.to_bits(),
            mentions: None,
        })
    }

    #[test]
    fn operation_record_mutation_has_gateway_owned_canonical_identity() {
        let mutation = LocalStateMutation::OperationRecord(OperationRecordMutation {
            kind: OperationKind::Send,
            operation_id: "send-atomic-queue".to_string(),
            receipt: OperationReceiptRecord::Send {
                operation_id: "send-atomic-queue".to_string(),
                session_id: "session-1".to_string(),
                input_ref: "input-1".to_string(),
                disposition: crate::domain::agent_session::events::SendDisposition::Queued {
                    queue_item_id: "queue-1".to_string(),
                },
                authentication: RecordAuthentication {
                    principal_mac: [1; 32],
                    binding_hmac: [2; 32],
                },
            },
            latest_status: OperationStatusRecord {
                kind: OperationKind::Send,
                value: OperationStatusValue::ProviderStartReserved {
                    obligation_id: "send-atomic-queue.exec".to_string(),
                },
            },
            expected: RevisionGuard::Expected(Revision::new(0).unwrap()),
            revision: Revision::new(1).unwrap(),
        });

        let identity =
            canonical_mutation_identity_v1(&mutation).expect("operation mutation identity");
        assert_eq!(
            canonical_mutation_identity_v1(&mutation).unwrap(),
            identity,
            "the same guarded semantic mutation must be deterministic"
        );
        assert_ne!(
            canonical_mutation_identity_v1(&LocalStateMutation::OperationRecord(
                OperationRecordMutation {
                    latest_status: OperationStatusRecord {
                        kind: OperationKind::Send,
                        value: OperationStatusValue::Running {
                            turn_id: "1".to_string(),
                        },
                    },
                    ..match mutation {
                        LocalStateMutation::OperationRecord(operation) => operation,
                        _ => unreachable!(),
                    }
                }
            ))
            .unwrap(),
            identity,
            "status changes must change the atomic batch identity"
        );
    }

    #[test]
    fn nested_additive_fields_follow_a_unique_semantic_array_identity() {
        let merged = merge_additive_payload(
            r#"{"items":[{"id":"a","known":"old","future":{"flag":true}}]}"#,
            r#"{"items":[{"id":"a","known":"old"}]}"#,
            r#"{"items":[{"id":"a","known":"new"}]}"#,
        )
        .expect("merge");
        let merged: serde_json::Value = serde_json::from_str(&merged).expect("JSON");
        assert_eq!(merged["items"][0]["known"], "new");
        assert_eq!(merged["items"][0]["future"]["flag"], true);
    }

    #[test]
    fn duplicate_kind_keys_never_graft_additive_fields_between_array_entries() {
        let merged = merge_additive_payload(
            r#"{"events":[{"kind":"text","content":"one","future":"first"},{"kind":"text","content":"two","future":"second"}]}"#,
            r#"{"events":[{"kind":"text","content":"one"},{"kind":"text","content":"two"}]}"#,
            r#"{"events":[{"kind":"text","content":"two changed"},{"kind":"text","content":"one changed"}]}"#,
        )
        .expect("merge");
        let merged: serde_json::Value = serde_json::from_str(&merged).expect("JSON");
        assert!(merged["events"][0].get("future").is_none());
        assert!(merged["events"][1].get("future").is_none());
    }

    #[test]
    fn agent_message_stored_v1_bytes_are_stable() {
        let record = agent_message_projection("message-1");
        let MessageProjectionRecord::AgentMessage(message) = &record else {
            unreachable!();
        };
        let expected = concat!(
            r#"{"id":"message-1","role":"agent","content":"hello","#,
            r#""streamingFinalSeq":7,"timestamp":1.5}"#
        );
        assert_eq!(
            encode_agent_message_projection_record_v1(message).expect("gateway encode"),
            expected
        );
        assert_eq!(
            encode_message_projection_record_v1(&record).expect("dispatch encode"),
            expected
        );
        assert_eq!(
            decode_message_projection_record_v1(expected, "session-1", "message-1")
                .expect("gateway decode"),
            record
        );
    }

    #[test]
    fn session_update_preserves_top_nested_and_unique_array_additions() {
        let record = agent_session_projection("session-1");
        let canonical =
            encode_session_projection_record_v1(&record).expect("canonical Stored V1 projection");
        let mut raw: serde_json::Value = serde_json::from_str(&canonical).expect("canonical JSON");
        raw.as_object_mut()
            .expect("projection object")
            .insert("future_top".to_string(), serde_json::json!({"flag": true}));
        raw["meta"]
            .as_object_mut()
            .expect("meta object")
            .insert("futureMeta".to_string(), serde_json::json!("kept"));
        raw["pending_send_queue"][0]
            .as_object_mut()
            .expect("queued-send object")
            .insert("future_queue".to_string(), serde_json::json!([1, 2, 3]));
        let raw = serde_json::to_string(&raw).expect("additive Stored V1");

        let decoded =
            decode_session_projection_record_v1(&raw, "session-1").expect("additive decode");
        assert_eq!(
            encode_session_projection_update_v1(Some(&raw), &decoded, "session-1")
                .expect("unchanged update"),
            raw,
            "semantically unchanged projections preserve their exact stored bytes"
        );

        let mut changed = decoded;
        let SessionProjectionRecord::AgentSession(agent_session) = &mut changed else {
            unreachable!();
        };
        agent_session.title = Some("after".to_string());
        agent_session.pending_send_queue[0].input_ref = "input-after".to_string();
        let merged = encode_session_projection_update_v1(Some(&raw), &changed, "session-1")
            .expect("changed update");
        let merged: serde_json::Value = serde_json::from_str(&merged).expect("merged JSON");
        assert_eq!(merged["title"], "after");
        assert_eq!(merged["pending_send_queue"][0]["input_ref"], "input-after");
        assert_eq!(merged["future_top"]["flag"], true);
        assert_eq!(merged["meta"]["futureMeta"], "kept");
        assert_eq!(
            merged["pending_send_queue"][0]["future_queue"],
            serde_json::json!([1, 2, 3])
        );
    }

    #[test]
    fn unknown_required_discriminators_fail_closed() {
        let message = encode_message_projection_record_v1(&agent_message_projection("message-1"))
            .expect("message encode")
            .replace(r#""role":"agent""#, r#""role":"future_role""#);
        assert!(decode_message_projection_record_v1(&message, "session-1", "message-1").is_err());

        let session = encode_session_projection_record_v1(&agent_session_projection("session-1"))
            .expect("session encode")
            .replace(
                r#""schema":"agent_session_projection_v1""#,
                r#""schema":"agent_session_projection_v2""#,
            );
        assert!(decode_session_projection_record_v1(&session, "session-1").is_err());
    }

    #[test]
    fn test_provider_hook_health_projection_session_start後の配送失敗warningを保持する() {
        let record = ProviderHookHealthProjectionRecord {
            provider: ProviderAgentSessionProviderRecord::Claude,
            latest_launch_id: "launch-1".to_string(),
            latest_launch_session_started: true,
            warning_launch_id: Some("launch-1".to_string()),
            warning_reason: Some("local_api_unavailable".to_string()),
        };

        let encoded = encode_provider_hook_health_projection_record_v1(&record).unwrap();

        assert_eq!(
            decode_provider_hook_health_projection_record_v1(&encoded).unwrap(),
            record
        );
    }
}
