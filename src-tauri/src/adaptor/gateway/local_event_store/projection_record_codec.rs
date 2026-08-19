//! Gateway-owned Stored V1 codecs for local-state projection rows.
//!
//! Repository ports exchange closed semantic records.  JSON schema labels,
//! raw payload preservation, row namespaces and the 16 MiB stored-record
//! bound stay on this side of the boundary.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::domain::local_event::{
    AgentSessionProviderRecord, LocalStateMutation, ProviderHookHealthProjectionRecord,
    ProviderSessionOwnershipProjectionRecord, RevisionGuard, SessionProjectionRecord,
};

use super::state_record_codec::{StoredOperationReceiptV1, StoredOperationStatusV1};

pub(crate) const PROJECTION_RECORD_MAX_BYTES: usize = 16 * 1024 * 1024;
pub(crate) const PROVIDER_SESSION_OWNERSHIP_STORAGE_PREFIX: &str = "provider-session-ownership:";
pub(crate) const PROVIDER_HOOK_HEALTH_STORAGE_PREFIX: &str = "provider-hook-health:";

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

fn provider_record_label(provider: AgentSessionProviderRecord) -> &'static str {
    match provider {
        AgentSessionProviderRecord::Claude => "claude",
        AgentSessionProviderRecord::Codex => "codex",
    }
}

fn decode_provider_record(value: &str) -> Result<AgentSessionProviderRecord, String> {
    match value {
        "claude" => Ok(AgentSessionProviderRecord::Claude),
        "codex" => Ok(AgentSessionProviderRecord::Codex),
        _ => Err("provider Session ownership provider is invalid".to_string()),
    }
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
        SessionProjectionRecord::ProviderSessionOwnership(projection) => {
            encode_provider_session_ownership_projection_record_v1(projection)?
        }
        SessionProjectionRecord::ProviderHookHealth(projection) => {
            encode_provider_hook_health_projection_record_v1(projection)?
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
    } else {
        return Err("legacy Agent Session projection is unsupported".to_string());
    };
    validate_session_projection_key(session_id, &record)?;
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

fn validate_session_projection_key(
    session_id: &str,
    record: &SessionProjectionRecord,
) -> Result<(), String> {
    match record {
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
        _ => Err("projection row identity does not match its semantic record".to_string()),
    }
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
        decode_provider_hook_health_projection_record_v1,
        encode_provider_hook_health_projection_record_v1, merge_additive_payload,
    };
    use crate::domain::local_event::{
        AgentSessionProviderRecord, ProviderHookHealthProjectionRecord,
    };

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
    fn test_provider_hook_health_projection_session_start後の配送失敗warningを保持する() {
        let record = ProviderHookHealthProjectionRecord {
            provider: AgentSessionProviderRecord::Claude,
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
