//! Gateway-owned codecs for the closed local-state record families.

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::domain::agent_session::entities::{
    InterruptReason, PermissionResponse, PermissionResponseDecision, TokenUsage, TurnResult,
    TurnStopReason,
};
use crate::domain::agent_session::events::{
    BackendSessionRecoveryReason, RecoveryActionKind, RecoveryResultClassification,
    SendDisposition, StopResolution, TurnTokenUsage,
};
use crate::domain::agent_session::value_objects::JsonPayload;
use crate::domain::local_event::record::*;
use crate::domain::local_event::{
    CommitOperationKind, OperationKind, QuitIntent, SessionOperationFailureKind, ShutdownPlanKey,
};
use crate::domain::workflow::{
    ExecutionInterruptionReason, ExecutionOrigin, ExecutionStatus, TokenUsage as WorkflowTokenUsage,
};

/// The persistence families whose JSON shape is owned by this gateway.
///
/// The SQL table/column is not a type discriminator: callers must select the
/// exact family represented by the mutation/query variant.  This prevents a
/// valid document from one table being accepted as another table's record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StoredRecordFamily {
    OperationReceipt,
    OperationStatus,
    Terminal,
    Obligation,
    RecoveryAction,
    RecoveryResult,
    ShutdownPlan,
    ShutdownTarget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StoredRecordCodecError {
    Malformed {
        family: StoredRecordFamily,
    },
    Incompatible {
        family: StoredRecordFamily,
        schema: String,
    },
    Integrity {
        family: StoredRecordFamily,
    },
    MissingReference {
        family: StoredRecordFamily,
        field: &'static str,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StoredClosedTagV1 {
    Accepted,
    Prepared,
    Pending,
    EffectReserved,
    Running,
    WaitingApproval,
    AwaitingProviderStart,
    AwaitingProviderResponse,
    Queued,
    ProviderStartReserved,
    ReconciliationRequired,
    OutcomeUnknown,
    Failed,
    FailedBeforeActivation,
    Completed,
    Interrupted,
    Terminal,
    Cancelled,
    Superseded,
    ExitPending,
    Exited,
    Preparing,
    Activated,
    ProviderEstablish,
    TurnExecution,
    QueuedSend,
    Permission,
    ProviderSession,
    BackendRecovery,
    RecoveryPublication,
    OperationBinding,
    PermissionResponse,
    ProviderInterrupt,
    SessionClose,
    QueuePause,
    AgentSession,
    WorkflowExecution,
    WorkflowNode,
    StartedTurn,
    Allow,
    Deny,
    Close,
    ArchiveOpen,
    ArchiveClosed,
    SwitchBackend,
    Restart,
    Exit,
}

macro_rules! semantic_dto {
    ($name:ident, $value:ty, $family:expr, $decode:ident, $encode:ident, $known:ident) => {
        #[derive(Debug, Clone, PartialEq)]
        pub(crate) struct $name {
            value: $value,
            raw: String,
        }

        impl $name {
            pub(crate) fn decode(raw: &str) -> Result<Self, StoredRecordCodecError> {
                let (object, _) = validated_object($family, raw)?;
                let value = $decode(&object)?;
                Ok(Self {
                    value,
                    raw: raw.to_string(),
                })
            }

            #[allow(dead_code)] // Uniform closed-codec API; immutable families do not compare values in production.
            pub(crate) fn value(&self) -> &$value {
                &self.value
            }

            pub(crate) fn into_value(self) -> $value {
                self.value
            }

            #[cfg(test)]
            pub(crate) fn raw(&self) -> &str {
                &self.raw
            }

            pub(crate) fn encode_new(value: &$value) -> Result<String, StoredRecordCodecError> {
                encode_and_validate($family, $encode(value)?)
            }

            /// Encode a CAS replacement while carrying forward every
            /// additive top-level member from the exact stored V1 document.
            /// Known semantic fields always come from `value`; an old field
            /// can therefore never override a required reference or tag.
            #[allow(dead_code)] // Uniform closed-codec API; immutable families never take a CAS replacement.
            pub(crate) fn encode_update(
                &self,
                value: &$value,
            ) -> Result<String, StoredRecordCodecError> {
                let encoded = Self::encode_new(value)?;
                let old_canonical = Self::encode_new(&self.value)?;
                let (_, schema) = validated_object($family, &self.raw)?;
                merge_additive_fields(
                    $family,
                    &self.raw,
                    &old_canonical,
                    &encoded,
                    $known(&schema),
                )
            }
        }
    };
}

semantic_dto!(
    StoredOperationReceiptV1,
    OperationReceiptRecord,
    StoredRecordFamily::OperationReceipt,
    decode_operation_receipt,
    encode_operation_receipt,
    known_operation_receipt_fields
);
semantic_dto!(
    StoredOperationStatusV1,
    OperationStatusRecord,
    StoredRecordFamily::OperationStatus,
    decode_operation_status,
    encode_operation_status,
    known_operation_status_fields
);
semantic_dto!(
    StoredTerminalV1,
    TerminalResultRecord,
    StoredRecordFamily::Terminal,
    decode_terminal,
    encode_terminal,
    known_terminal_fields
);
semantic_dto!(
    StoredObligationV1,
    ObligationRecord,
    StoredRecordFamily::Obligation,
    decode_obligation,
    encode_obligation,
    known_obligation_fields
);
semantic_dto!(
    StoredRecoveryActionV1,
    RecoveryAttemptRecord,
    StoredRecordFamily::RecoveryAction,
    decode_recovery_attempt,
    encode_recovery_attempt,
    known_recovery_attempt_fields
);
semantic_dto!(
    StoredRecoveryResultV1,
    RecoveryResultRecord,
    StoredRecordFamily::RecoveryResult,
    decode_recovery_result,
    encode_recovery_result,
    known_recovery_result_fields
);
semantic_dto!(
    StoredShutdownPlanV1,
    ShutdownPlanRecord,
    StoredRecordFamily::ShutdownPlan,
    decode_shutdown_plan,
    encode_shutdown_plan,
    known_shutdown_plan_fields
);
semantic_dto!(
    StoredShutdownTargetV1,
    ShutdownTargetRecord,
    StoredRecordFamily::ShutdownTarget,
    decode_shutdown_target,
    encode_shutdown_target,
    known_shutdown_target_fields
);
/// A decoded, version-one persistence DTO.  Every variant retains the exact
/// input bytes so additive fields survive read/export/rewrite unchanged.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum StoredStateRecordV1 {
    OperationReceipt(StoredOperationReceiptV1),
    OperationStatus(StoredOperationStatusV1),
    Terminal(StoredTerminalV1),
    Obligation(StoredObligationV1),
    RecoveryAction(StoredRecoveryActionV1),
    RecoveryResult(StoredRecoveryResultV1),
    ShutdownPlan(StoredShutdownPlanV1),
    ShutdownTarget(StoredShutdownTargetV1),
}

#[cfg(test)]
impl StoredStateRecordV1 {
    pub(crate) fn decode(
        family: StoredRecordFamily,
        raw: &str,
    ) -> Result<Self, StoredRecordCodecError> {
        Ok(match family {
            StoredRecordFamily::OperationReceipt => {
                Self::OperationReceipt(StoredOperationReceiptV1::decode(raw)?)
            }
            StoredRecordFamily::OperationStatus => {
                Self::OperationStatus(StoredOperationStatusV1::decode(raw)?)
            }
            StoredRecordFamily::Terminal => Self::Terminal(StoredTerminalV1::decode(raw)?),
            StoredRecordFamily::Obligation => Self::Obligation(StoredObligationV1::decode(raw)?),
            StoredRecordFamily::RecoveryAction => {
                Self::RecoveryAction(StoredRecoveryActionV1::decode(raw)?)
            }
            StoredRecordFamily::RecoveryResult => {
                Self::RecoveryResult(StoredRecoveryResultV1::decode(raw)?)
            }
            StoredRecordFamily::ShutdownPlan => {
                Self::ShutdownPlan(StoredShutdownPlanV1::decode(raw)?)
            }
            StoredRecordFamily::ShutdownTarget => {
                Self::ShutdownTarget(StoredShutdownTargetV1::decode(raw)?)
            }
        })
    }

    pub(crate) fn encode(&self) -> &str {
        match self {
            Self::OperationReceipt(value) => value.raw(),
            Self::OperationStatus(value) => value.raw(),
            Self::Terminal(value) => value.raw(),
            Self::Obligation(value) => value.raw(),
            Self::RecoveryAction(value) => value.raw(),
            Self::RecoveryResult(value) => value.raw(),
            Self::ShutdownPlan(value) => value.raw(),
            Self::ShutdownTarget(value) => value.raw(),
        }
    }
}

fn validated_object(
    family: StoredRecordFamily,
    raw: &str,
) -> Result<(Map<String, Value>, String), StoredRecordCodecError> {
    let value: Value =
        serde_json::from_str(raw).map_err(|_| StoredRecordCodecError::Malformed { family })?;
    let object = value
        .as_object()
        .cloned()
        .ok_or(StoredRecordCodecError::Malformed { family })?;
    let schema = required_text(&object, family, "schema")?.to_string();
    if !allowed_schemas(family).contains(&schema.as_str()) {
        return Err(StoredRecordCodecError::Incompatible { family, schema });
    }
    validate_required_shape(family, &schema, &object)?;
    validate_typed_metadata(family, &schema, &object)?;
    Ok((object, schema))
}

fn encode_and_validate(
    family: StoredRecordFamily,
    value: Value,
) -> Result<String, StoredRecordCodecError> {
    let raw =
        serde_json::to_string(&value).map_err(|_| StoredRecordCodecError::Malformed { family })?;
    validated_object(family, &raw)?;
    Ok(raw)
}

/// Merge additive members from the exact old document at every object depth.
///
/// `old_canonical` is the gateway's lossless encoding of the decoded closed
/// value.  A member present in `old` but absent at the same location in
/// `old_canonical` is therefore an additive persistence member, rather than a
/// domain field.  Its raw JSON value is spliced into the new encoding without
/// a parse/re-serialize cycle.  Known fields always come from `new`, including
/// fields removed by a variant transition.
fn merge_additive_fields(
    family: StoredRecordFamily,
    old: &str,
    old_canonical: &str,
    new: &str,
    known_top_level: &[&str],
) -> Result<String, StoredRecordCodecError> {
    let merged = merge_additive_value(family, old, old_canonical, new, Some(known_top_level))?;
    validated_object(family, &merged)?;
    Ok(merged)
}

fn merge_additive_value(
    family: StoredRecordFamily,
    old: &str,
    old_canonical: &str,
    new: &str,
    known_fields: Option<&[&str]>,
) -> Result<String, StoredRecordCodecError> {
    use serde_json::value::RawValue;
    use std::collections::BTreeMap;

    let first = |raw: &str| raw.bytes().find(|byte| !byte.is_ascii_whitespace());
    match (first(old), first(old_canonical), first(new)) {
        (Some(b'{'), Some(b'{'), Some(b'{')) => {
            let old_fields: BTreeMap<String, Box<RawValue>> = serde_json::from_str(old)
                .map_err(|_| StoredRecordCodecError::Malformed { family })?;
            let canonical_fields: BTreeMap<String, Box<RawValue>> =
                serde_json::from_str(old_canonical)
                    .map_err(|_| StoredRecordCodecError::Malformed { family })?;
            let new_fields: BTreeMap<String, Box<RawValue>> = serde_json::from_str(new)
                .map_err(|_| StoredRecordCodecError::Malformed { family })?;
            let mut output = Vec::with_capacity(new_fields.len() + old_fields.len());

            for (key, new_value) in new_fields {
                let value = match (old_fields.get(&key), canonical_fields.get(&key)) {
                    (Some(old_value), Some(canonical_value)) => merge_additive_value(
                        family,
                        old_value.get(),
                        canonical_value.get(),
                        new_value.get(),
                        None,
                    )?,
                    _ => new_value.get().to_string(),
                };
                output.push((key, value));
            }

            for (key, old_value) in old_fields {
                let is_known = canonical_fields.contains_key(&key)
                    || known_fields.is_some_and(|fields| fields.contains(&key.as_str()));
                if !is_known && !output.iter().any(|(saved, _)| saved == &key) {
                    output.push((key, old_value.get().to_string()));
                }
            }

            let mut merged = String::from("{");
            for (index, (key, value)) in output.into_iter().enumerate() {
                if index != 0 {
                    merged.push(',');
                }
                merged.push_str(
                    &serde_json::to_string(&key)
                        .map_err(|_| StoredRecordCodecError::Malformed { family })?,
                );
                merged.push(':');
                merged.push_str(&value);
            }
            merged.push('}');
            Ok(merged)
        }
        (Some(b'['), Some(b'['), Some(b'[')) => {
            let old_items: Vec<Box<RawValue>> = serde_json::from_str(old)
                .map_err(|_| StoredRecordCodecError::Malformed { family })?;
            let canonical_items: Vec<Box<RawValue>> = serde_json::from_str(old_canonical)
                .map_err(|_| StoredRecordCodecError::Malformed { family })?;
            let new_items: Vec<Box<RawValue>> = serde_json::from_str(new)
                .map_err(|_| StoredRecordCodecError::Malformed { family })?;
            let can_align = old_items.len() == canonical_items.len()
                && canonical_items.len() == new_items.len();
            let mut merged = String::from("[");
            for (index, new_item) in new_items.iter().enumerate() {
                if index != 0 {
                    merged.push(',');
                }
                if can_align {
                    merged.push_str(&merge_additive_value(
                        family,
                        old_items[index].get(),
                        canonical_items[index].get(),
                        new_item.get(),
                        None,
                    )?);
                } else {
                    merged.push_str(new_item.get());
                }
            }
            merged.push(']');
            Ok(merged)
        }
        _ => Ok(new.to_string()),
    }
}

fn malformed<T>(family: StoredRecordFamily) -> Result<T, StoredRecordCodecError> {
    Err(StoredRecordCodecError::Malformed { family })
}

fn string_field(
    object: &Map<String, Value>,
    family: StoredRecordFamily,
    field: &'static str,
) -> Result<String, StoredRecordCodecError> {
    bounded_reference(required_text(object, family, field)?, family, field)
}

fn optional_string(object: &Map<String, Value>, field: &str) -> Option<String> {
    object
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn u64_field(
    object: &Map<String, Value>,
    family: StoredRecordFamily,
    field: &'static str,
) -> Result<u64, StoredRecordCodecError> {
    object
        .get(field)
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str().and_then(|raw| raw.parse().ok()))
        })
        .filter(|value| *value <= i64::MAX as u64)
        .ok_or(StoredRecordCodecError::MissingReference { family, field })
}

fn optional_u64(object: &Map<String, Value>, field: &str) -> Option<u64> {
    object.get(field).and_then(|value| {
        value
            .as_u64()
            .or_else(|| value.as_str().and_then(|raw| raw.parse().ok()))
    })
}

fn optional_i64(object: &Map<String, Value>, field: &str) -> Option<i64> {
    object.get(field).and_then(|value| {
        value
            .as_i64()
            .or_else(|| value.as_str().and_then(|raw| raw.parse().ok()))
    })
}

fn hash_field(
    object: &Map<String, Value>,
    family: StoredRecordFamily,
    field: &'static str,
) -> Result<[u8; 32], StoredRecordCodecError> {
    decode_hash32(family, required_text(object, family, field)?)
}

fn encode_authentication(authentication: &RecordAuthentication, object: &mut Map<String, Value>) {
    object.insert(
        "principal_mac".to_string(),
        Value::String(hex::encode(authentication.principal_mac)),
    );
    object.insert(
        "binding_hmac".to_string(),
        Value::String(hex::encode(authentication.binding_hmac)),
    );
}

fn decode_authentication(
    object: &Map<String, Value>,
    family: StoredRecordFamily,
) -> Result<RecordAuthentication, StoredRecordCodecError> {
    Ok(RecordAuthentication {
        principal_mac: hash_field(object, family, "principal_mac")?,
        binding_hmac: hash_field(object, family, "binding_hmac")?,
    })
}

fn quit_fields(intent: QuitIntent) -> (&'static str, i64) {
    match intent {
        QuitIntent::Exit { code } => ("exit", code),
        QuitIntent::Restart { code } => ("restart", code),
    }
}

fn decode_quit_intent(
    object: &Map<String, Value>,
    family: StoredRecordFamily,
) -> Result<QuitIntent, StoredRecordCodecError> {
    let code = required_i64(object, family, "exit_code")?;
    match required_text(object, family, "intent")? {
        "exit" => Ok(QuitIntent::Exit { code }),
        "restart" => Ok(QuitIntent::Restart { code }),
        other => Err(StoredRecordCodecError::Incompatible {
            family,
            schema: format!("quit_intent.{other}"),
        }),
    }
}

fn encode_send_disposition(value: &SendDisposition) -> Value {
    match value {
        SendDisposition::StartedTurn { turn_id } => {
            serde_json::json!({"type":"started_turn","turn_id":turn_id})
        }
        SendDisposition::Queued { queue_item_id } => {
            serde_json::json!({"type":"queued","queue_item_id":queue_item_id})
        }
    }
}

fn decode_send_disposition(
    value: &Value,
    family: StoredRecordFamily,
) -> Result<SendDisposition, StoredRecordCodecError> {
    let object = value
        .as_object()
        .ok_or(StoredRecordCodecError::Malformed { family })?;
    match required_text(object, family, "type")? {
        "started_turn" => Ok(SendDisposition::StartedTurn {
            turn_id: string_field(object, family, "turn_id")?,
        }),
        "queued" => Ok(SendDisposition::Queued {
            queue_item_id: string_field(object, family, "queue_item_id")?,
        }),
        other => Err(StoredRecordCodecError::Incompatible {
            family,
            schema: format!("send_disposition.{other}"),
        }),
    }
}

fn encode_operation_receipt(
    value: &OperationReceiptRecord,
) -> Result<Value, StoredRecordCodecError> {
    let family = StoredRecordFamily::OperationReceipt;
    let mut object = Map::new();
    match value {
        OperationReceiptRecord::Send {
            operation_id,
            session_id,
            input_ref,
            disposition,
            authentication,
        } => {
            object.insert("schema".into(), Value::String("send_receipt_v1".into()));
            object.insert("operation_id".into(), Value::String(operation_id.clone()));
            object.insert("session_id".into(), Value::String(session_id.clone()));
            object.insert("input_ref".into(), Value::String(input_ref.clone()));
            object.insert("disposition".into(), encode_send_disposition(disposition));
            encode_authentication(authentication, &mut object);
        }
        OperationReceiptRecord::PermissionResponse {
            operation_id,
            session_id,
            request_id,
            input_ref,
            authentication,
        } => {
            object.insert(
                "schema".into(),
                Value::String("permission_response_receipt_v1".into()),
            );
            object.insert("operation_id".into(), Value::String(operation_id.clone()));
            object.insert("session_id".into(), Value::String(session_id.clone()));
            object.insert("request_id".into(), Value::String(request_id.clone()));
            object.insert("input_ref".into(), Value::String(input_ref.clone()));
            encode_authentication(authentication, &mut object);
        }
        OperationReceiptRecord::Stop {
            operation_id,
            session_id,
            turn_id,
            accepted_revision,
            authentication,
        } => {
            object.insert("schema".into(), Value::String("stop_receipt_v1".into()));
            object.insert("operation_id".into(), Value::String(operation_id.clone()));
            object.insert("session_id".into(), Value::String(session_id.clone()));
            object.insert("turn_id".into(), Value::String(turn_id.clone()));
            object.insert("accepted_revision".into(), Value::from(*accepted_revision));
            encode_authentication(authentication, &mut object);
        }
        OperationReceiptRecord::SessionLifecycle {
            operation_id,
            session_id,
            action,
            first_accepted_revision,
            commit_operation_kind,
            authentication,
        } => {
            object.insert("schema".into(), Value::String("slc_receipt_v1".into()));
            object.insert("operation_id".into(), Value::String(operation_id.clone()));
            object.insert("session_id".into(), Value::String(session_id.clone()));
            object.insert("action".into(), encode_lifecycle_action(action));
            object.insert(
                "first_accepted_revision".into(),
                Value::from(*first_accepted_revision),
            );
            object.insert(
                "commit_operation_kind".into(),
                Value::String(commit_operation_kind.label().to_string()),
            );
            encode_authentication(authentication, &mut object);
        }
        OperationReceiptRecord::ApplicationQuit {
            operation_id,
            plan,
            intent,
            t0_ms,
            deadline_ms,
            binding_hmac,
        } => {
            let (intent, code) = quit_fields(*intent);
            object.insert(
                "schema".into(),
                Value::String("application_quit_receipt_v1".into()),
            );
            object.insert("operation_id".into(), Value::String(operation_id.clone()));
            object.insert(
                "shutdown_id".into(),
                Value::String(plan.shutdown_id.clone()),
            );
            object.insert("intent".into(), Value::String(intent.into()));
            object.insert("exit_code".into(), Value::from(code));
            object.insert("t0_ms".into(), Value::from(*t0_ms));
            object.insert("deadline_ms".into(), Value::from(*deadline_ms));
            object.insert(
                "binding_hmac".into(),
                Value::String(hex::encode(binding_hmac)),
            );
        }
    }
    if object.is_empty() {
        return malformed(family);
    }
    Ok(Value::Object(object))
}

fn decode_operation_receipt(
    object: &Map<String, Value>,
) -> Result<OperationReceiptRecord, StoredRecordCodecError> {
    decode_operation_receipt_for_family(object, StoredRecordFamily::OperationReceipt)
}

fn decode_operation_receipt_for_family(
    object: &Map<String, Value>,
    family: StoredRecordFamily,
) -> Result<OperationReceiptRecord, StoredRecordCodecError> {
    match required_text(object, family, "schema")? {
        "send_receipt_v1" => Ok(OperationReceiptRecord::Send {
            operation_id: string_field(object, family, "operation_id")?,
            session_id: string_field(object, family, "session_id")?,
            input_ref: string_field(object, family, "input_ref")?,
            disposition: decode_send_disposition(
                object
                    .get("disposition")
                    .ok_or(StoredRecordCodecError::MissingReference {
                        family,
                        field: "disposition",
                    })?,
                family,
            )?,
            authentication: decode_authentication(object, family)?,
        }),
        "permission_response_receipt_v1" => Ok(OperationReceiptRecord::PermissionResponse {
            operation_id: string_field(object, family, "operation_id")?,
            session_id: string_field(object, family, "session_id")?,
            request_id: string_field(object, family, "request_id")?,
            input_ref: string_field(object, family, "input_ref")?,
            authentication: decode_authentication(object, family)?,
        }),
        "stop_receipt_v1" => Ok(OperationReceiptRecord::Stop {
            operation_id: string_field(object, family, "operation_id")?,
            session_id: string_field(object, family, "session_id")?,
            turn_id: string_field(object, family, "turn_id")?,
            accepted_revision: u64_field(object, family, "accepted_revision")?,
            authentication: decode_authentication(object, family)?,
        }),
        "slc_receipt_v1" => Ok(OperationReceiptRecord::SessionLifecycle {
            operation_id: string_field(object, family, "operation_id")?,
            session_id: string_field(object, family, "session_id")?,
            action: decode_lifecycle_action(
                object
                    .get("action")
                    .ok_or(StoredRecordCodecError::MissingReference {
                        family,
                        field: "action",
                    })?,
                family,
            )?,
            first_accepted_revision: required_i64(object, family, "first_accepted_revision")?,
            commit_operation_kind: decode_commit_operation_kind(
                object
                    .get("commit_operation_kind")
                    .and_then(Value::as_str)
                    .unwrap_or("session_lifecycle"),
                family,
            )?,
            authentication: decode_authentication(object, family)?,
        }),
        "application_quit_receipt_v1" => Ok(OperationReceiptRecord::ApplicationQuit {
            operation_id: string_field(object, family, "operation_id")?,
            plan: ShutdownPlanKey {
                shutdown_id: string_field(object, family, "shutdown_id")?,
            },
            intent: decode_quit_intent(object, family)?,
            t0_ms: required_i64(object, family, "t0_ms")?,
            deadline_ms: required_i64(object, family, "deadline_ms")?,
            binding_hmac: hash_field(object, family, "binding_hmac")?,
        }),
        schema => Err(StoredRecordCodecError::Incompatible {
            family,
            schema: schema.to_string(),
        }),
    }
}

fn encode_lifecycle_action(value: &SessionLifecycleRecordAction) -> Value {
    match value {
        SessionLifecycleRecordAction::Close => serde_json::json!({"type":"close"}),
        SessionLifecycleRecordAction::ArchiveOpen => serde_json::json!({"type":"archive_open"}),
        SessionLifecycleRecordAction::ArchiveClosed => {
            serde_json::json!({"type":"archive_closed"})
        }
        SessionLifecycleRecordAction::SwitchBackend { backend_id } => {
            serde_json::json!({"type":"switch_backend","backend_id":backend_id})
        }
    }
}

fn decode_lifecycle_action(
    value: &Value,
    family: StoredRecordFamily,
) -> Result<SessionLifecycleRecordAction, StoredRecordCodecError> {
    let object = value
        .as_object()
        .ok_or(StoredRecordCodecError::Malformed { family })?;
    match required_text(object, family, "type")? {
        "close" => Ok(SessionLifecycleRecordAction::Close),
        "archive_open" => Ok(SessionLifecycleRecordAction::ArchiveOpen),
        "archive_closed" => Ok(SessionLifecycleRecordAction::ArchiveClosed),
        "switch_backend" => Ok(SessionLifecycleRecordAction::SwitchBackend {
            backend_id: string_field(object, family, "backend_id")?,
        }),
        other => Err(StoredRecordCodecError::Incompatible {
            family,
            schema: format!("lifecycle_action.{other}"),
        }),
    }
}

fn decode_commit_operation_kind(
    raw: &str,
    family: StoredRecordFamily,
) -> Result<CommitOperationKind, StoredRecordCodecError> {
    match raw {
        "send" => Ok(CommitOperationKind::Send),
        "permission_response" => Ok(CommitOperationKind::PermissionResponse),
        "stop" => Ok(CommitOperationKind::Stop),
        "session_lifecycle" => Ok(CommitOperationKind::SessionLifecycle),
        "application_quit" => Ok(CommitOperationKind::ApplicationQuit),
        "recovery" => Ok(CommitOperationKind::Recovery),
        "user_mutation" => Ok(CommitOperationKind::UserMutation),
        "shutdown_target" => Ok(CommitOperationKind::ShutdownTarget),
        "operation_progress" => Ok(CommitOperationKind::OperationProgress),
        "projection" => Ok(CommitOperationKind::Projection),
        "workflow" => Ok(CommitOperationKind::Workflow),
        other => Err(StoredRecordCodecError::Incompatible {
            family,
            schema: format!("commit_operation_kind.{other}"),
        }),
    }
}

fn known_operation_receipt_fields(schema: &str) -> &'static [&'static str] {
    match schema {
        "send_receipt_v1" => &[
            "schema",
            "operation_id",
            "session_id",
            "input_ref",
            "disposition",
            "principal_mac",
            "binding_hmac",
        ],
        "permission_response_receipt_v1" => &[
            "schema",
            "operation_id",
            "session_id",
            "request_id",
            "input_ref",
            "principal_mac",
            "binding_hmac",
        ],
        "stop_receipt_v1" => &[
            "schema",
            "operation_id",
            "session_id",
            "turn_id",
            "accepted_revision",
            "principal_mac",
            "binding_hmac",
        ],
        "slc_receipt_v1" => &[
            "schema",
            "operation_id",
            "session_id",
            "action",
            "first_accepted_revision",
            "commit_operation_kind",
            "principal_mac",
            "binding_hmac",
        ],
        "application_quit_receipt_v1" => &[
            "schema",
            "operation_id",
            "shutdown_id",
            "intent",
            "exit_code",
            "t0_ms",
            "deadline_ms",
            "binding_hmac",
        ],
        _ => &["schema"],
    }
}

fn operation_status_schema(
    value: &OperationStatusRecord,
) -> Result<&'static str, StoredRecordCodecError> {
    match value.kind {
        OperationKind::Send => Ok("send_status_v1"),
        OperationKind::PermissionResponse => Ok("permission_response_status_v1"),
        OperationKind::Stop => Ok("stop_status_v1"),
        OperationKind::SessionLifecycle => Ok("slc_status_v1"),
        OperationKind::ApplicationQuit => Ok("application_quit_status_v1"),
    }
}

fn encode_operation_status(value: &OperationStatusRecord) -> Result<Value, StoredRecordCodecError> {
    let schema = operation_status_schema(value)?;
    let nested = encode_operation_status_value(&value.value);
    let field = if matches!(
        value.kind,
        OperationKind::Send | OperationKind::PermissionResponse
    ) {
        "status"
    } else {
        "state"
    };
    Ok(serde_json::json!({"schema":schema,field:nested}))
}

fn encode_operation_status_value(value: &OperationStatusValue) -> Value {
    match value {
        OperationStatusValue::Accepted => serde_json::json!({"type":"accepted"}),
        OperationStatusValue::AwaitingProviderStart {
            dependency_obligation_ids,
        } => {
            serde_json::json!({
                "type":"awaiting_provider_start",
                "dependency_obligation_ids":dependency_obligation_ids
            })
        }
        OperationStatusValue::AwaitingProviderResponse { obligation_id } => {
            serde_json::json!({"type":"awaiting_provider_response","obligation_id":obligation_id})
        }
        OperationStatusValue::Queued {
            queue_item_id,
            reserved_turn_id,
        } => {
            serde_json::json!({
                "type":"queued",
                "queue_item_id":queue_item_id,
                "reserved_turn_id":reserved_turn_id
            })
        }
        OperationStatusValue::ProviderStartReserved { obligation_id } => {
            serde_json::json!({"type":"provider_start_reserved","obligation_id":obligation_id})
        }
        OperationStatusValue::Running { turn_id } => {
            serde_json::json!({"type":"running","turn_id":turn_id})
        }
        OperationStatusValue::Completed => serde_json::json!({"type":"completed"}),
        OperationStatusValue::PermissionCompleted { decision } => serde_json::json!({
            "type":"completed",
            "decision":match decision {
                PermissionDecisionRecord::Allowed => "allow",
                PermissionDecisionRecord::Denied => "deny",
            }
        }),
        OperationStatusValue::StopCompleted { resolution } => serde_json::json!({
            "type":"completed",
            "resolution":match resolution {
                StopResolution::Succeeded => "succeeded",
                StopResolution::Superseded => "superseded",
            }
        }),
        OperationStatusValue::Preparing => serde_json::json!({"type":"preparing"}),
        OperationStatusValue::Activated => serde_json::json!({"type":"activated"}),
        OperationStatusValue::ExitPending => serde_json::json!({"type":"exit_pending"}),
        OperationStatusValue::Exited => serde_json::json!({"type":"exited"}),
        OperationStatusValue::OutcomeUnknown {
            operation_id,
            plan,
            activation_commit_id,
        } => serde_json::json!({
            "type":"outcome_unknown",
            "operation_id":operation_id,
            "shutdown_id":plan.shutdown_id,
            "activation_commit_id":activation_commit_id,
        }),
        OperationStatusValue::FailedBeforeActivation { failure } => serde_json::json!({
            "type":"failed_before_activation","failure":encode_failure(failure)
        }),
        OperationStatusValue::ReconciliationRequired { failure } => serde_json::json!({
            "type":"reconciliation_required","failure":encode_failure(failure)
        }),
        OperationStatusValue::Failed { failure } => {
            serde_json::json!({"type":"failed","failure":encode_failure(failure)})
        }
        OperationStatusValue::Terminal { result } => {
            serde_json::json!({"type":"terminal","result":encode_turn_result(result)})
        }
    }
}

fn decode_operation_status(
    object: &Map<String, Value>,
) -> Result<OperationStatusRecord, StoredRecordCodecError> {
    decode_operation_status_for_family(object, StoredRecordFamily::OperationStatus)
}

fn decode_operation_status_for_family(
    object: &Map<String, Value>,
    family: StoredRecordFamily,
) -> Result<OperationStatusRecord, StoredRecordCodecError> {
    let schema = required_text(object, family, "schema")?;
    let (kind, field) = match schema {
        "send_status_v1" => (OperationKind::Send, "status"),
        "permission_response_status_v1" => (OperationKind::PermissionResponse, "status"),
        "stop_status_v1" => (OperationKind::Stop, "state"),
        "slc_status_v1" => (OperationKind::SessionLifecycle, "state"),
        "application_quit_status_v1" => (OperationKind::ApplicationQuit, "state"),
        _ => {
            return Err(StoredRecordCodecError::Incompatible {
                family,
                schema: schema.to_string(),
            })
        }
    };
    let nested = object
        .get(field)
        .and_then(Value::as_object)
        .ok_or(StoredRecordCodecError::MissingReference { family, field })?;
    let tag = required_text(nested, family, "type")?;
    let value = match tag {
        "accepted" => OperationStatusValue::Accepted,
        "awaiting_provider_start" => OperationStatusValue::AwaitingProviderStart {
            dependency_obligation_ids: nested
                .get("dependency_obligation_ids")
                .and_then(Value::as_array)
                .ok_or(StoredRecordCodecError::MissingReference {
                    family,
                    field: "dependency_obligation_ids",
                })?
                .iter()
                .map(|entry| {
                    entry.as_str().map(str::to_string).ok_or(
                        StoredRecordCodecError::MissingReference {
                            family,
                            field: "dependency_obligation_ids",
                        },
                    )
                })
                .collect::<Result<Vec<_>, _>>()?,
        },
        "awaiting_provider_response" => OperationStatusValue::AwaitingProviderResponse {
            obligation_id: string_field(nested, family, "obligation_id")?,
        },
        "queued" => OperationStatusValue::Queued {
            queue_item_id: string_field(nested, family, "queue_item_id")?,
            reserved_turn_id: string_field(nested, family, "reserved_turn_id")?,
        },
        "provider_start_reserved" => OperationStatusValue::ProviderStartReserved {
            obligation_id: string_field(nested, family, "obligation_id")?,
        },
        "running" => OperationStatusValue::Running {
            turn_id: string_field(nested, family, "turn_id")?,
        },
        "completed" if kind == OperationKind::PermissionResponse => {
            OperationStatusValue::PermissionCompleted {
                decision: match required_text(nested, family, "decision")? {
                    "allow" | "allowed" => PermissionDecisionRecord::Allowed,
                    "deny" | "denied" => PermissionDecisionRecord::Denied,
                    other => {
                        return Err(StoredRecordCodecError::Incompatible {
                            family,
                            schema: format!("permission_decision.{other}"),
                        })
                    }
                },
            }
        }
        "completed" if kind == OperationKind::Stop => OperationStatusValue::StopCompleted {
            resolution: match required_text(nested, family, "resolution")? {
                "succeeded" => StopResolution::Succeeded,
                "superseded" => StopResolution::Superseded,
                other => {
                    return Err(StoredRecordCodecError::Incompatible {
                        family,
                        schema: format!("stop_resolution.{other}"),
                    })
                }
            },
        },
        "completed" => OperationStatusValue::Completed,
        "preparing" => OperationStatusValue::Preparing,
        "activated" => OperationStatusValue::Activated,
        "exit_pending" => OperationStatusValue::ExitPending,
        "exited" => OperationStatusValue::Exited,
        "outcome_unknown" => OperationStatusValue::OutcomeUnknown {
            operation_id: string_field(nested, family, "operation_id")?,
            plan: ShutdownPlanKey {
                shutdown_id: string_field(nested, family, "shutdown_id")?,
            },
            activation_commit_id: string_field(nested, family, "activation_commit_id")?,
        },
        "failed_before_activation" => OperationStatusValue::FailedBeforeActivation {
            failure: decode_failure(
                nested
                    .get("failure")
                    .ok_or(StoredRecordCodecError::MissingReference {
                        family,
                        field: "failure",
                    })?,
                family,
            )?,
        },
        "reconciliation_required" => OperationStatusValue::ReconciliationRequired {
            failure: decode_failure(
                nested
                    .get("failure")
                    .ok_or(StoredRecordCodecError::MissingReference {
                        family,
                        field: "failure",
                    })?,
                family,
            )?,
        },
        "failed" => OperationStatusValue::Failed {
            failure: decode_failure(
                nested
                    .get("failure")
                    .ok_or(StoredRecordCodecError::MissingReference {
                        family,
                        field: "failure",
                    })?,
                family,
            )?,
        },
        "terminal" => OperationStatusValue::Terminal {
            result: decode_turn_result(
                nested
                    .get("result")
                    .ok_or(StoredRecordCodecError::MissingReference {
                        family,
                        field: "result",
                    })?,
                family,
            )?,
        },
        other => {
            return Err(StoredRecordCodecError::Incompatible {
                family,
                schema: format!("{schema}.{other}"),
            })
        }
    };
    Ok(OperationStatusRecord { kind, value })
}

fn known_operation_status_fields(_schema: &str) -> &'static [&'static str] {
    &["schema", "status", "state"]
}

fn failure_kind_label(value: SessionOperationFailureKind) -> &'static str {
    match value {
        SessionOperationFailureKind::StorageUnavailable => "storage_unavailable",
        SessionOperationFailureKind::StorageCorrupt => "storage_corrupt",
        SessionOperationFailureKind::PersistFailure => "persist_failure",
        SessionOperationFailureKind::ProtocolIncompatible => "protocol_incompatible",
        SessionOperationFailureKind::ProviderUnavailable => "provider_unavailable",
        SessionOperationFailureKind::ExternalEffectFailed => "external_effect_failed",
        SessionOperationFailureKind::OutcomeUnknown => "outcome_unknown",
        SessionOperationFailureKind::DeadlineExceeded => "deadline_exceeded",
        SessionOperationFailureKind::CapacityExceeded => "capacity_exceeded",
        SessionOperationFailureKind::StopCapacityExceeded => "stop_capacity_exceeded",
        SessionOperationFailureKind::ShutdownAuthorityMismatch => "shutdown_authority_mismatch",
        SessionOperationFailureKind::TargetRevisionChanged => "target_revision_changed",
        SessionOperationFailureKind::OwnerRevisionChanged => "owner_revision_changed",
        SessionOperationFailureKind::RuntimeGenerationChanged => "runtime_generation_changed",
        SessionOperationFailureKind::InvalidEffectIntent => "invalid_effect_intent",
        SessionOperationFailureKind::PreviousShutdownReconciliationRequired => {
            "previous_shutdown_reconciliation_required"
        }
        SessionOperationFailureKind::Internal => "internal",
    }
}

fn parse_failure_kind(raw: &str) -> Option<SessionOperationFailureKind> {
    let normalized =
        raw.chars()
            .enumerate()
            .fold(String::new(), |mut value, (index, character)| {
                if character.is_ascii_uppercase() {
                    if index != 0 {
                        value.push('_');
                    }
                    value.push(character.to_ascii_lowercase());
                } else {
                    value.push(character);
                }
                value
            });
    Some(match normalized.as_str() {
        "storage_unavailable" => SessionOperationFailureKind::StorageUnavailable,
        "storage_corrupt" => SessionOperationFailureKind::StorageCorrupt,
        "persist_failure" => SessionOperationFailureKind::PersistFailure,
        "protocol_incompatible" => SessionOperationFailureKind::ProtocolIncompatible,
        "provider_unavailable" => SessionOperationFailureKind::ProviderUnavailable,
        "external_effect_failed" => SessionOperationFailureKind::ExternalEffectFailed,
        "outcome_unknown" => SessionOperationFailureKind::OutcomeUnknown,
        "deadline_exceeded" => SessionOperationFailureKind::DeadlineExceeded,
        "capacity_exceeded" => SessionOperationFailureKind::CapacityExceeded,
        "stop_capacity_exceeded" => SessionOperationFailureKind::StopCapacityExceeded,
        "shutdown_authority_mismatch" => SessionOperationFailureKind::ShutdownAuthorityMismatch,
        "target_revision_changed" => SessionOperationFailureKind::TargetRevisionChanged,
        "owner_revision_changed" => SessionOperationFailureKind::OwnerRevisionChanged,
        "runtime_generation_changed" => SessionOperationFailureKind::RuntimeGenerationChanged,
        "invalid_effect_intent" => SessionOperationFailureKind::InvalidEffectIntent,
        "previous_shutdown_reconciliation_required" => {
            SessionOperationFailureKind::PreviousShutdownReconciliationRequired
        }
        "internal" => SessionOperationFailureKind::Internal,
        _ => return None,
    })
}

fn encode_failure(value: &crate::domain::local_event::SafeOperationFailure) -> Value {
    let mut object = Map::new();
    object.insert(
        "kind".into(),
        Value::String(failure_kind_label(value.kind).to_string()),
    );
    object.insert("retryable".into(), Value::Bool(value.retryable));
    object.insert(
        "label".into(),
        Value::String(value.label.value().to_string()),
    );
    if let Some(detail) = &value.detail {
        object.insert("detail".into(), Value::String(detail.value().to_string()));
    }
    object.insert(
        "correlation_id".into(),
        Value::String(value.correlation_id.clone()),
    );
    Value::Object(object)
}

fn decode_failure(
    value: &Value,
    family: StoredRecordFamily,
) -> Result<crate::domain::local_event::SafeOperationFailure, StoredRecordCodecError> {
    let object = value
        .as_object()
        .ok_or(StoredRecordCodecError::Malformed { family })?;
    let raw_kind = required_text(object, family, "kind")?;
    let kind =
        parse_failure_kind(raw_kind).ok_or_else(|| StoredRecordCodecError::Incompatible {
            family,
            schema: format!("failure_kind.{raw_kind}"),
        })?;
    let retryable = object.get("retryable").and_then(Value::as_bool).ok_or(
        StoredRecordCodecError::MissingReference {
            family,
            field: "retryable",
        },
    )?;
    let label = object
        .get("label")
        .or_else(|| object.get("message"))
        .and_then(Value::as_str)
        .ok_or(StoredRecordCodecError::MissingReference {
            family,
            field: "label|message",
        })?;
    let correlation_id = string_field(object, family, "correlation_id")?;
    let mut failure = crate::domain::local_event::SafeOperationFailure::new(
        kind,
        retryable,
        label,
        correlation_id,
    );
    if let Some(detail) = object.get("detail").and_then(Value::as_str) {
        failure = failure.with_detail(detail);
    }
    Ok(failure)
}

fn encode_token_usage(value: &TokenUsage) -> Value {
    let mut object = Map::new();
    object.insert("input_tokens".into(), Value::from(value.input_tokens));
    object.insert("output_tokens".into(), Value::from(value.output_tokens));
    if let Some(total) = value.total_tokens {
        object.insert("total_tokens".into(), Value::from(total));
    }
    if let Some(window) = value.context_window_tokens {
        object.insert("context_window_tokens".into(), Value::from(window));
    }
    Value::Object(object)
}

fn decode_token_usage(
    value: &Value,
    family: StoredRecordFamily,
) -> Result<TokenUsage, StoredRecordCodecError> {
    let object = value
        .as_object()
        .ok_or(StoredRecordCodecError::Malformed { family })?;
    Ok(TokenUsage {
        input_tokens: u64_field(object, family, "input_tokens")?,
        output_tokens: u64_field(object, family, "output_tokens")?,
        total_tokens: optional_u64(object, "total_tokens"),
        context_window_tokens: optional_u64(object, "context_window_tokens"),
    })
}

fn encode_turn_result(value: &TurnResult) -> Value {
    match value {
        TurnResult::Completed {
            stop_reason,
            token_usage,
        } => {
            let mut object = Map::new();
            object.insert("type".into(), Value::String("completed".into()));
            if matches!(stop_reason, Some(TurnStopReason::Refusal)) {
                object.insert("stop_reason".into(), Value::String("refusal".into()));
            }
            if let Some(usage) = token_usage {
                object.insert("token_usage".into(), encode_token_usage(usage));
            }
            Value::Object(object)
        }
        TurnResult::Failed { error, token_usage } => {
            let mut object = Map::new();
            object.insert("type".into(), Value::String("failed".into()));
            object.insert("error".into(), Value::String(error.clone()));
            if let Some(usage) = token_usage {
                object.insert("token_usage".into(), encode_token_usage(usage));
            }
            Value::Object(object)
        }
        TurnResult::Interrupted { reason, error } => {
            let mut object = Map::new();
            object.insert("type".into(), Value::String("interrupted".into()));
            object.insert(
                "reason".into(),
                Value::String(
                    match reason {
                        InterruptReason::Abort => "abort",
                        InterruptReason::Timeout => "timeout",
                        InterruptReason::Crash => "crash",
                        InterruptReason::SessionClosed => "session_closed",
                    }
                    .into(),
                ),
            );
            if let Some(error) = error {
                object.insert("error".into(), Value::String(error.clone()));
            }
            Value::Object(object)
        }
    }
}

fn decode_turn_result(
    value: &Value,
    family: StoredRecordFamily,
) -> Result<TurnResult, StoredRecordCodecError> {
    let object = value
        .as_object()
        .ok_or(StoredRecordCodecError::Malformed { family })?;
    let token_usage = object
        .get("token_usage")
        .map(|usage| decode_token_usage(usage, family))
        .transpose()?;
    match required_text(object, family, "type")? {
        "completed" => Ok(TurnResult::Completed {
            stop_reason: match object.get("stop_reason").and_then(Value::as_str) {
                None => None,
                Some("refusal") => Some(TurnStopReason::Refusal),
                Some(other) => {
                    return Err(StoredRecordCodecError::Incompatible {
                        family,
                        schema: format!("turn_stop_reason.{other}"),
                    })
                }
            },
            token_usage,
        }),
        "failed" => Ok(TurnResult::Failed {
            error: string_field(object, family, "error")?,
            token_usage,
        }),
        "interrupted" => Ok(TurnResult::Interrupted {
            reason: match required_text(object, family, "reason")? {
                "abort" => InterruptReason::Abort,
                "timeout" => InterruptReason::Timeout,
                "crash" => InterruptReason::Crash,
                "session_closed" => InterruptReason::SessionClosed,
                other => {
                    return Err(StoredRecordCodecError::Incompatible {
                        family,
                        schema: format!("interrupt_reason.{other}"),
                    })
                }
            },
            error: optional_string(object, "error"),
        }),
        other => Err(StoredRecordCodecError::Incompatible {
            family,
            schema: format!("turn_result.{other}"),
        }),
    }
}

fn terminal_reason_label(value: TerminalInterruptReasonRecord) -> &'static str {
    match value {
        TerminalInterruptReasonRecord::Abort => "abort",
        TerminalInterruptReasonRecord::Timeout => "timeout",
        TerminalInterruptReasonRecord::Crash => "crash",
        TerminalInterruptReasonRecord::SessionClosed => "session_closed",
    }
}

fn decode_terminal_reason(
    raw: &str,
    family: StoredRecordFamily,
) -> Result<TerminalInterruptReasonRecord, StoredRecordCodecError> {
    match raw {
        "abort" => Ok(TerminalInterruptReasonRecord::Abort),
        "timeout" => Ok(TerminalInterruptReasonRecord::Timeout),
        "crash" => Ok(TerminalInterruptReasonRecord::Crash),
        "session_closed" => Ok(TerminalInterruptReasonRecord::SessionClosed),
        other => Err(StoredRecordCodecError::Incompatible {
            family,
            schema: format!("terminal_reason.{other}"),
        }),
    }
}

fn encode_terminal(value: &TerminalResultRecord) -> Result<Value, StoredRecordCodecError> {
    Ok(match value {
        TerminalResultRecord::AgentTurn {
            kind,
            session_id,
            turn_id,
            message_id,
            streaming_final_sequence,
            completed_at_bits,
            result,
        } => {
            let mut object = Map::new();
            object.insert(
                "schema".into(),
                Value::String("agent_turn_terminal_v1".into()),
            );
            object.insert(
                "terminal_kind".into(),
                Value::String(
                    match kind {
                        AgentTerminalKind::Completed => "completed",
                        AgentTerminalKind::Abort => "abort",
                        AgentTerminalKind::Timeout => "timeout",
                        AgentTerminalKind::Crash => "crash",
                        AgentTerminalKind::SessionClosed => "session_closed",
                    }
                    .into(),
                ),
            );
            object.insert("session_id".into(), Value::String(session_id.clone()));
            object.insert("turn_id".into(), Value::String(turn_id.clone()));
            object.insert("message_id".into(), Value::String(message_id.clone()));
            object.insert(
                "streaming_final_seq".into(),
                Value::String(streaming_final_sequence.to_string()),
            );
            object.insert(
                "completed_at_bits".into(),
                Value::String(completed_at_bits.to_string()),
            );
            let AgentTurnTerminalResultRecord::Current(result) = result;
            object.insert("turn_result".into(), encode_turn_result(result));
            Value::Object(object)
        }
        TerminalResultRecord::SessionClosed {
            operation_id,
            reason,
            result,
        } => serde_json::json!({
            "schema":"session_closed_terminal_v1",
            "operation_id":operation_id,
            "reason":terminal_reason_label(*reason),
            "turn_result":encode_turn_result(result),
        }),
        TerminalResultRecord::Stop {
            operation_id,
            reason,
            exit_code,
            result,
        } => {
            let mut object = Map::new();
            object.insert("schema".into(), Value::String("stop_terminal_v1".into()));
            object.insert("operation_id".into(), Value::String(operation_id.clone()));
            object.insert(
                "reason".into(),
                Value::String(
                    reason
                        .map(terminal_reason_label)
                        .unwrap_or("terminal_winner")
                        .to_string(),
                ),
            );
            if let Some(exit_code) = exit_code {
                object.insert("exit_code".into(), Value::from(*exit_code));
            }
            object.insert("turn_result".into(), encode_turn_result(result));
            Value::Object(object)
        }
        TerminalResultRecord::StopSuperseded {
            terminal_identity,
            terminal_result_sha256,
        } => serde_json::json!({
            "schema":"stop_superseded_v1",
            "terminal_identity":terminal_identity,
            "terminal_result_sha256":hex::encode(terminal_result_sha256),
        }),
    })
}

fn decode_terminal(
    object: &Map<String, Value>,
) -> Result<TerminalResultRecord, StoredRecordCodecError> {
    let family = StoredRecordFamily::Terminal;
    match required_text(object, family, "schema")? {
        "agent_turn_terminal_v1" => {
            let kind = match required_text(object, family, "terminal_kind")? {
                "completed" => AgentTerminalKind::Completed,
                "abort" => AgentTerminalKind::Abort,
                "timeout" => AgentTerminalKind::Timeout,
                "crash" => AgentTerminalKind::Crash,
                "session_closed" => AgentTerminalKind::SessionClosed,
                other => {
                    return Err(StoredRecordCodecError::Incompatible {
                        family,
                        schema: format!("agent_terminal_kind.{other}"),
                    })
                }
            };
            let result = AgentTurnTerminalResultRecord::Current(decode_turn_result(
                object
                    .get("turn_result")
                    .ok_or(StoredRecordCodecError::MissingReference {
                        family,
                        field: "turn_result",
                    })?,
                family,
            )?);
            Ok(TerminalResultRecord::AgentTurn {
                kind,
                session_id: string_field(object, family, "session_id")?,
                turn_id: string_field(object, family, "turn_id")?,
                message_id: string_field(object, family, "message_id")?,
                streaming_final_sequence: u64_field(object, family, "streaming_final_seq")?,
                completed_at_bits: u64_field(object, family, "completed_at_bits")?,
                result,
            })
        }
        "session_closed_terminal_v1" => Ok(TerminalResultRecord::SessionClosed {
            operation_id: string_field(object, family, "operation_id")?,
            reason: decode_terminal_reason(required_text(object, family, "reason")?, family)?,
            result: decode_turn_result(
                object
                    .get("turn_result")
                    .ok_or(StoredRecordCodecError::MissingReference {
                        family,
                        field: "turn_result",
                    })?,
                family,
            )?,
        }),
        "stop_terminal_v1" => {
            let reason = match required_text(object, family, "reason")? {
                "terminal_winner" => None,
                other => Some(decode_terminal_reason(other, family)?),
            };
            Ok(TerminalResultRecord::Stop {
                operation_id: string_field(object, family, "operation_id")?,
                reason,
                exit_code: optional_i64(object, "exit_code")
                    .map(i32::try_from)
                    .transpose()
                    .map_err(|_| StoredRecordCodecError::Malformed { family })?,
                result: decode_turn_result(
                    object
                        .get("turn_result")
                        .ok_or(StoredRecordCodecError::MissingReference {
                            family,
                            field: "turn_result",
                        })?,
                    family,
                )?,
            })
        }
        "stop_superseded_v1" => Ok(TerminalResultRecord::StopSuperseded {
            terminal_identity: string_field(object, family, "terminal_identity")?,
            terminal_result_sha256: hash_field(object, family, "terminal_result_sha256")?,
        }),
        schema => Err(StoredRecordCodecError::Incompatible {
            family,
            schema: schema.to_string(),
        }),
    }
}

fn known_terminal_fields(schema: &str) -> &'static [&'static str] {
    match schema {
        "agent_turn_terminal_v1" => &[
            "schema",
            "terminal_kind",
            "session_id",
            "turn_id",
            "message_id",
            "streaming_final_seq",
            "completed_at_bits",
            "turn_result",
        ],
        "session_closed_terminal_v1" => &["schema", "operation_id", "reason", "turn_result"],
        "stop_terminal_v1" => &[
            "schema",
            "operation_id",
            "reason",
            "exit_code",
            "turn_result",
        ],
        "stop_superseded_v1" => &["schema", "terminal_identity", "terminal_result_sha256"],
        _ => &["schema"],
    }
}

fn obligation_state_label(value: ObligationStateRecord) -> &'static str {
    match value {
        ObligationStateRecord::Prepared => "prepared",
        ObligationStateRecord::Pending => "pending",
        ObligationStateRecord::EffectReserved => "effect_reserved",
        ObligationStateRecord::Running => "running",
        ObligationStateRecord::WaitingApproval => "waiting_approval",
        ObligationStateRecord::OutcomeUnknown => "outcome_unknown",
        ObligationStateRecord::ReconciliationRequired => "reconciliation_required",
        ObligationStateRecord::Failed => "failed",
        ObligationStateRecord::Completed => "completed",
        ObligationStateRecord::Cancelled => "cancelled",
    }
}

fn decode_obligation_state(
    object: &Map<String, Value>,
    family: StoredRecordFamily,
) -> Result<ObligationStateRecord, StoredRecordCodecError> {
    match required_text(object, family, "state")? {
        "prepared" => Ok(ObligationStateRecord::Prepared),
        "pending" => Ok(ObligationStateRecord::Pending),
        "effect_reserved" => Ok(ObligationStateRecord::EffectReserved),
        "running" => Ok(ObligationStateRecord::Running),
        "waiting_approval" => Ok(ObligationStateRecord::WaitingApproval),
        "outcome_unknown" => Ok(ObligationStateRecord::OutcomeUnknown),
        "reconciliation_required" => Ok(ObligationStateRecord::ReconciliationRequired),
        "failed" => Ok(ObligationStateRecord::Failed),
        "completed" => Ok(ObligationStateRecord::Completed),
        "cancelled" => Ok(ObligationStateRecord::Cancelled),
        other => Err(StoredRecordCodecError::Incompatible {
            family,
            schema: format!("obligation_state.{other}"),
        }),
    }
}

fn recovery_action_label(value: RecoveryActionKind) -> &'static str {
    match value {
        RecoveryActionKind::ReadAgain => "read_again",
        RecoveryActionKind::RetrySameEffect => "retry_same_effect",
        RecoveryActionKind::UseObservedResult => "use_observed_result",
        RecoveryActionKind::CancelIfSafe => "cancel_if_safe",
        RecoveryActionKind::KeepForManualResolution => "keep_for_manual_resolution",
    }
}

fn decode_recovery_action_kind(
    raw: &str,
    family: StoredRecordFamily,
) -> Result<RecoveryActionKind, StoredRecordCodecError> {
    match raw {
        "read_again" => Ok(RecoveryActionKind::ReadAgain),
        "retry_same_effect" => Ok(RecoveryActionKind::RetrySameEffect),
        "use_observed_result" => Ok(RecoveryActionKind::UseObservedResult),
        "cancel_if_safe" => Ok(RecoveryActionKind::CancelIfSafe),
        "keep_for_manual_resolution" => Ok(RecoveryActionKind::KeepForManualResolution),
        other => Err(StoredRecordCodecError::Incompatible {
            family,
            schema: format!("recovery_action.{other}"),
        }),
    }
}

fn recovery_classification_label(value: RecoveryResultClassification) -> &'static str {
    match value {
        RecoveryResultClassification::Pending => "pending",
        RecoveryResultClassification::Succeeded => "succeeded",
        RecoveryResultClassification::ConfirmedNoEffect => "confirmed_no_effect",
        RecoveryResultClassification::Ambiguous => "ambiguous",
        RecoveryResultClassification::CancelledBeforeEffect => "cancelled_before_effect",
        RecoveryResultClassification::Unchanged => "unchanged",
    }
}

fn decode_recovery_classification(
    raw: &str,
    family: StoredRecordFamily,
) -> Result<RecoveryResultClassification, StoredRecordCodecError> {
    match raw {
        "pending" => Ok(RecoveryResultClassification::Pending),
        "succeeded" => Ok(RecoveryResultClassification::Succeeded),
        "confirmed_no_effect" => Ok(RecoveryResultClassification::ConfirmedNoEffect),
        "ambiguous" => Ok(RecoveryResultClassification::Ambiguous),
        "cancelled_before_effect" => Ok(RecoveryResultClassification::CancelledBeforeEffect),
        "unchanged" => Ok(RecoveryResultClassification::Unchanged),
        other => Err(StoredRecordCodecError::Incompatible {
            family,
            schema: format!("recovery_classification.{other}"),
        }),
    }
}

fn encode_recovery_transition(value: &ObligationRecoveryActionRecord) -> Value {
    let mut object = Map::new();
    object.insert("action_id".into(), Value::String(value.action_id.clone()));
    object.insert("origin_revision".into(), Value::from(value.origin_revision));
    object.insert(
        "action".into(),
        Value::String(recovery_action_label(value.action).to_string()),
    );
    object.insert(
        "effect_identity".into(),
        Value::String(value.effect_identity.clone()),
    );
    object.insert(
        "state".into(),
        Value::String(obligation_state_label(value.state).to_string()),
    );
    if let Some(classification) = value.classification {
        object.insert(
            "classification".into(),
            Value::String(recovery_classification_label(classification).to_string()),
        );
    }
    Value::Object(object)
}

fn decode_recovery_transition(
    value: &Value,
    family: StoredRecordFamily,
) -> Result<ObligationRecoveryActionRecord, StoredRecordCodecError> {
    let object = value
        .as_object()
        .ok_or(StoredRecordCodecError::Malformed { family })?;
    Ok(ObligationRecoveryActionRecord {
        action_id: string_field(object, family, "action_id")?,
        origin_revision: u64_field(object, family, "origin_revision")?,
        action: decode_recovery_action_kind(required_text(object, family, "action")?, family)?,
        effect_identity: string_field(object, family, "effect_identity")?,
        state: decode_obligation_state(object, family)?,
        classification: object
            .get("classification")
            .and_then(Value::as_str)
            .map(|raw| decode_recovery_classification(raw, family))
            .transpose()?,
    })
}

fn encode_authoritative_observation(value: &AuthoritativeEffectObservationRecord) -> Value {
    serde_json::json!({
        "effect_identity":value.effect_identity,
        "origin_revision":value.origin_revision,
        "classification":recovery_classification_label(value.classification),
        "cancellable":value.cancellable,
        "safe_view":value.safe_view,
        "result_sha256":hex::encode(value.result_sha256),
        "proof_mac":hex::encode(value.proof_mac),
    })
}

fn decode_authoritative_observation(
    value: &Value,
    family: StoredRecordFamily,
) -> Result<AuthoritativeEffectObservationRecord, StoredRecordCodecError> {
    let object = value
        .as_object()
        .ok_or(StoredRecordCodecError::Malformed { family })?;
    Ok(AuthoritativeEffectObservationRecord {
        effect_identity: string_field(object, family, "effect_identity")?,
        origin_revision: u64_field(object, family, "origin_revision")?,
        classification: decode_recovery_classification(
            required_text(object, family, "classification")?,
            family,
        )?,
        cancellable: object.get("cancellable").and_then(Value::as_bool).ok_or(
            StoredRecordCodecError::MissingReference {
                family,
                field: "cancellable",
            },
        )?,
        safe_view: string_field(object, family, "safe_view")?,
        result_sha256: hash_field(object, family, "result_sha256")?,
        proof_mac: hash_field(object, family, "proof_mac")?,
    })
}

fn json_payload_value(
    value: &JsonPayload,
    family: StoredRecordFamily,
) -> Result<Value, StoredRecordCodecError> {
    serde_json::from_str(value.as_str()).map_err(|_| StoredRecordCodecError::Malformed { family })
}

fn decode_json_payload(
    value: &Value,
    family: StoredRecordFamily,
) -> Result<JsonPayload, StoredRecordCodecError> {
    serde_json::to_string(value)
        .map(JsonPayload::new_unchecked)
        .map_err(|_| StoredRecordCodecError::Malformed { family })
}

fn encode_permission_response(
    value: &PermissionResponse,
    family: StoredRecordFamily,
) -> Result<Value, StoredRecordCodecError> {
    let decision = match &value.decision {
        PermissionResponseDecision::Allow {
            updated_input,
            answers,
        } => serde_json::json!({
            "type":"allow",
            "updated_input":updated_input
                .as_ref()
                .map(|value| json_payload_value(value, family))
                .transpose()?,
            "answers":answers
                .as_ref()
                .map(|value| json_payload_value(value, family))
                .transpose()?,
        }),
        PermissionResponseDecision::Deny { message } => {
            serde_json::json!({"type":"deny","message":message})
        }
    };
    Ok(serde_json::json!({
        "request_id":value.request_id,
        "decision":decision,
    }))
}

fn decode_permission_response(
    value: &Value,
    family: StoredRecordFamily,
) -> Result<PermissionResponse, StoredRecordCodecError> {
    let object = value
        .as_object()
        .ok_or(StoredRecordCodecError::Malformed { family })?;
    let decision = object.get("decision").and_then(Value::as_object).ok_or(
        StoredRecordCodecError::MissingReference {
            family,
            field: "decision",
        },
    )?;
    let decision = match required_text(decision, family, "type")? {
        "allow" => PermissionResponseDecision::Allow {
            updated_input: decision
                .get("updated_input")
                .filter(|value| !value.is_null())
                .map(|value| decode_json_payload(value, family))
                .transpose()?,
            answers: decision
                .get("answers")
                .filter(|value| !value.is_null())
                .map(|value| decode_json_payload(value, family))
                .transpose()?,
        },
        "deny" => PermissionResponseDecision::Deny {
            message: match decision.get("message") {
                None | Some(Value::Null) => None,
                Some(Value::String(message)) => Some(message.clone()),
                _ => return malformed(family),
            },
        },
        other => {
            return Err(StoredRecordCodecError::Incompatible {
                family,
                schema: format!("permission_response_decision.{other}"),
            })
        }
    };
    Ok(PermissionResponse {
        request_id: string_field(object, family, "request_id")?,
        decision,
    })
}

fn encode_workflow_context(value: &crate::domain::workflow::WorkflowNodeContext) -> Value {
    serde_json::json!({
        "execution_id":value.execution_id,
        "node_execution_id":value.node_execution_id,
        "workflow_name":value.workflow_name,
        "node_name":value.node_name,
        "attempt":value.attempt,
        "parent_node_name":value.parent_node_name,
        "parent_attempt":value.parent_attempt,
        "order":value.order,
        "startup_timeout_secs":value.startup_timeout_secs,
        "startup_max_retries":value.startup_max_retries,
        "stale_timeout_secs":value.stale_timeout_secs,
    })
}

fn decode_workflow_context(
    value: &Value,
    family: StoredRecordFamily,
) -> Result<crate::domain::workflow::WorkflowNodeContext, StoredRecordCodecError> {
    let object = value
        .as_object()
        .ok_or(StoredRecordCodecError::Malformed { family })?;
    Ok(crate::domain::workflow::WorkflowNodeContext {
        execution_id: string_field(object, family, "execution_id")?,
        node_execution_id: string_field(object, family, "node_execution_id")?,
        workflow_name: string_field(object, family, "workflow_name")?,
        node_name: string_field(object, family, "node_name")?,
        attempt: u32::try_from(u64_field(object, family, "attempt")?)
            .map_err(|_| StoredRecordCodecError::Malformed { family })?,
        parent_node_name: optional_string(object, "parent_node_name"),
        parent_attempt: optional_u64(object, "parent_attempt")
            .map(u32::try_from)
            .transpose()
            .map_err(|_| StoredRecordCodecError::Malformed { family })?,
        order: u32::try_from(u64_field(object, family, "order")?)
            .map_err(|_| StoredRecordCodecError::Malformed { family })?,
        startup_timeout_secs: optional_u64(object, "startup_timeout_secs"),
        startup_max_retries: optional_u64(object, "startup_max_retries")
            .map(u32::try_from)
            .transpose()
            .map_err(|_| StoredRecordCodecError::Malformed { family })?,
        stale_timeout_secs: optional_u64(object, "stale_timeout_secs"),
    })
}

fn encode_publication_message(value: &RecoveryPublicationMessageRecord) -> Value {
    let mut object = Map::new();
    object.insert(
        "kind".into(),
        Value::String(
            match value.kind {
                RecoveryPublicationMessageKindRecord::Notice => "notice",
                RecoveryPublicationMessageKindRecord::Error => "error",
            }
            .to_string(),
        ),
    );
    object.insert(
        "recovery_id".into(),
        Value::String(value.recovery_id.clone()),
    );
    object.insert("message_id".into(), Value::String(value.message_id.clone()));
    if let Some(error) = &value.error {
        object.insert("error".into(), Value::String(error.clone()));
    }
    Value::Object(object)
}

fn decode_publication_message(
    value: &Value,
    family: StoredRecordFamily,
) -> Result<RecoveryPublicationMessageRecord, StoredRecordCodecError> {
    let object = value
        .as_object()
        .ok_or(StoredRecordCodecError::Malformed { family })?;
    Ok(RecoveryPublicationMessageRecord {
        kind: match required_text(object, family, "kind")? {
            "notice" => RecoveryPublicationMessageKindRecord::Notice,
            "error" => RecoveryPublicationMessageKindRecord::Error,
            other => {
                return Err(StoredRecordCodecError::Incompatible {
                    family,
                    schema: format!("recovery_publication_message.{other}"),
                })
            }
        },
        recovery_id: string_field(object, family, "recovery_id")?,
        message_id: string_field(object, family, "message_id")?,
        error: optional_string(object, "error"),
    })
}

fn lifecycle_action_record_label(value: &SessionLifecycleRecordAction) -> String {
    match value {
        SessionLifecycleRecordAction::Close => "close".to_string(),
        SessionLifecycleRecordAction::ArchiveOpen => "archive_open".to_string(),
        SessionLifecycleRecordAction::ArchiveClosed => "archive_closed".to_string(),
        SessionLifecycleRecordAction::SwitchBackend { backend_id } => {
            format!("switch_backend:{backend_id}")
        }
    }
}

fn decode_lifecycle_action_record_label(
    raw: &str,
    family: StoredRecordFamily,
) -> Result<SessionLifecycleRecordAction, StoredRecordCodecError> {
    match raw {
        "close" => Ok(SessionLifecycleRecordAction::Close),
        "archive_open" => Ok(SessionLifecycleRecordAction::ArchiveOpen),
        "archive_closed" => Ok(SessionLifecycleRecordAction::ArchiveClosed),
        _ if raw.starts_with("switch_backend:") => {
            Ok(SessionLifecycleRecordAction::SwitchBackend {
                backend_id: bounded_reference(
                    &raw["switch_backend:".len()..],
                    family,
                    "action.backend_id",
                )?,
            })
        }
        other => Err(StoredRecordCodecError::Incompatible {
            family,
            schema: format!("session_close_action.{other}"),
        }),
    }
}

fn encode_obligation(value: &ObligationRecord) -> Result<Value, StoredRecordCodecError> {
    let family = StoredRecordFamily::Obligation;
    match value {
        ObligationRecord::RecoveryTransition {
            original,
            recovery_action,
        } => {
            let mut object = encode_obligation(original)?
                .as_object()
                .cloned()
                .ok_or(StoredRecordCodecError::Malformed { family })?;
            object.insert(
                "recovery_action".into(),
                encode_recovery_transition(recovery_action),
            );
            Ok(Value::Object(object))
        }
        ObligationRecord::Observed {
            original,
            observation,
        } => {
            let mut object = encode_obligation(original)?
                .as_object()
                .cloned()
                .ok_or(StoredRecordCodecError::Malformed { family })?;
            object
                .entry("effect_identity".to_string())
                .or_insert_with(|| Value::String(observation.effect_identity.clone()));
            object.insert(
                "authoritative_observation".into(),
                encode_authoritative_observation(observation),
            );
            Ok(Value::Object(object))
        }
        ObligationRecord::Send {
            obligation_id,
            operation_id,
            session_id,
            kind,
            disposition,
            human_message_id,
            assistant_message_id,
            reserved_turn_id,
            turn_id,
            dependency_obligation_ids,
            canonical_payload,
            state,
        } => Ok(serde_json::json!({
            "schema":"send_obligation_v1",
            "obligation_id":obligation_id,
            "operation_id":operation_id,
            "session_id":session_id,
            "kind":match kind {
                SendObligationKindRecord::ProviderEstablish => "provider_establish",
                SendObligationKindRecord::TurnExecution => "turn_execution",
            },
            "disposition":match disposition {
                SendObligationDispositionRecord::StartedTurn => "started_turn",
                SendObligationDispositionRecord::Queued => "queued",
            },
            "human_message_id":human_message_id,
            "assistant_message_id":assistant_message_id,
            "reserved_turn_id":reserved_turn_id,
            "turn_id":turn_id,
            "dependency_obligation_ids":dependency_obligation_ids,
            "canonical_payload":canonical_payload,
            "state":obligation_state_label(*state),
        })),
        ObligationRecord::PermissionResponse {
            operation_id,
            effect_identity,
            session_id,
            turn_id,
            response,
            owner_access,
            from_runtime_state,
            state,
        } => Ok(serde_json::json!({
            "schema":"permission_response_obligation_v1",
            "operation_id":operation_id,
            "effect_identity":effect_identity,
            "session_id":session_id,
            "turn_id":turn_id,
            "request_id":response.request_id,
            "exact_response":encode_permission_response(response, family)?,
            "owner_access":owner_access,
            "from_runtime_state":from_runtime_state,
            "state":obligation_state_label(*state),
        })),
        ObligationRecord::StopInterrupt {
            operation_id,
            session_id,
            turn_id,
            expected_revision,
            deadline_ms,
            state,
        } => Ok(serde_json::json!({
            "schema":"stop_interrupt_obligation_v1",
            "operation_id":operation_id,
            "session_id":session_id,
            "turn_id":turn_id,
            "expected_revision":expected_revision,
            "deadline_ms":deadline_ms,
            "state":obligation_state_label(*state),
        })),
        ObligationRecord::SessionClose {
            obligation_id,
            operation_id,
            session_id,
            action,
            state,
        } => Ok(serde_json::json!({
            "schema":"session_close_obligation_v1",
            "obligation_id":obligation_id,
            "operation_id":operation_id,
            "session_id":session_id,
            "action":lifecycle_action_record_label(action),
            "state":obligation_state_label(*state),
        })),
        ObligationRecord::BackendSessionRecovery {
            session_id,
            recovery_id,
            detail,
            state,
        } => {
            let mut object = Map::new();
            object.insert(
                "schema".into(),
                Value::String("backend_session_recovery_obligation_v1".into()),
            );
            object.insert("session_id".into(), Value::String(session_id.clone()));
            object.insert("recovery_id".into(), Value::String(recovery_id.clone()));
            object.insert(
                "state".into(),
                Value::String(obligation_state_label(*state).to_string()),
            );
            match detail {
                BackendSessionRecoveryObligationRecord::EffectReserved {
                    old_provider_session_generation,
                    reason,
                    reserved_at_bits,
                } => {
                    object.insert(
                        "old_provider_session_generation".into(),
                        Value::String(old_provider_session_generation.to_string()),
                    );
                    object.insert(
                        "reason".into(),
                        Value::String(
                            match reason {
                                BackendSessionRecoveryReason::ResumeMismatch => "resume_mismatch",
                                BackendSessionRecoveryReason::BackendSessionLost => {
                                    "backend_session_lost"
                                }
                            }
                            .to_string(),
                        ),
                    );
                    object.insert(
                        "reserved_at_bits".into(),
                        Value::String(reserved_at_bits.to_string()),
                    );
                }
                BackendSessionRecoveryObligationRecord::Completed {
                    old_provider_session_generation,
                    provider_session_generation,
                    backend_session_id,
                    completed_at_bits,
                } => {
                    object.insert(
                        "old_provider_session_generation".into(),
                        Value::String(old_provider_session_generation.to_string()),
                    );
                    object.insert(
                        "provider_session_generation".into(),
                        Value::String(provider_session_generation.to_string()),
                    );
                    object.insert(
                        "backend_session_id".into(),
                        Value::String(backend_session_id.clone()),
                    );
                    object.insert(
                        "completed_at_bits".into(),
                        Value::String(completed_at_bits.to_string()),
                    );
                }
                BackendSessionRecoveryObligationRecord::Failed {
                    error_sha256,
                    failed_at_bits,
                } => {
                    object.insert(
                        "error_sha256".into(),
                        Value::String(hex::encode(error_sha256)),
                    );
                    object.insert(
                        "failed_at_bits".into(),
                        Value::String(failed_at_bits.to_string()),
                    );
                }
            }
            Ok(Value::Object(object))
        }
        ObligationRecord::WorkflowShutdown {
            operation_id,
            effect_identity,
            owner_revision,
            execution_id,
            state,
        } => Ok(serde_json::json!({
            "schema":"workflow_shutdown_effect_v1",
            "operation_id":operation_id,
            "effect_identity":effect_identity,
            "owner_revision":owner_revision,
            "execution_id":execution_id,
            "state":obligation_state_label(*state),
        })),
        ObligationRecord::WorkflowTurnCompletion {
            session_id,
            turn_id,
            terminal_identity,
            notification_sha256,
            detail,
            state,
        } => {
            let mut object = Map::new();
            object.insert(
                "schema".into(),
                Value::String("workflow_turn_completion_obligation_v1".into()),
            );
            object.insert("session_id".into(), Value::String(session_id.clone()));
            object.insert("turn_id".into(), Value::String(turn_id.clone()));
            object.insert(
                "terminal_identity".into(),
                Value::String(terminal_identity.clone()),
            );
            object.insert(
                "notification_sha256".into(),
                Value::String(hex::encode(notification_sha256)),
            );
            object.insert(
                "state".into(),
                Value::String(obligation_state_label(*state).to_string()),
            );
            match detail {
                WorkflowTurnCompletionObligationRecord::Pending {
                    workflow_context,
                    message_id,
                    exit_code,
                    failure_signal,
                    token_usage,
                    interrupted,
                } => {
                    object.insert(
                        "workflow_context".into(),
                        encode_workflow_context(workflow_context),
                    );
                    object.insert("message_id".into(), Value::String(message_id.clone()));
                    object.insert("exit_code".into(), Value::from(*exit_code));
                    if let Some(signal) = failure_signal {
                        object.insert(
                            "failure_signal".into(),
                            Value::String(
                                match signal {
                                    WorkflowTurnFailureSignalRecord::ModelRefusal => {
                                        "model_refusal"
                                    }
                                }
                                .to_string(),
                            ),
                        );
                    }
                    if let Some(usage) = token_usage {
                        object.insert(
                            "token_usage".into(),
                            serde_json::json!({
                                "input_tokens":usage.input_tokens,
                                "output_tokens":usage.output_tokens,
                            }),
                        );
                    }
                    object.insert("interrupted".into(), Value::Bool(*interrupted));
                }
                WorkflowTurnCompletionObligationRecord::Completed { completed_at_bits } => {
                    object.insert(
                        "completed_at_bits".into(),
                        Value::String(completed_at_bits.to_string()),
                    );
                }
            }
            Ok(Value::Object(object))
        }
        ObligationRecord::RecoveryPublication {
            session_id,
            recovery_id,
            message_id,
            source_obligation_id,
            detail,
            state,
        } => {
            let mut object = Map::new();
            object.insert(
                "schema".into(),
                Value::String("recovery_publication_obligation_v1".into()),
            );
            object.insert("session_id".into(), Value::String(session_id.clone()));
            object.insert("recovery_id".into(), Value::String(recovery_id.clone()));
            object.insert("message_id".into(), Value::String(message_id.clone()));
            object.insert(
                "source_obligation_id".into(),
                Value::String(source_obligation_id.clone()),
            );
            object.insert(
                "state".into(),
                Value::String(obligation_state_label(*state).to_string()),
            );
            match detail {
                RecoveryPublicationObligationRecord::Pending { pending_message } => {
                    object.insert(
                        "pending_message".into(),
                        encode_publication_message(pending_message),
                    );
                }
                RecoveryPublicationObligationRecord::Completed { published_at_bits } => {
                    object.insert(
                        "published_at_bits".into(),
                        Value::String(published_at_bits.to_string()),
                    );
                }
            }
            Ok(Value::Object(object))
        }
        ObligationRecord::ProviderEstablish {
            operation_id,
            effect_identity,
            session_id,
            state,
        } => Ok(serde_json::json!({
            "schema":"provider_establish_obligation_v1",
            "operation_id":operation_id,
            "effect_identity":effect_identity,
            "session_id":session_id,
            "state":obligation_state_label(*state),
        })),
        ObligationRecord::TurnExecution {
            operation_id,
            session_id,
            turn_id,
            state,
        } => Ok(serde_json::json!({
            "schema":"turn_execution_obligation_v1",
            "operation_id":operation_id,
            "session_id":session_id,
            "turn_id":turn_id,
            "state":obligation_state_label(*state),
        })),
        ObligationRecord::TerminalCommit {
            operation_id,
            session_id,
            turn_id,
            terminal_identity,
            state,
        } => Ok(serde_json::json!({
            "schema":"terminal_commit_obligation_v1",
            "operation_id":operation_id,
            "session_id":session_id,
            "turn_id":turn_id,
            "terminal_identity":terminal_identity,
            "state":obligation_state_label(*state),
        })),
        ObligationRecord::RecoveryReserved {
            recovery_id,
            effect_identity,
            state,
        } => Ok(serde_json::json!({
            "schema":"recovery_reserved_obligation_v1",
            "recovery_id":recovery_id,
            "effect_identity":effect_identity,
            "state":obligation_state_label(*state),
        })),
        ObligationRecord::RecoveryCompleted {
            recovery_id,
            effect_identity,
            classification,
            state,
        } => Ok(serde_json::json!({
            "schema":"recovery_completed_obligation_v1",
            "recovery_id":recovery_id,
            "effect_identity":effect_identity,
            "classification":recovery_classification_label(*classification),
            "state":obligation_state_label(*state),
        })),
        ObligationRecord::FeedbackReservation {
            feedback_id,
            attempt_id,
            session_id,
            operation,
            process_instance_id,
        } => Ok(serde_json::json!({
            "schema":"session_feedback_reservation_v1",
            "feedback_id":feedback_id,
            "attempt_id":attempt_id,
            "session_id":session_id,
            "operation":notice_operation_label(*operation),
            "process_instance_id":process_instance_id,
        })),
        ObligationRecord::Feedback {
            feedback_id,
            attempt_id,
            session_id,
            operation,
            actions,
            resolution_identity,
            failure,
        } => Ok(serde_json::json!({
            "schema":"session_feedback_v1",
            "feedback_id":feedback_id,
            "attempt_id":attempt_id,
            "session_id":session_id,
            "operation":notice_operation_label(*operation),
            "actions":actions.iter().map(|action| match action {
                FeedbackActionRecord::Dismiss => "dismiss",
                FeedbackActionRecord::RetryResolution => "retry_resolution",
            }).collect::<Vec<_>>(),
            "resolution_identity":resolution_identity,
            "failure":encode_failure(failure),
        })),
        ObligationRecord::WorkflowExecution { execution } => Ok(serde_json::json!({
            "schema":"workflow_execution_projection_v1",
            "deleted":false,
            "execution":encode_workflow_execution(execution, family)?,
        })),
    }
}

fn decode_obligation(
    object: &Map<String, Value>,
) -> Result<ObligationRecord, StoredRecordCodecError> {
    let family = StoredRecordFamily::Obligation;
    let recovery_action = object
        .get("recovery_action")
        .map(|value| decode_recovery_transition(value, family))
        .transpose()?;
    let observation = object
        .get("authoritative_observation")
        .map(|value| decode_authoritative_observation(value, family))
        .transpose()?;
    let mut record = decode_obligation_base(object, family)?;
    if let Some(recovery_action) = recovery_action {
        record = ObligationRecord::RecoveryTransition {
            original: Box::new(record),
            recovery_action,
        };
    }
    if let Some(observation) = observation {
        if object
            .get("effect_identity")
            .and_then(Value::as_str)
            .is_some_and(|identity| identity != observation.effect_identity)
        {
            return Err(StoredRecordCodecError::Integrity { family });
        }
        record = ObligationRecord::Observed {
            original: Box::new(record),
            observation,
        };
    }
    Ok(record)
}

fn decode_obligation_base(
    object: &Map<String, Value>,
    family: StoredRecordFamily,
) -> Result<ObligationRecord, StoredRecordCodecError> {
    let schema = required_text(object, family, "schema")?;
    // Feedback and workflow projection rows predate the common obligation
    // state member. Their closed variants do not expose this placeholder.
    let state = if matches!(
        schema,
        "session_feedback_reservation_v1"
            | "session_feedback_v1"
            | "workflow_execution_projection_v1"
    ) {
        ObligationStateRecord::Completed
    } else {
        decode_obligation_state(object, family)?
    };
    match schema {
        "send_obligation_v1" => {
            let disposition = match object.get("disposition") {
                Some(Value::String(tag)) if tag == "started_turn" => {
                    SendObligationDispositionRecord::StartedTurn
                }
                Some(Value::String(tag)) if tag == "queued" => {
                    SendObligationDispositionRecord::Queued
                }
                Some(Value::String(tag)) => {
                    return Err(StoredRecordCodecError::Incompatible {
                        family,
                        schema: format!("send_obligation.disposition.{tag}"),
                    })
                }
                // Early V1 rows predate the explicit disposition member.  Their
                // already-persisted queue/turn references are sufficient to apply
                // the total compatibility conversion without guessing a new ID.
                None if object
                    .get("queue_item_id")
                    .and_then(Value::as_str)
                    .is_some() =>
                {
                    SendObligationDispositionRecord::Queued
                }
                None => SendObligationDispositionRecord::StartedTurn,
                Some(_) => return malformed(family),
            };
            let operation_id = string_field(object, family, "operation_id")?;
            Ok(ObligationRecord::Send {
                obligation_id: optional_string(object, "obligation_id")
                    .unwrap_or_else(|| operation_id.clone()),
                operation_id,
                session_id: string_field(object, family, "session_id")?,
                kind: match required_text(object, family, "kind")? {
                    "provider_establish" => SendObligationKindRecord::ProviderEstablish,
                    "turn_execution" => SendObligationKindRecord::TurnExecution,
                    other => {
                        return Err(StoredRecordCodecError::Incompatible {
                            family,
                            schema: format!("send_obligation.kind.{other}"),
                        })
                    }
                },
                disposition,
                human_message_id: optional_string(object, "human_message_id"),
                assistant_message_id: optional_string(object, "assistant_message_id"),
                reserved_turn_id: optional_string(object, "reserved_turn_id"),
                turn_id: optional_string(object, "turn_id"),
                dependency_obligation_ids: object
                    .get("dependency_obligation_ids")
                    .and_then(Value::as_array)
                    .map(|values| {
                        values
                            .iter()
                            .map(|value| {
                                value
                                    .as_str()
                                    .map(str::to_string)
                                    .ok_or(StoredRecordCodecError::Malformed { family })
                            })
                            .collect::<Result<Vec<_>, _>>()
                    })
                    .transpose()?
                    .unwrap_or_default(),
                canonical_payload: optional_string(object, "canonical_payload").unwrap_or_default(),
                state,
            })
        }
        "permission_response_obligation_v1" => {
            let response = decode_permission_response(
                object
                    .get("exact_response")
                    .ok_or(StoredRecordCodecError::MissingReference {
                        family,
                        field: "exact_response",
                    })?,
                family,
            )?;
            if object.get("request_id").and_then(Value::as_str)
                != Some(response.request_id.as_str())
            {
                return Err(StoredRecordCodecError::Integrity { family });
            }
            Ok(ObligationRecord::PermissionResponse {
                operation_id: string_field(object, family, "operation_id")?,
                effect_identity: optional_string(object, "effect_identity").unwrap_or_else(|| {
                    format!(
                        "permission-response:{}",
                        object
                            .get("operation_id")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                    )
                }),
                session_id: string_field(object, family, "session_id")?,
                turn_id: string_field(object, family, "turn_id")?,
                response,
                owner_access: object.get("owner_access").and_then(Value::as_bool).ok_or(
                    StoredRecordCodecError::MissingReference {
                        family,
                        field: "owner_access",
                    },
                )?,
                from_runtime_state: object
                    .get("from_runtime_state")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                state,
            })
        }
        "stop_interrupt_obligation_v1" => Ok(ObligationRecord::StopInterrupt {
            operation_id: string_field(object, family, "operation_id")?,
            session_id: string_field(object, family, "session_id")?,
            turn_id: string_field(object, family, "turn_id")?,
            expected_revision: optional_u64(object, "expected_revision").unwrap_or(0),
            deadline_ms: optional_i64(object, "deadline_ms").unwrap_or(0),
            state,
        }),
        "session_close_obligation_v1" => Ok(ObligationRecord::SessionClose {
            obligation_id: string_field(object, family, "obligation_id")?,
            operation_id: string_field(object, family, "operation_id")?,
            session_id: string_field(object, family, "session_id")?,
            action: object
                .get("action")
                .and_then(Value::as_str)
                .map(|raw| decode_lifecycle_action_record_label(raw, family))
                .transpose()?
                .unwrap_or(SessionLifecycleRecordAction::Close),
            state,
        }),
        "backend_session_recovery_obligation_v1" => {
            let detail = if object.contains_key("error_sha256") {
                BackendSessionRecoveryObligationRecord::Failed {
                    error_sha256: hash_field(object, family, "error_sha256")?,
                    failed_at_bits: u64_field(object, family, "failed_at_bits")?,
                }
            } else if object.contains_key("backend_session_id") {
                BackendSessionRecoveryObligationRecord::Completed {
                    old_provider_session_generation: u64_field(
                        object,
                        family,
                        "old_provider_session_generation",
                    )?,
                    provider_session_generation: u64_field(
                        object,
                        family,
                        "provider_session_generation",
                    )?,
                    backend_session_id: string_field(object, family, "backend_session_id")?,
                    completed_at_bits: u64_field(object, family, "completed_at_bits")?,
                }
            } else {
                BackendSessionRecoveryObligationRecord::EffectReserved {
                    old_provider_session_generation: u64_field(
                        object,
                        family,
                        "old_provider_session_generation",
                    )?,
                    reason: match required_text(object, family, "reason")? {
                        "resume_mismatch" => BackendSessionRecoveryReason::ResumeMismatch,
                        "backend_session_lost" => BackendSessionRecoveryReason::BackendSessionLost,
                        other => {
                            return Err(StoredRecordCodecError::Incompatible {
                                family,
                                schema: format!("backend_recovery_reason.{other}"),
                            })
                        }
                    },
                    reserved_at_bits: u64_field(object, family, "reserved_at_bits")?,
                }
            };
            Ok(ObligationRecord::BackendSessionRecovery {
                session_id: string_field(object, family, "session_id")?,
                recovery_id: string_field(object, family, "recovery_id")?,
                detail,
                state,
            })
        }
        "workflow_shutdown_effect_v1" => Ok(ObligationRecord::WorkflowShutdown {
            operation_id: string_field(object, family, "operation_id")?,
            effect_identity: string_field(object, family, "effect_identity")?,
            owner_revision: required_i64(object, family, "owner_revision")?,
            execution_id: string_field(object, family, "execution_id")?,
            state,
        }),
        "workflow_turn_completion_obligation_v1" => {
            let detail = if object.contains_key("completed_at_bits") {
                WorkflowTurnCompletionObligationRecord::Completed {
                    completed_at_bits: u64_field(object, family, "completed_at_bits")?,
                }
            } else {
                WorkflowTurnCompletionObligationRecord::Pending {
                    workflow_context: Box::new(decode_workflow_context(
                        object.get("workflow_context").ok_or(
                            StoredRecordCodecError::MissingReference {
                                family,
                                field: "workflow_context",
                            },
                        )?,
                        family,
                    )?),
                    message_id: string_field(object, family, "message_id")?,
                    exit_code: required_i64(object, family, "exit_code")?,
                    failure_signal: object
                        .get("failure_signal")
                        .and_then(Value::as_str)
                        .map(|raw| match raw {
                            "model_refusal" => Ok(WorkflowTurnFailureSignalRecord::ModelRefusal),
                            other => Err(StoredRecordCodecError::Incompatible {
                                family,
                                schema: format!("workflow_failure_signal.{other}"),
                            }),
                        })
                        .transpose()?,
                    token_usage: object
                        .get("token_usage")
                        .map(|usage| {
                            let usage = usage
                                .as_object()
                                .ok_or(StoredRecordCodecError::Malformed { family })?;
                            Ok(TurnTokenUsage {
                                input_tokens: u64_field(usage, family, "input_tokens")?,
                                output_tokens: u64_field(usage, family, "output_tokens")?,
                            })
                        })
                        .transpose()?,
                    interrupted: object.get("interrupted").and_then(Value::as_bool).ok_or(
                        StoredRecordCodecError::MissingReference {
                            family,
                            field: "interrupted",
                        },
                    )?,
                }
            };
            Ok(ObligationRecord::WorkflowTurnCompletion {
                session_id: string_field(object, family, "session_id")?,
                turn_id: string_field(object, family, "turn_id")?,
                terminal_identity: string_field(object, family, "terminal_identity")?,
                notification_sha256: hash_field(object, family, "notification_sha256")?,
                detail,
                state,
            })
        }
        "recovery_publication_obligation_v1" => {
            let detail = if object.contains_key("published_at_bits") {
                RecoveryPublicationObligationRecord::Completed {
                    published_at_bits: u64_field(object, family, "published_at_bits")?,
                }
            } else {
                RecoveryPublicationObligationRecord::Pending {
                    pending_message: decode_publication_message(
                        object.get("pending_message").ok_or(
                            StoredRecordCodecError::MissingReference {
                                family,
                                field: "pending_message",
                            },
                        )?,
                        family,
                    )?,
                }
            };
            Ok(ObligationRecord::RecoveryPublication {
                session_id: string_field(object, family, "session_id")?,
                recovery_id: string_field(object, family, "recovery_id")?,
                message_id: string_field(object, family, "message_id")?,
                source_obligation_id: string_field(object, family, "source_obligation_id")?,
                detail,
                state,
            })
        }
        "provider_establish_obligation_v1" => Ok(ObligationRecord::ProviderEstablish {
            operation_id: string_field(object, family, "operation_id")?,
            effect_identity: string_field(object, family, "effect_identity")?,
            session_id: string_field(object, family, "session_id")?,
            state,
        }),
        "turn_execution_obligation_v1" => Ok(ObligationRecord::TurnExecution {
            operation_id: string_field(object, family, "operation_id")?,
            session_id: string_field(object, family, "session_id")?,
            turn_id: string_field(object, family, "turn_id")?,
            state,
        }),
        "terminal_commit_obligation_v1" => Ok(ObligationRecord::TerminalCommit {
            operation_id: string_field(object, family, "operation_id")?,
            session_id: string_field(object, family, "session_id")?,
            turn_id: string_field(object, family, "turn_id")?,
            terminal_identity: string_field(object, family, "terminal_identity")?,
            state,
        }),
        "recovery_reserved_obligation_v1" => Ok(ObligationRecord::RecoveryReserved {
            recovery_id: string_field(object, family, "recovery_id")?,
            effect_identity: string_field(object, family, "effect_identity")?,
            state,
        }),
        "recovery_completed_obligation_v1" => Ok(ObligationRecord::RecoveryCompleted {
            recovery_id: string_field(object, family, "recovery_id")?,
            effect_identity: string_field(object, family, "effect_identity")?,
            classification: decode_recovery_classification(
                required_text(object, family, "classification")?,
                family,
            )?,
            state,
        }),
        "session_feedback_reservation_v1" => Ok(ObligationRecord::FeedbackReservation {
            feedback_id: string_field(object, family, "feedback_id")?,
            attempt_id: string_field(object, family, "attempt_id")?,
            session_id: string_field(object, family, "session_id")?,
            operation: decode_notice_operation(
                required_text(object, family, "operation")?,
                family,
            )?,
            process_instance_id: string_field(object, family, "process_instance_id")?,
        }),
        "session_feedback_v1" => Ok(ObligationRecord::Feedback {
            feedback_id: string_field(object, family, "feedback_id")?,
            attempt_id: string_field(object, family, "attempt_id")?,
            session_id: string_field(object, family, "session_id")?,
            operation: decode_notice_operation(
                required_text(object, family, "operation")?,
                family,
            )?,
            actions: object
                .get("actions")
                .and_then(Value::as_array)
                .ok_or(StoredRecordCodecError::MissingReference {
                    family,
                    field: "actions",
                })?
                .iter()
                .map(|value| match value.as_str() {
                    Some("dismiss") => Ok(FeedbackActionRecord::Dismiss),
                    Some("retry_resolution") => Ok(FeedbackActionRecord::RetryResolution),
                    Some(other) => Err(StoredRecordCodecError::Incompatible {
                        family,
                        schema: format!("feedback_action.{other}"),
                    }),
                    None => malformed(family),
                })
                .collect::<Result<Vec<_>, _>>()?,
            resolution_identity: optional_string(object, "resolution_identity"),
            failure: decode_failure(
                object
                    .get("failure")
                    .ok_or(StoredRecordCodecError::MissingReference {
                        family,
                        field: "failure",
                    })?,
                family,
            )?,
        }),
        "workflow_execution_projection_v1" => Ok(ObligationRecord::WorkflowExecution {
            execution: decode_workflow_execution(
                object
                    .get("execution")
                    .ok_or(StoredRecordCodecError::MissingReference {
                        family,
                        field: "execution",
                    })?,
                family,
            )?,
        }),
        other => Err(StoredRecordCodecError::Incompatible {
            family,
            schema: other.to_string(),
        }),
    }
}

fn notice_operation_label(value: AgentSessionNoticeOperationRecord) -> &'static str {
    match value {
        AgentSessionNoticeOperationRecord::Send => "send",
        AgentSessionNoticeOperationRecord::LoadSession => "load_session",
        AgentSessionNoticeOperationRecord::LoadOlder => "load_older",
        AgentSessionNoticeOperationRecord::CancelQueue => "cancel_queue",
        AgentSessionNoticeOperationRecord::ResumeQueue => "resume_queue",
        AgentSessionNoticeOperationRecord::CloseSession => "close_session",
        AgentSessionNoticeOperationRecord::RestoreSession => "restore_session",
        AgentSessionNoticeOperationRecord::ArchiveSession => "archive_session",
        AgentSessionNoticeOperationRecord::ForkSession => "fork_session",
        AgentSessionNoticeOperationRecord::SetTitle => "set_title",
        AgentSessionNoticeOperationRecord::RespondPermission => "respond_permission",
        AgentSessionNoticeOperationRecord::SetBackend => "set_backend",
    }
}

fn decode_notice_operation(
    raw: &str,
    family: StoredRecordFamily,
) -> Result<AgentSessionNoticeOperationRecord, StoredRecordCodecError> {
    match raw {
        "send" => Ok(AgentSessionNoticeOperationRecord::Send),
        "load_session" => Ok(AgentSessionNoticeOperationRecord::LoadSession),
        "load_older" => Ok(AgentSessionNoticeOperationRecord::LoadOlder),
        "cancel_queue" => Ok(AgentSessionNoticeOperationRecord::CancelQueue),
        "resume_queue" => Ok(AgentSessionNoticeOperationRecord::ResumeQueue),
        "close_session" => Ok(AgentSessionNoticeOperationRecord::CloseSession),
        "restore_session" => Ok(AgentSessionNoticeOperationRecord::RestoreSession),
        "archive_session" => Ok(AgentSessionNoticeOperationRecord::ArchiveSession),
        "fork_session" => Ok(AgentSessionNoticeOperationRecord::ForkSession),
        "set_title" => Ok(AgentSessionNoticeOperationRecord::SetTitle),
        "respond_permission" => Ok(AgentSessionNoticeOperationRecord::RespondPermission),
        "set_backend" => Ok(AgentSessionNoticeOperationRecord::SetBackend),
        other => Err(StoredRecordCodecError::Incompatible {
            family,
            schema: format!("feedback_operation.{other}"),
        }),
    }
}

fn finite_from_bits(bits: u64, family: StoredRecordFamily) -> Result<f64, StoredRecordCodecError> {
    let value = f64::from_bits(bits);
    if !value.is_finite() {
        return malformed(family);
    }
    Ok(value)
}

fn decode_finite(
    object: &Map<String, Value>,
    field: &'static str,
    family: StoredRecordFamily,
) -> Result<u64, StoredRecordCodecError> {
    object
        .get(field)
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
        .map(f64::to_bits)
        .ok_or(StoredRecordCodecError::MissingReference { family, field })
}

fn encode_workflow_execution(
    value: &WorkflowExecutionMetadataRecord,
    family: StoredRecordFamily,
) -> Result<Value, StoredRecordCodecError> {
    Ok(serde_json::json!({
        "executionId":value.execution_id,
        "workflowName":value.workflow_name,
        "status":value.status.as_str(),
        "worktreePath":value.worktree_path,
        "currentNode":value.current_node,
        "createdFrom":value.created_from.as_public_value(),
        "startedAt":finite_from_bits(value.started_at_bits, family)?,
        "updatedAt":finite_from_bits(value.updated_at_bits, family)?,
        "completedAt":value.completed_at_bits
            .map(|bits| finite_from_bits(bits, family))
            .transpose()?,
        "errorReason":value.error_reason,
        "interruptionReason":value.interruption_reason.map(|reason| reason.as_str()),
        "resumeFromNode":value.resume_from_node,
        "totalTokenUsage":{
            "inputTokens":value.total_token_usage.input_tokens,
            "outputTokens":value.total_token_usage.output_tokens,
        },
    }))
}

fn decode_workflow_execution(
    value: &Value,
    family: StoredRecordFamily,
) -> Result<WorkflowExecutionMetadataRecord, StoredRecordCodecError> {
    let object = value
        .as_object()
        .ok_or(StoredRecordCodecError::Malformed { family })?;
    let status = match required_text(object, family, "status")? {
        "running" => ExecutionStatus::Running,
        "waiting_approval" => ExecutionStatus::WaitingApproval,
        "completed" => ExecutionStatus::Completed,
        "failed" => ExecutionStatus::Failed,
        "aborted" => ExecutionStatus::Aborted,
        "interrupted" => ExecutionStatus::Interrupted,
        other => {
            return Err(StoredRecordCodecError::Incompatible {
                family,
                schema: format!("workflow_execution_status.{other}"),
            })
        }
    };
    let created_from = match required_text(object, family, "createdFrom")? {
        "desktop_ui" | "desktop-ui" => ExecutionOrigin::DesktopUi,
        "cli" => ExecutionOrigin::Cli,
        "agent" => ExecutionOrigin::Agent,
        "api" => ExecutionOrigin::Api,
        other => {
            return Err(StoredRecordCodecError::Incompatible {
                family,
                schema: format!("workflow_execution_origin.{other}"),
            })
        }
    };
    let interruption_reason = object
        .get("interruptionReason")
        .and_then(Value::as_str)
        .map(|raw| {
            ExecutionInterruptionReason::from_reason(raw).ok_or_else(|| {
                StoredRecordCodecError::Incompatible {
                    family,
                    schema: format!("workflow_interruption_reason.{raw}"),
                }
            })
        })
        .transpose()?;
    let usage = object
        .get("totalTokenUsage")
        .and_then(Value::as_object)
        .ok_or(StoredRecordCodecError::MissingReference {
            family,
            field: "totalTokenUsage",
        })?;
    Ok(WorkflowExecutionMetadataRecord {
        execution_id: string_field(object, family, "executionId")?,
        workflow_name: string_field(object, family, "workflowName")?,
        status,
        worktree_path: string_field(object, family, "worktreePath")?,
        current_node: optional_string(object, "currentNode"),
        created_from,
        started_at_bits: decode_finite(object, "startedAt", family)?,
        updated_at_bits: decode_finite(object, "updatedAt", family)?,
        completed_at_bits: object
            .get("completedAt")
            .filter(|value| !value.is_null())
            .map(|_| decode_finite(object, "completedAt", family))
            .transpose()?,
        error_reason: optional_string(object, "errorReason"),
        interruption_reason,
        resume_from_node: optional_string(object, "resumeFromNode"),
        total_token_usage: WorkflowTokenUsage {
            input_tokens: u64_field(usage, family, "inputTokens")?,
            output_tokens: u64_field(usage, family, "outputTokens")?,
        },
    })
}

fn known_obligation_fields(schema: &str) -> &'static [&'static str] {
    // This superset is deliberate: a field known to any closed V1 obligation
    // is never carried forward as an additive field after changing variants.
    // Unknown members remain exact raw fragments.
    let _ = schema;
    &[
        "schema",
        "state",
        "obligation_id",
        "operation_id",
        "session_id",
        "turn_id",
        "kind",
        "disposition",
        "human_message_id",
        "assistant_message_id",
        "canonical_payload",
        "reserved_turn_id",
        "dependency_obligation_ids",
        "effect_identity",
        "request_id",
        "exact_response",
        "owner_access",
        "from_runtime_state",
        "expected_revision",
        "deadline_ms",
        "action",
        "recovery_id",
        "old_provider_session_generation",
        "provider_session_generation",
        "backend_session_id",
        "reason",
        "reserved_at_bits",
        "completed_at_bits",
        "error_sha256",
        "failed_at_bits",
        "owner_revision",
        "execution_id",
        "terminal_identity",
        "notification_sha256",
        "workflow_context",
        "message_id",
        "exit_code",
        "failure_signal",
        "token_usage",
        "interrupted",
        "source_obligation_id",
        "pending_message",
        "published_at_bits",
        "safe_actions",
        "queue_item_id",
        "input_ref",
        "request",
        "operation_kind",
        "known_observation",
        "missing_evidence",
        "classification",
        "recovery_action",
        "authoritative_observation",
        "feedback_id",
        "attempt_id",
        "process_instance_id",
        "operation",
        "actions",
        "resolution_identity",
        "failure",
        "deleted",
        "execution",
    ]
}

fn decode_recovery_attempt(
    object: &Map<String, Value>,
) -> Result<RecoveryAttemptRecord, StoredRecordCodecError> {
    let family = StoredRecordFamily::RecoveryAction;
    match required_text(object, family, "schema")? {
        "feedback_retry_attempt_v1" => Ok(RecoveryAttemptRecord::FeedbackRetry {
            feedback_id: string_field(object, family, "feedback_id")?,
            origin_revision: u64_field(object, family, "origin_revision")?,
            resolution_identity: string_field(object, family, "resolution_identity")?,
            state: decode_obligation_state(object, family)?,
        }),
        "recovery_action_attempt_v1" if object.contains_key("resource_ref") => {
            let code = required_i64(object, family, "exit_code")?;
            let intent = match object.get("intent").and_then(Value::as_str) {
                Some("exit") | None => QuitIntent::Exit { code },
                Some("restart") => QuitIntent::Restart { code },
                Some(other) => {
                    return Err(StoredRecordCodecError::Incompatible {
                        family,
                        schema: format!("shutdown_recovery_intent.{other}"),
                    })
                }
            };
            Ok(RecoveryAttemptRecord::ShutdownTarget {
                resource_ref: string_field(object, family, "resource_ref")?,
                plan: ShutdownPlanKey {
                    shutdown_id: string_field(object, family, "shutdown_id")?,
                },
                ordinal: required_i64(object, family, "ordinal")?,
                target_key: string_field(object, family, "target_key")?,
                origin_revision: u64_field(object, family, "origin_revision")?,
                action: decode_recovery_action_kind(
                    required_text(object, family, "action")?,
                    family,
                )?,
                effect_identity_sha256: hash_field(object, family, "effect_identity_sha256")?,
                intent,
                state: decode_obligation_state(object, family)?,
                failure: object
                    .get("failure")
                    .filter(|value| !value.is_null())
                    .map(|value| decode_failure(value, family))
                    .transpose()?,
            })
        }
        "recovery_action_attempt_v1" => Ok(RecoveryAttemptRecord::Obligation {
            obligation_id: string_field(object, family, "obligation_id")?,
            origin_revision: u64_field(object, family, "origin_revision")?,
            action: decode_recovery_action_kind(required_text(object, family, "action")?, family)?,
            effect_identity: string_field(object, family, "effect_identity")?,
            state: decode_obligation_state(object, family)?,
            failure: object
                .get("failure")
                .filter(|value| !value.is_null())
                .map(|value| decode_failure(value, family))
                .transpose()?,
        }),
        schema => Err(StoredRecordCodecError::Incompatible {
            family,
            schema: schema.to_string(),
        }),
    }
}

fn encode_recovery_attempt(value: &RecoveryAttemptRecord) -> Result<Value, StoredRecordCodecError> {
    let mut object = Map::new();
    match value {
        RecoveryAttemptRecord::Obligation {
            obligation_id,
            origin_revision,
            action,
            effect_identity,
            state,
            failure,
        } => {
            object.insert(
                "schema".into(),
                Value::String("recovery_action_attempt_v1".into()),
            );
            object.insert("obligation_id".into(), Value::String(obligation_id.clone()));
            object.insert("origin_revision".into(), Value::from(*origin_revision));
            object.insert(
                "action".into(),
                Value::String(recovery_action_label(*action).into()),
            );
            object.insert(
                "effect_identity".into(),
                Value::String(effect_identity.clone()),
            );
            object.insert(
                "state".into(),
                Value::String(obligation_state_label(*state).into()),
            );
            if let Some(failure) = failure {
                object.insert("failure".into(), encode_failure(failure));
            }
        }
        RecoveryAttemptRecord::ShutdownTarget {
            resource_ref,
            plan,
            ordinal,
            target_key,
            origin_revision,
            action,
            effect_identity_sha256,
            intent,
            state,
            failure,
        } => {
            let (intent_label, exit_code) = quit_fields(*intent);
            object.insert(
                "schema".into(),
                Value::String("recovery_action_attempt_v1".into()),
            );
            object.insert("resource_ref".into(), Value::String(resource_ref.clone()));
            object.insert(
                "shutdown_id".into(),
                Value::String(plan.shutdown_id.clone()),
            );
            object.insert("ordinal".into(), Value::from(*ordinal));
            object.insert("target_key".into(), Value::String(target_key.clone()));
            object.insert("origin_revision".into(), Value::from(*origin_revision));
            object.insert(
                "action".into(),
                Value::String(recovery_action_label(*action).into()),
            );
            object.insert(
                "effect_identity_sha256".into(),
                Value::String(hex::encode(effect_identity_sha256)),
            );
            object.insert("intent".into(), Value::String(intent_label.into()));
            object.insert("exit_code".into(), Value::from(exit_code));
            object.insert(
                "state".into(),
                Value::String(obligation_state_label(*state).into()),
            );
            if let Some(failure) = failure {
                object.insert("failure".into(), encode_failure(failure));
            }
        }
        RecoveryAttemptRecord::FeedbackRetry {
            feedback_id,
            origin_revision,
            resolution_identity,
            state,
        } => {
            object.insert(
                "schema".into(),
                Value::String("feedback_retry_attempt_v1".into()),
            );
            object.insert("feedback_id".into(), Value::String(feedback_id.clone()));
            object.insert("origin_revision".into(), Value::from(*origin_revision));
            object.insert(
                "resolution_identity".into(),
                Value::String(resolution_identity.clone()),
            );
            object.insert(
                "state".into(),
                Value::String(obligation_state_label(*state).into()),
            );
        }
    }
    Ok(Value::Object(object))
}

fn known_recovery_attempt_fields(_: &str) -> &'static [&'static str] {
    &[
        "schema",
        "obligation_id",
        "origin_revision",
        "action",
        "effect_identity",
        "state",
        "failure",
        "resource_ref",
        "shutdown_id",
        "ordinal",
        "target_key",
        "effect_identity_sha256",
        "intent",
        "exit_code",
        "feedback_id",
        "resolution_identity",
    ]
}

fn recovery_outcome_label(value: RecoveryResultOutcomeRecord) -> &'static str {
    match value {
        RecoveryResultOutcomeRecord::Pending => "pending",
        RecoveryResultOutcomeRecord::Terminal => "terminal",
        RecoveryResultOutcomeRecord::Unchanged => "unchanged",
    }
}

fn decode_recovery_outcome(
    raw: &str,
    family: StoredRecordFamily,
) -> Result<RecoveryResultOutcomeRecord, StoredRecordCodecError> {
    match raw {
        "pending" => Ok(RecoveryResultOutcomeRecord::Pending),
        "terminal" => Ok(RecoveryResultOutcomeRecord::Terminal),
        "unchanged" => Ok(RecoveryResultOutcomeRecord::Unchanged),
        other => Err(StoredRecordCodecError::Incompatible {
            family,
            schema: format!("recovery_outcome.{other}"),
        }),
    }
}

fn recovery_result_pair_is_valid(
    outcome: RecoveryResultOutcomeRecord,
    classification: RecoveryResultClassification,
) -> bool {
    matches!(
        (outcome, classification),
        (
            RecoveryResultOutcomeRecord::Pending,
            RecoveryResultClassification::Pending
                | RecoveryResultClassification::ConfirmedNoEffect
                | RecoveryResultClassification::Ambiguous
        ) | (
            RecoveryResultOutcomeRecord::Terminal,
            RecoveryResultClassification::Succeeded
                | RecoveryResultClassification::CancelledBeforeEffect
        ) | (
            RecoveryResultOutcomeRecord::Unchanged,
            RecoveryResultClassification::Unchanged
        )
    )
}

fn encode_recovery_resource_view(
    value: &RecoveryResourceViewRecord,
) -> Result<String, StoredRecordCodecError> {
    let family = StoredRecordFamily::RecoveryResult;
    match value {
        RecoveryResourceViewRecord::SafeSummary(value) => Ok(value.clone()),
        RecoveryResourceViewRecord::Operation { kind, operation_id } => {
            serde_json::to_string(&serde_json::json!({
                "schema":"operation_recovery_result_v1",
                "kind":kind.label(),
                "operation_id":operation_id,
            }))
            .map_err(|_| StoredRecordCodecError::Malformed { family })
        }
        RecoveryResourceViewRecord::Session { session_id } => serde_json::to_string(
            &serde_json::json!({"schema":"session_recovery_result_v1","session_id":session_id}),
        )
        .map_err(|_| StoredRecordCodecError::Malformed { family }),
        RecoveryResourceViewRecord::BackendRecovery {
            session_id,
            recovery_id,
        } => serde_json::to_string(&serde_json::json!({
            "schema":"backend_recovery_result_v1",
            "session_id":session_id,
            "recovery_id":recovery_id,
        }))
        .map_err(|_| StoredRecordCodecError::Malformed { family }),
        RecoveryResourceViewRecord::ShutdownTarget {
            plan,
            ordinal,
            target_id,
            state,
        } => serde_json::to_string(&serde_json::json!({
            "schema":"shutdown_target_recovery_result_v1",
            "shutdown_id":plan.shutdown_id,
            "ordinal":ordinal,
            "target_key":target_id,
            "state":shutdown_target_state_label(*state),
        }))
        .map_err(|_| StoredRecordCodecError::Malformed { family }),
        RecoveryResourceViewRecord::ReconciliationRequired { failure } => {
            serde_json::to_string(&serde_json::json!({
                "schema":"reconciliation_required_recovery_result_v1",
                "failure":encode_failure(failure),
            }))
            .map_err(|_| StoredRecordCodecError::Malformed { family })
        }
    }
}

fn canonical_recovery_result_sha256(
    outcome: RecoveryResultOutcomeRecord,
    classification: RecoveryResultClassification,
    resource_revision: u64,
    resource_view: &str,
) -> Result<[u8; 32], StoredRecordCodecError> {
    let family = StoredRecordFamily::RecoveryResult;
    serde_json::to_vec(&serde_json::json!({
        "schema":"recovery_action_canonical_result_v1",
        "outcome":recovery_outcome_label(outcome),
        "classification":recovery_classification_label(classification),
        "resource_revision":resource_revision,
        "resource_view":resource_view,
    }))
    .map(|bytes| Sha256::digest(bytes).into())
    .map_err(|_| StoredRecordCodecError::Malformed { family })
}

pub(crate) fn canonicalize_recovery_result_record(
    outcome: RecoveryResultOutcomeRecord,
    classification: RecoveryResultClassification,
    resource_revision: u64,
    resource_view: RecoveryResourceViewRecord,
) -> Result<RecoveryResultRecord, StoredRecordCodecError> {
    let family = StoredRecordFamily::RecoveryResult;
    if !recovery_result_pair_is_valid(outcome, classification) {
        return Err(StoredRecordCodecError::Incompatible {
            family,
            schema: "recovery_action_result_v1.outcome_classification".into(),
        });
    }
    let resource_view = encode_recovery_resource_view(&resource_view)?;
    let canonical_result_sha256 = canonical_recovery_result_sha256(
        outcome,
        classification,
        resource_revision,
        &resource_view,
    )?;
    Ok(RecoveryResultRecord::Action(RecoveryActionResultRecord {
        outcome,
        classification,
        resource_revision,
        canonical_result_sha256,
        // Preserve the exact gateway-owned public V1 rendering that the hash
        // authenticates. The semantic input never crosses back as persistence
        // text through the repository port.
        resource_view: RecoveryResourceViewRecord::SafeSummary(resource_view),
    }))
}

fn decode_recovery_result(
    object: &Map<String, Value>,
) -> Result<RecoveryResultRecord, StoredRecordCodecError> {
    let family = StoredRecordFamily::RecoveryResult;
    match required_text(object, family, "schema")? {
        "feedback_retry_result_v1" => Ok(RecoveryResultRecord::FeedbackRetry {
            feedback_id: string_field(object, family, "feedback_id")?,
            resource_revision: u64_field(object, family, "resource_revision")?,
            resolved: match required_text(object, family, "outcome")? {
                "resolved" => true,
                "failed" => false,
                other => {
                    return Err(StoredRecordCodecError::Incompatible {
                        family,
                        schema: format!("feedback_retry_outcome.{other}"),
                    })
                }
            },
        }),
        "recovery_action_result_v1" => {
            let outcome =
                decode_recovery_outcome(required_text(object, family, "outcome")?, family)?;
            let classification = decode_recovery_classification(
                required_text(object, family, "classification")?,
                family,
            )?;
            if !recovery_result_pair_is_valid(outcome, classification) {
                return Err(StoredRecordCodecError::Incompatible {
                    family,
                    schema: "recovery_action_result_v1.outcome_classification".into(),
                });
            }
            let resource_revision = u64_field(object, family, "resource_revision")?;
            let canonical_result_sha256 = hash_field(object, family, "canonical_result_sha256")?;
            let resource_view = string_field(object, family, "resource_view")?;
            let expected = canonical_recovery_result_sha256(
                outcome,
                classification,
                resource_revision,
                &resource_view,
            )?;
            if expected != canonical_result_sha256 {
                return Err(StoredRecordCodecError::Integrity { family });
            }
            Ok(RecoveryResultRecord::Action(RecoveryActionResultRecord {
                outcome,
                classification,
                resource_revision,
                canonical_result_sha256,
                // The V1 contract deliberately signs the exact safe public
                // string. Keeping it opaque here preserves those signed bytes.
                resource_view: RecoveryResourceViewRecord::SafeSummary(resource_view),
            }))
        }
        schema => Err(StoredRecordCodecError::Incompatible {
            family,
            schema: schema.to_string(),
        }),
    }
}

fn encode_recovery_result(value: &RecoveryResultRecord) -> Result<Value, StoredRecordCodecError> {
    let family = StoredRecordFamily::RecoveryResult;
    match value {
        RecoveryResultRecord::FeedbackRetry {
            feedback_id,
            resource_revision,
            resolved,
        } => Ok(serde_json::json!({
            "schema":"feedback_retry_result_v1",
            "feedback_id":feedback_id,
            "resource_revision":resource_revision,
            "outcome":if *resolved { "resolved" } else { "failed" },
        })),
        RecoveryResultRecord::Action(result) => {
            if !recovery_result_pair_is_valid(result.outcome, result.classification) {
                return Err(StoredRecordCodecError::Incompatible {
                    family,
                    schema: "recovery_action_result_v1.outcome_classification".into(),
                });
            }
            let resource_view = encode_recovery_resource_view(&result.resource_view)?;
            let expected = canonical_recovery_result_sha256(
                result.outcome,
                result.classification,
                result.resource_revision,
                &resource_view,
            )?;
            if expected != result.canonical_result_sha256 {
                return Err(StoredRecordCodecError::Integrity { family });
            }
            Ok(serde_json::json!({
                "schema":"recovery_action_result_v1",
                "outcome":recovery_outcome_label(result.outcome),
                "classification":recovery_classification_label(result.classification),
                "resource_revision":result.resource_revision,
                "canonical_result_sha256":hex::encode(result.canonical_result_sha256),
                "resource_view":resource_view,
            }))
        }
    }
}

fn known_recovery_result_fields(_: &str) -> &'static [&'static str] {
    &[
        "schema",
        "outcome",
        "classification",
        "resource_revision",
        "canonical_result_sha256",
        "resource_view",
        "feedback_id",
    ]
}

fn shutdown_target_state_label(value: ShutdownTargetStateRecord) -> &'static str {
    match value {
        ShutdownTargetStateRecord::Prepared => "prepared",
        ShutdownTargetStateRecord::EffectReserved => "effect_reserved",
        ShutdownTargetStateRecord::Completed => "completed",
        ShutdownTargetStateRecord::Failed => "failed",
        ShutdownTargetStateRecord::ReconciliationRequired => "reconciliation_required",
    }
}

fn decode_shutdown_target_state(
    raw: &str,
    family: StoredRecordFamily,
) -> Result<ShutdownTargetStateRecord, StoredRecordCodecError> {
    match raw {
        "prepared" => Ok(ShutdownTargetStateRecord::Prepared),
        "effect_reserved" => Ok(ShutdownTargetStateRecord::EffectReserved),
        "completed" => Ok(ShutdownTargetStateRecord::Completed),
        "failed" => Ok(ShutdownTargetStateRecord::Failed),
        "reconciliation_required" => Ok(ShutdownTargetStateRecord::ReconciliationRequired),
        other => Err(StoredRecordCodecError::Incompatible {
            family,
            schema: format!("shutdown_target_state.{other}"),
        }),
    }
}

fn encode_shutdown_plan(value: &ShutdownPlanRecord) -> Result<Value, StoredRecordCodecError> {
    let (intent, exit_code) = quit_fields(value.intent);
    let mut object = Map::new();
    object.insert(
        "schema".into(),
        Value::String("shutdown_plan_summary_v1".into()),
    );
    object.insert(
        "operation_id".into(),
        Value::String(value.operation_id.clone()),
    );
    object.insert("intent".into(), Value::String(intent.into()));
    object.insert("exit_code".into(), Value::from(exit_code));
    object.insert("t0_ms".into(), Value::from(value.t0_ms));
    if let Some(field) = value.preparation_cutoff_ms {
        object.insert("preparation_cutoff_ms".into(), Value::from(field));
    }
    object.insert("deadline_ms".into(), Value::from(value.deadline_ms));
    for (key, field) in [
        ("target_count", value.target_count),
        ("prepared_count", value.prepared_count),
        ("effect_reserved_count", value.effect_reserved_count),
        ("terminal_count", value.terminal_count),
        ("completed_count", value.completed_count),
        ("unresolved_count", value.unresolved_count),
        ("recovery_snapshot_count", value.recovery_snapshot_count),
        ("shutdown_effect_count", value.shutdown_effect_count),
    ] {
        if let Some(field) = field {
            object.insert(key.into(), Value::from(field));
        }
    }
    if let Some(field) = &value.recovery_snapshot_id {
        object.insert("recovery_snapshot_id".into(), Value::String(field.clone()));
    }
    object.insert(
        "process_instance_id".into(),
        Value::String(value.process_instance_id.clone()),
    );
    if let Some(outcome) = value.outcome {
        object.insert(
            "outcome".into(),
            Value::String(
                match outcome {
                    ShutdownOutcomeRecord::Completed => "completed",
                    ShutdownOutcomeRecord::AbortedBeforeActivation => "aborted_before_activation",
                    ShutdownOutcomeRecord::ReconciliationRequired => "reconciliation_required",
                }
                .into(),
            ),
        );
    }
    if let Some(failure) = &value.failure {
        object.insert("failure".into(), encode_failure(failure));
    }
    if let Some(field) = value.admission_open {
        object.insert("admission_open".into(), Value::Bool(field));
    }
    if let Some(field) = value.retry_quit_same_boot {
        object.insert("retry_quit_same_boot".into(), Value::Bool(field));
    }
    Ok(Value::Object(object))
}

fn decode_shutdown_plan(
    object: &Map<String, Value>,
) -> Result<ShutdownPlanRecord, StoredRecordCodecError> {
    let family = StoredRecordFamily::ShutdownPlan;
    let code = optional_i64(object, "exit_code").unwrap_or(0);
    let intent = match required_text(object, family, "intent")? {
        "exit" => QuitIntent::Exit { code },
        "restart" => QuitIntent::Restart { code },
        other => {
            return Err(StoredRecordCodecError::Incompatible {
                family,
                schema: format!("shutdown_intent.{other}"),
            })
        }
    };
    let count = |key: &'static str| {
        let Some(value) = object.get(key).filter(|value| !value.is_null()) else {
            return Ok(None);
        };
        value
            .as_u64()
            .filter(|value| *value <= i64::MAX as u64)
            .map(Some)
            .ok_or(StoredRecordCodecError::MissingReference { family, field: key })
    };
    let outcome = match object.get("outcome").and_then(Value::as_str) {
        None | Some("in_progress") => None,
        Some("completed") => Some(ShutdownOutcomeRecord::Completed),
        Some("aborted_before_activation") => Some(ShutdownOutcomeRecord::AbortedBeforeActivation),
        Some("reconciliation_required" | "exited_with_recovery") => {
            Some(ShutdownOutcomeRecord::ReconciliationRequired)
        }
        Some(other) => {
            return Err(StoredRecordCodecError::Incompatible {
                family,
                schema: format!("shutdown_outcome.{other}"),
            })
        }
    };
    Ok(ShutdownPlanRecord {
        operation_id: string_field(object, family, "operation_id")?,
        intent,
        t0_ms: optional_i64(object, "t0_ms").unwrap_or(0),
        preparation_cutoff_ms: optional_i64(object, "preparation_cutoff_ms"),
        deadline_ms: required_i64(object, family, "deadline_ms")?,
        target_count: count("target_count")?,
        prepared_count: count("prepared_count")?,
        effect_reserved_count: count("effect_reserved_count")?,
        terminal_count: count("terminal_count")?,
        completed_count: count("completed_count")?,
        unresolved_count: count("unresolved_count")?,
        recovery_snapshot_count: count("recovery_snapshot_count")?,
        recovery_snapshot_id: optional_string(object, "recovery_snapshot_id"),
        process_instance_id: optional_string(object, "process_instance_id").unwrap_or_default(),
        outcome,
        failure: object
            .get("failure")
            .filter(|value| !value.is_null())
            .map(|value| decode_failure(value, family))
            .transpose()?,
        shutdown_effect_count: count("shutdown_effect_count")?,
        admission_open: object.get("admission_open").and_then(Value::as_bool),
        retry_quit_same_boot: object.get("retry_quit_same_boot").and_then(Value::as_bool),
    })
}

fn known_shutdown_plan_fields(_: &str) -> &'static [&'static str] {
    &[
        "schema",
        "operation_id",
        "intent",
        "exit_code",
        "t0_ms",
        "preparation_cutoff_ms",
        "deadline_ms",
        "target_count",
        "prepared_count",
        "effect_reserved_count",
        "terminal_count",
        "completed_count",
        "unresolved_count",
        "recovery_snapshot_count",
        "recovery_snapshot_id",
        "process_instance_id",
        "outcome",
        "failure",
        "shutdown_effect_count",
        "admission_open",
        "retry_quit_same_boot",
    ]
}

fn decode_shutdown_target(
    object: &Map<String, Value>,
) -> Result<ShutdownTargetRecord, StoredRecordCodecError> {
    let family = StoredRecordFamily::ShutdownTarget;
    match required_text(object, family, "schema")? {
        "shutdown_target_v1" => {
            // Target identity is an opaque executor-owned key. Its enclosing
            // stored record is bounded, but the public page boundary is what
            // decides whether a large target may be returned.
            let target_id = required_text(object, family, "target_id")?.to_string();
            let effect_identity =
                optional_string(object, "effect_identity").unwrap_or_else(|| target_id.clone());
            Ok(ShutdownTargetRecord::Target {
                target_id,
                kind: match required_text(object, family, "kind")? {
                    "agent_session" => ShutdownTargetKindRecord::AgentSession,
                    "workflow_execution" => ShutdownTargetKindRecord::WorkflowExecution,
                    "workflow_node" => ShutdownTargetKindRecord::WorkflowNode,
                    other => {
                        return Err(StoredRecordCodecError::Incompatible {
                            family,
                            schema: format!("shutdown_target_kind.{other}"),
                        })
                    }
                },
                state: decode_shutdown_target_state(
                    required_text(object, family, "state")?,
                    family,
                )?,
                effect_identity,
                owner_operation_id: optional_string(object, "owner_operation_id"),
                failure: object
                    .get("failure")
                    .filter(|value| !value.is_null())
                    .map(|value| decode_failure(value, family))
                    .transpose()?,
                recovery_action: object
                    .get("recovery_action")
                    .filter(|value| !value.is_null())
                    .map(|value| {
                        let value = value
                            .as_object()
                            .ok_or(StoredRecordCodecError::Malformed { family })?;
                        Ok(ShutdownTargetRecoveryRecord {
                            action_id: string_field(value, family, "action_id")?,
                            origin_revision: u64_field(value, family, "origin_revision")?,
                            action: decode_recovery_action_kind(
                                required_text(value, family, "action")?,
                                family,
                            )?,
                            state: decode_obligation_state(value, family)?,
                        })
                    })
                    .transpose()?,
            })
        }
        "shutdown_recovery_snapshot_v1" => {
            // This is an embedded StoredObligationV1 document, not an
            // identity/reference. The enclosing record bound already limits
            // it, while the 512-byte reference bound would reject valid
            // recovery snapshots.
            let raw = required_text(object, family, "record")?.to_string();
            let record = StoredObligationV1::decode(&raw)?.into_value();
            Ok(ShutdownTargetRecord::RecoverySnapshot {
                obligation_id: string_field(object, family, "obligation_id")?,
                ordered_key: optional_string(object, "ordered_key").unwrap_or_default(),
                owner: string_field(object, family, "owner")?,
                revision: optional_u64(object, "revision").unwrap_or(0),
                record: Box::new(record),
            })
        }
        schema => Err(StoredRecordCodecError::Incompatible {
            family,
            schema: schema.to_string(),
        }),
    }
}

fn encode_shutdown_target(value: &ShutdownTargetRecord) -> Result<Value, StoredRecordCodecError> {
    match value {
        ShutdownTargetRecord::Target {
            target_id,
            kind,
            state,
            effect_identity,
            owner_operation_id,
            failure,
            recovery_action,
        } => {
            let mut object = Map::new();
            object.insert("schema".into(), Value::String("shutdown_target_v1".into()));
            object.insert("target_id".into(), Value::String(target_id.clone()));
            object.insert(
                "kind".into(),
                Value::String(
                    match kind {
                        ShutdownTargetKindRecord::AgentSession => "agent_session",
                        ShutdownTargetKindRecord::WorkflowExecution => "workflow_execution",
                        ShutdownTargetKindRecord::WorkflowNode => "workflow_node",
                    }
                    .into(),
                ),
            );
            object.insert(
                "state".into(),
                Value::String(shutdown_target_state_label(*state).into()),
            );
            object.insert(
                "effect_identity".into(),
                Value::String(effect_identity.clone()),
            );
            if let Some(owner) = owner_operation_id {
                object.insert("owner_operation_id".into(), Value::String(owner.clone()));
            }
            if let Some(failure) = failure {
                object.insert("failure".into(), encode_failure(failure));
            }
            if let Some(action) = recovery_action {
                object.insert(
                    "recovery_action".into(),
                    serde_json::json!({
                        "action_id":action.action_id,
                        "origin_revision":action.origin_revision,
                        "action":recovery_action_label(action.action),
                        "state":obligation_state_label(action.state),
                    }),
                );
            }
            Ok(Value::Object(object))
        }
        ShutdownTargetRecord::RecoverySnapshot {
            obligation_id,
            ordered_key,
            owner,
            revision,
            record,
        } => Ok(serde_json::json!({
            "schema":"shutdown_recovery_snapshot_v1",
            "obligation_id":obligation_id,
            "ordered_key":ordered_key,
            "owner":owner,
            "revision":revision,
            "record":StoredObligationV1::encode_new(record)?,
        })),
    }
}

fn known_shutdown_target_fields(_: &str) -> &'static [&'static str] {
    &[
        "schema",
        "target_id",
        "kind",
        "state",
        "effect_identity",
        "owner_operation_id",
        "failure",
        "recovery_action",
        "obligation_id",
        "ordered_key",
        "owner",
        "revision",
        "record",
    ]
}

fn validate_typed_metadata(
    family: StoredRecordFamily,
    schema: &str,
    object: &Map<String, Value>,
) -> Result<(), StoredRecordCodecError> {
    let nested_tag = |key: &str| {
        object
            .get(key)
            .and_then(Value::as_object)
            .and_then(|value| value.get("type"))
            .and_then(Value::as_str)
    };
    let state_raw = object
        .get("state")
        .and_then(Value::as_str)
        .or_else(|| nested_tag("state"))
        .or_else(|| nested_tag("status"));
    state_raw
        .map(|value| parse_closed_tag(family, schema, value))
        .transpose()?;
    let kind_raw = object
        .get("kind")
        .and_then(Value::as_str)
        .or_else(|| object.get("intent").and_then(Value::as_str));
    kind_raw
        .map(|value| parse_closed_tag(family, schema, value))
        .transpose()?;
    for key in [
        "resource_revision",
        "origin_revision",
        "revision",
        "next_source_ordinal",
        "source_count",
    ] {
        if object
            .get(key)
            .and_then(Value::as_u64)
            .is_some_and(|value| value > i64::MAX as u64)
        {
            return Err(StoredRecordCodecError::Integrity { family });
        }
    }
    for value in [
        "canonical_result_sha256",
        "source_summary_sha256",
        "inventory_sha256",
    ]
    .into_iter()
    .filter_map(|key| object.get(key).and_then(Value::as_str))
    {
        decode_hash32(family, value)?;
    }
    Ok(())
}

fn parse_closed_tag(
    family: StoredRecordFamily,
    schema: &str,
    value: &str,
) -> Result<StoredClosedTagV1, StoredRecordCodecError> {
    let tag = match value {
        "accepted" => StoredClosedTagV1::Accepted,
        "prepared" => StoredClosedTagV1::Prepared,
        "pending" => StoredClosedTagV1::Pending,
        "effect_reserved" => StoredClosedTagV1::EffectReserved,
        "running" => StoredClosedTagV1::Running,
        "waiting_approval" => StoredClosedTagV1::WaitingApproval,
        "awaiting_provider_start" => StoredClosedTagV1::AwaitingProviderStart,
        "awaiting_provider_response" => StoredClosedTagV1::AwaitingProviderResponse,
        "queued" => StoredClosedTagV1::Queued,
        "provider_start_reserved" => StoredClosedTagV1::ProviderStartReserved,
        "reconciliation_required" => StoredClosedTagV1::ReconciliationRequired,
        "outcome_unknown" => StoredClosedTagV1::OutcomeUnknown,
        "failed" => StoredClosedTagV1::Failed,
        "failed_before_activation" => StoredClosedTagV1::FailedBeforeActivation,
        "completed" => StoredClosedTagV1::Completed,
        "interrupted" => StoredClosedTagV1::Interrupted,
        "terminal" => StoredClosedTagV1::Terminal,
        "cancelled" => StoredClosedTagV1::Cancelled,
        "superseded" => StoredClosedTagV1::Superseded,
        "exit_pending" => StoredClosedTagV1::ExitPending,
        "exited" => StoredClosedTagV1::Exited,
        "preparing" => StoredClosedTagV1::Preparing,
        "activated" => StoredClosedTagV1::Activated,
        "provider_establish" => StoredClosedTagV1::ProviderEstablish,
        "turn_execution" => StoredClosedTagV1::TurnExecution,
        "queued_send" => StoredClosedTagV1::QueuedSend,
        "permission" => StoredClosedTagV1::Permission,
        "provider_session" => StoredClosedTagV1::ProviderSession,
        "backend_recovery" => StoredClosedTagV1::BackendRecovery,
        "recovery_publication" => StoredClosedTagV1::RecoveryPublication,
        "operation_binding" => StoredClosedTagV1::OperationBinding,
        "permission_response" => StoredClosedTagV1::PermissionResponse,
        "provider_interrupt" => StoredClosedTagV1::ProviderInterrupt,
        "session_close" => StoredClosedTagV1::SessionClose,
        "queue_pause" => StoredClosedTagV1::QueuePause,
        "agent_session" => StoredClosedTagV1::AgentSession,
        "workflow_execution" => StoredClosedTagV1::WorkflowExecution,
        "workflow_node" => StoredClosedTagV1::WorkflowNode,
        "started_turn" => StoredClosedTagV1::StartedTurn,
        "allow" => StoredClosedTagV1::Allow,
        "deny" => StoredClosedTagV1::Deny,
        "close" => StoredClosedTagV1::Close,
        "archive_open" => StoredClosedTagV1::ArchiveOpen,
        "archive_closed" => StoredClosedTagV1::ArchiveClosed,
        "switch_backend" => StoredClosedTagV1::SwitchBackend,
        "restart" => StoredClosedTagV1::Restart,
        "exit" => StoredClosedTagV1::Exit,
        _ => {
            return Err(StoredRecordCodecError::Incompatible {
                family,
                schema: format!("{schema}.{value}"),
            })
        }
    };
    Ok(tag)
}

fn decode_hash32(
    family: StoredRecordFamily,
    value: &str,
) -> Result<[u8; 32], StoredRecordCodecError> {
    let decoded = hex::decode(value).map_err(|_| StoredRecordCodecError::Integrity { family })?;
    decoded
        .try_into()
        .map_err(|_| StoredRecordCodecError::Integrity { family })
}

fn allowed_schemas(family: StoredRecordFamily) -> &'static [&'static str] {
    match family {
        StoredRecordFamily::OperationReceipt => &[
            "send_receipt_v1",
            "permission_response_receipt_v1",
            "stop_receipt_v1",
            "slc_receipt_v1",
            "application_quit_receipt_v1",
        ],
        StoredRecordFamily::OperationStatus => &[
            "send_status_v1",
            "permission_response_status_v1",
            "stop_status_v1",
            "slc_status_v1",
            "application_quit_status_v1",
        ],
        StoredRecordFamily::Terminal => &[
            "agent_turn_terminal_v1",
            "session_closed_terminal_v1",
            "stop_terminal_v1",
            "stop_superseded_v1",
        ],
        StoredRecordFamily::Obligation => &[
            "send_obligation_v1",
            "provider_establish_obligation_v1",
            "turn_execution_obligation_v1",
            "permission_response_obligation_v1",
            "stop_interrupt_obligation_v1",
            "session_close_obligation_v1",
            "backend_session_recovery_obligation_v1",
            "workflow_shutdown_effect_v1",
            "workflow_turn_completion_obligation_v1",
            "recovery_publication_obligation_v1",
            "recovery_reserved_obligation_v1",
            "recovery_completed_obligation_v1",
            "terminal_commit_obligation_v1",
            "session_feedback_reservation_v1",
            "session_feedback_v1",
            "workflow_execution_projection_v1",
        ],
        StoredRecordFamily::RecoveryAction => {
            &["recovery_action_attempt_v1", "feedback_retry_attempt_v1"]
        }
        StoredRecordFamily::RecoveryResult => {
            &["recovery_action_result_v1", "feedback_retry_result_v1"]
        }
        StoredRecordFamily::ShutdownPlan => &["shutdown_plan_summary_v1"],
        StoredRecordFamily::ShutdownTarget => {
            &["shutdown_target_v1", "shutdown_recovery_snapshot_v1"]
        }
    }
}

fn required_text<'a>(
    object: &'a Map<String, Value>,
    family: StoredRecordFamily,
    field: &'static str,
) -> Result<&'a str, StoredRecordCodecError> {
    object
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or(StoredRecordCodecError::MissingReference { family, field })
}

fn bounded_reference(
    value: &str,
    family: StoredRecordFamily,
    field: &'static str,
) -> Result<String, StoredRecordCodecError> {
    if value.is_empty() || value.len() > 512 || value.bytes().any(|byte| byte.is_ascii_control()) {
        return Err(StoredRecordCodecError::MissingReference { family, field });
    }
    Ok(value.to_string())
}

fn required_object<'a>(
    object: &'a Map<String, Value>,
    family: StoredRecordFamily,
    field: &'static str,
) -> Result<&'a Map<String, Value>, StoredRecordCodecError> {
    object
        .get(field)
        .and_then(Value::as_object)
        .ok_or(StoredRecordCodecError::MissingReference { family, field })
}

fn required_i64(
    object: &Map<String, Value>,
    family: StoredRecordFamily,
    field: &'static str,
) -> Result<i64, StoredRecordCodecError> {
    object
        .get(field)
        .and_then(Value::as_i64)
        .ok_or(StoredRecordCodecError::MissingReference { family, field })
}

fn require_text_fields(
    object: &Map<String, Value>,
    family: StoredRecordFamily,
    fields: &[&'static str],
) -> Result<(), StoredRecordCodecError> {
    for field in fields {
        required_text(object, family, field)?;
    }
    Ok(())
}

fn validate_required_shape(
    family: StoredRecordFamily,
    schema: &str,
    object: &Map<String, Value>,
) -> Result<(), StoredRecordCodecError> {
    match schema {
        "send_receipt_v1" => {
            require_text_fields(
                object,
                family,
                &[
                    "operation_id",
                    "session_id",
                    "input_ref",
                    "principal_mac",
                    "binding_hmac",
                ],
            )?;
            let disposition = required_object(object, family, "disposition")?;
            match required_text(disposition, family, "type")? {
                "started_turn" => {
                    required_text(disposition, family, "turn_id")?;
                }
                "queued" => {
                    required_text(disposition, family, "queue_item_id")?;
                }
                _ => {
                    return Err(StoredRecordCodecError::Incompatible {
                        family,
                        schema: "send_receipt_v1.disposition".to_string(),
                    })
                }
            }
        }
        "permission_response_receipt_v1" => require_text_fields(
            object,
            family,
            &[
                "operation_id",
                "session_id",
                "request_id",
                "input_ref",
                "principal_mac",
                "binding_hmac",
            ],
        )?,
        "stop_receipt_v1" => require_text_fields(
            object,
            family,
            &[
                "operation_id",
                "session_id",
                "turn_id",
                "principal_mac",
                "binding_hmac",
            ],
        )?,
        "slc_receipt_v1" => require_text_fields(
            object,
            family,
            &[
                "operation_id",
                "session_id",
                "principal_mac",
                "binding_hmac",
            ],
        )?,
        "application_quit_receipt_v1" => {
            require_text_fields(
                object,
                family,
                &["operation_id", "shutdown_id", "intent", "binding_hmac"],
            )?;
            required_i64(object, family, "deadline_ms")?;
        }
        "send_status_v1" | "permission_response_status_v1" => {
            required_object(object, family, "status")?;
        }
        "stop_status_v1" | "slc_status_v1" | "application_quit_status_v1" => {
            required_object(object, family, "state")?;
        }
        "agent_turn_terminal_v1"
        | "session_closed_terminal_v1"
        | "stop_terminal_v1"
        | "stop_superseded_v1" => {
            if object.get("session_id").and_then(Value::as_str).is_none()
                && object.get("operation_id").and_then(Value::as_str).is_none()
                && object
                    .get("terminal_identity")
                    .and_then(Value::as_str)
                    .is_none()
            {
                return Err(StoredRecordCodecError::MissingReference {
                    family,
                    field: "session_id|operation_id",
                });
            }
        }
        "send_obligation_v1" => require_text_fields(
            object,
            family,
            &["operation_id", "session_id", "kind", "state"],
        )?,
        "permission_response_obligation_v1" => require_text_fields(
            object,
            family,
            &["operation_id", "session_id", "request_id", "state"],
        )?,
        "stop_interrupt_obligation_v1" => require_text_fields(
            object,
            family,
            &["operation_id", "session_id", "turn_id", "state"],
        )?,
        "session_close_obligation_v1" => {
            require_text_fields(object, family, &["operation_id", "session_id", "state"])?;
        }
        "backend_session_recovery_obligation_v1" => {
            require_text_fields(object, family, &["session_id", "recovery_id", "state"])?;
        }
        "workflow_shutdown_effect_v1" => {
            require_text_fields(
                object,
                family,
                &["execution_id", "effect_identity", "state"],
            )?;
        }
        "recovery_publication_obligation_v1"
        | "provider_establish_obligation_v1"
        | "turn_execution_obligation_v1"
        | "workflow_turn_completion_obligation_v1"
        | "recovery_reserved_obligation_v1"
        | "recovery_completed_obligation_v1"
        | "terminal_commit_obligation_v1" => {
            required_text(object, family, "state")?;
        }
        "recovery_action_attempt_v1" if object.contains_key("resource_ref") => {
            require_text_fields(
                object,
                family,
                &[
                    "resource_ref",
                    "shutdown_id",
                    "target_key",
                    "action",
                    "effect_identity_sha256",
                    "intent",
                    "state",
                ],
            )?;
            required_i64(object, family, "ordinal")?;
            required_i64(object, family, "origin_revision")?;
            required_i64(object, family, "exit_code")?;
            hash_field(object, family, "effect_identity_sha256")?;
        }
        "recovery_action_attempt_v1" => {
            require_text_fields(
                object,
                family,
                &["obligation_id", "action", "effect_identity", "state"],
            )?;
            required_i64(object, family, "origin_revision")?;
        }
        "feedback_retry_attempt_v1" => require_text_fields(
            object,
            family,
            &["feedback_id", "resolution_identity", "state"],
        )?,
        "recovery_action_result_v1" => validate_recovery_result(family, object)?,
        "feedback_retry_result_v1" => {
            require_text_fields(object, family, &["feedback_id", "outcome"])?;
            required_i64(object, family, "resource_revision")?;
        }
        "session_feedback_reservation_v1" => require_text_fields(
            object,
            family,
            &[
                "feedback_id",
                "attempt_id",
                "session_id",
                "operation",
                "process_instance_id",
            ],
        )?,
        "session_feedback_v1" => {
            require_text_fields(
                object,
                family,
                &["feedback_id", "attempt_id", "session_id", "operation"],
            )?;
            required_object(object, family, "failure")?;
            if object.get("actions").and_then(Value::as_array).is_none() {
                return Err(StoredRecordCodecError::MissingReference {
                    family,
                    field: "actions",
                });
            }
        }
        "workflow_execution_projection_v1" => {
            if object.get("deleted").and_then(Value::as_bool) != Some(false) {
                return Err(StoredRecordCodecError::Incompatible {
                    family,
                    schema: "workflow_execution_projection_v1.deleted".to_string(),
                });
            }
            required_object(object, family, "execution")?;
        }
        "shutdown_plan_summary_v1" => {
            required_text(object, family, "operation_id")?;
            required_text(object, family, "intent")?;
            required_i64(object, family, "deadline_ms")?;
        }
        "shutdown_target_v1" => {
            require_text_fields(object, family, &["target_id", "kind", "state"])?;
        }
        "shutdown_recovery_snapshot_v1" => {
            require_text_fields(object, family, &["obligation_id", "owner", "record"])?;
        }
        _ => {}
    }
    Ok(())
}

fn validate_recovery_result(
    family: StoredRecordFamily,
    object: &Map<String, Value>,
) -> Result<(), StoredRecordCodecError> {
    let outcome = required_text(object, family, "outcome")?;
    let classification = required_text(object, family, "classification")?;
    let resource_revision = object
        .get("resource_revision")
        .and_then(Value::as_u64)
        .filter(|revision| *revision <= i64::MAX as u64)
        .ok_or(StoredRecordCodecError::MissingReference {
            family,
            field: "resource_revision",
        })?;
    let resource_view = required_text(object, family, "resource_view")?;
    let actual = required_text(object, family, "canonical_result_sha256")?;
    if actual.len() != 64 || !actual.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(StoredRecordCodecError::Integrity { family });
    }
    let canonical = serde_json::json!({
        "schema": "recovery_action_canonical_result_v1",
        "outcome": outcome,
        "classification": classification,
        "resource_revision": resource_revision,
        "resource_view": resource_view,
    });
    let expected = hex::encode(Sha256::digest(
        serde_json::to_vec(&canonical).map_err(|_| StoredRecordCodecError::Malformed { family })?,
    ));
    if actual != expected {
        return Err(StoredRecordCodecError::Integrity { family });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_required_stored_v1_family_round_trips_and_preserves_additive_raw() {
        let fixtures = [
            (
                StoredRecordFamily::OperationReceipt,
                r#"{"schema":"send_receipt_v1","operation_id":"op-1","session_id":"s-1","input_ref":"input-1","disposition":{"type":"started_turn","turn_id":"1"},"principal_mac":"0000000000000000000000000000000000000000000000000000000000000000","binding_hmac":"0000000000000000000000000000000000000000000000000000000000000000"}"#,
            ),
            (
                StoredRecordFamily::OperationStatus,
                r#"{"schema":"send_status_v1","status":{"type":"running","turn_id":"1"}}"#,
            ),
            (
                StoredRecordFamily::Terminal,
                r#"{"schema":"agent_turn_terminal_v1","session_id":"s-1","turn_id":"1","terminal_kind":"completed","message_id":"m-1","streaming_final_seq":"0","completed_at_bits":"0","turn_result":{"type":"completed"}}"#,
            ),
            (
                StoredRecordFamily::Obligation,
                r#"{"schema":"send_obligation_v1","operation_id":"op-1","session_id":"s-1","kind":"provider_establish","state":"prepared"}"#,
            ),
            (
                StoredRecordFamily::RecoveryAction,
                r#"{"schema":"recovery_action_attempt_v1","obligation_id":"ob-1","origin_revision":0,"action":"read_again","effect_identity":"effect-1","state":"prepared"}"#,
            ),
            (
                StoredRecordFamily::RecoveryResult,
                r#"{"schema":"recovery_action_result_v1","outcome":"unchanged","classification":"unchanged","resource_revision":0,"canonical_result_sha256":"19a17b3ff7a0c995e744c55b111f4f6c85a1cf8ba2a07ad88681b9e241cdd8b1","resource_view":"{}"}"#,
            ),
            (
                StoredRecordFamily::ShutdownPlan,
                r#"{"schema":"shutdown_plan_summary_v1","operation_id":"quit-1","intent":"exit","deadline_ms":15001}"#,
            ),
            (
                StoredRecordFamily::ShutdownTarget,
                r#"{"schema":"shutdown_target_v1","target_id":"s-1","kind":"agent_session","state":"prepared"}"#,
            ),
        ];

        for (family, raw) in fixtures {
            let with_additive = format!(
                "{},\"future_additive\": {{ \"exponent\": 1e+09, \"order\": [3, 2, 1] }} }}",
                raw.strip_suffix('}').expect("object fixture")
            );
            let decoded = StoredStateRecordV1::decode(family, &with_additive)
                .expect("known v1 fixture with an additive member");
            assert_eq!(decoded.encode(), with_additive, "{family:?}");
        }
    }

    #[test]
    fn unknown_additive_fields_are_preserved_byte_for_byte() {
        let raw = r#"{"schema":"shutdown_target_v1","target_id":"s-1","kind":"agent_session","state":"prepared","future":{"z":2,"a":1}}"#;
        let decoded = StoredStateRecordV1::decode(StoredRecordFamily::ShutdownTarget, raw)
            .expect("additive field");
        assert_eq!(decoded.encode(), raw);
    }

    #[test]
    fn cas_update_preserves_nested_additive_raw_without_stale_known_fields() {
        let raw = r#"{"schema":"send_status_v1","status":{"type":"running","turn_id":"turn-1","future_nested": { "exponent": 1e+09, "order": [3, 2, 1] }},"future_top": { "spacing": true }}"#;
        let stored = StoredOperationStatusV1::decode(raw).expect("status with additive members");
        assert_eq!(
            stored.raw(),
            raw,
            "query/export keeps the exact stored bytes"
        );
        let mut updated = stored.value().clone();
        updated.value = OperationStatusValue::Completed;
        let encoded = stored
            .encode_update(&updated)
            .expect("CAS update with additive members");

        assert!(encoded.contains(r#""future_nested":{ "exponent": 1e+09, "order": [3, 2, 1] }"#));
        assert!(encoded.contains(r#""future_top":{ "spacing": true }"#));
        assert!(
            !encoded.contains("turn_id"),
            "known old-variant fields are not carried forward"
        );
        assert!(encoded.contains(r#""status":{"type":"completed""#));
    }

    #[test]
    fn obligation_cas_update_preserves_additive_members_inside_recovery_action() {
        let raw = r#"{"schema":"send_obligation_v1","obligation_id":"ob-1","operation_id":"op-1","session_id":"s-1","kind":"turn_execution","disposition":"started_turn","state":"reconciliation_required","recovery_action":{"action_id":"action-1","origin_revision":0,"action":"read_again","effect_identity":"effect-1","state":"prepared","future_proof": { "raw": 9.00e-3 }}}"#;
        let stored =
            StoredObligationV1::decode(raw).expect("obligation with additive action proof");
        let encoded = stored
            .encode_update(stored.value())
            .expect("obligation CAS update");
        assert!(encoded.contains(r#""future_proof":{ "raw": 9.00e-3 }"#));
    }

    #[test]
    fn embedded_records_are_not_limited_to_the_short_reference_bound() {
        let obligation = serde_json::json!({
            "schema": "send_obligation_v1",
            "obligation_id": "ob-1",
            "operation_id": "op-1",
            "session_id": "s-1",
            "kind": "turn_execution",
            "disposition": "started_turn",
            "canonical_payload": "x".repeat(1_024),
            "state": "prepared",
        })
        .to_string();
        let snapshot = serde_json::json!({
            "schema": "shutdown_recovery_snapshot_v1",
            "obligation_id": "ob-1",
            "ordered_key": "0001-ob-1",
            "owner": "s-1",
            "revision": 0,
            "record": obligation,
        })
        .to_string();
        StoredShutdownTargetV1::decode(&snapshot)
            .expect("embedded recovery obligation may exceed an identity bound");
    }

    #[test]
    fn unknown_required_version_and_variant_fail_closed() {
        for raw in [
            r#"{"schema":"send_receipt_v2","operation_id":"op-1"}"#,
            r#"{"schema":"send_receipt_v1","operation_id":"op-1","session_id":"s-1","input_ref":"input-1","disposition":{"type":"future_required"},"principal_mac":"0000000000000000000000000000000000000000000000000000000000000000","binding_hmac":"0000000000000000000000000000000000000000000000000000000000000000"}"#,
        ] {
            assert!(matches!(
                StoredStateRecordV1::decode(StoredRecordFamily::OperationReceipt, raw),
                Err(StoredRecordCodecError::Incompatible { .. })
            ));
        }
    }

    #[test]
    fn f06_unknown_required_failure_kind_is_incompatible_not_internal() {
        let raw = r#"{
            "schema":"send_status_v1",
            "status":{
                "type":"failed",
                "failure":{
                    "kind":"future_required_failure",
                    "retryable":false,
                    "label":"future failure",
                    "correlation_id":"f06-future-failure"
                }
            }
        }"#;

        assert!(matches!(
            StoredOperationStatusV1::decode(raw),
            Err(StoredRecordCodecError::Incompatible {
                family: StoredRecordFamily::OperationStatus,
                ..
            })
        ));
    }

    #[test]
    fn malformed_hash_and_required_reference_fail_closed() {
        for (family, raw) in [
            (
                StoredRecordFamily::RecoveryResult,
                r#"{"schema":"recovery_action_result_v1","outcome":"unchanged","classification":"unchanged","resource_revision":0,"canonical_result_sha256":"ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff","resource_view":"{}"}"#,
            ),
            (
                StoredRecordFamily::Obligation,
                r#"{"schema":"send_obligation_v1","operation_id":"op-1","state":"prepared"}"#,
            ),
            (StoredRecordFamily::ShutdownTarget, "not-json"),
        ] {
            assert!(StoredStateRecordV1::decode(family, raw).is_err());
        }
    }

    #[test]
    fn arbitrary_nonversioned_string_is_not_a_normal_record() {
        assert!(matches!(
            StoredStateRecordV1::decode(StoredRecordFamily::Obligation, "arbitrary"),
            Err(StoredRecordCodecError::Malformed { .. })
        ));
    }
}
