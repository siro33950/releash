use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::domain::local_event::record::*;
use crate::domain::local_event::{
    OperationKind, QuitIntent, RecoveryActionKind, RecoveryResultClassification,
    SessionOperationFailureKind, ShutdownPlanKey,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StoredRecordFamily {
    OperationReceipt,
    OperationStatus,
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

macro_rules! stored_record {
    ($name:ident, $value:ty, $family:expr, $decode:ident, $encode:ident) => {
        #[derive(Debug, Clone, PartialEq)]
        pub(crate) struct $name {
            value: $value,
            raw: String,
        }

        impl $name {
            pub(crate) fn decode(raw: &str) -> Result<Self, StoredRecordCodecError> {
                let object = validated_object($family, raw)?;
                let value = $decode(&object)?;
                Ok(Self {
                    value,
                    raw: raw.to_string(),
                })
            }

            pub(crate) fn into_value(self) -> $value {
                self.value
            }

            pub(crate) fn encode_new(value: &$value) -> Result<String, StoredRecordCodecError> {
                encode_and_validate($family, $encode(value)?)
            }
        }
    };
}

macro_rules! stored_record_value {
    ($name:ident, $value:ty) => {
        impl $name {
            pub(crate) fn value(&self) -> &$value {
                &self.value
            }
        }
    };
}

macro_rules! stored_record_update {
    ($name:ident, $value:ty, $family:expr, $encode:ident) => {
        impl $name {
            pub(crate) fn encode_update(
                &self,
                value: &$value,
            ) -> Result<String, StoredRecordCodecError> {
                let mut encoded = $encode(value)?;
                let old: Value = serde_json::from_str(&self.raw)
                    .map_err(|_| StoredRecordCodecError::Malformed { family: $family })?;
                if let (Some(encoded), Some(old)) = (encoded.as_object_mut(), old.as_object()) {
                    for (key, value) in old {
                        if !encoded.contains_key(key) {
                            encoded.insert(key.clone(), value.clone());
                        }
                    }
                }
                encode_and_validate($family, encoded)
            }
        }
    };
}

stored_record!(
    StoredOperationReceiptV1,
    OperationReceiptRecord,
    StoredRecordFamily::OperationReceipt,
    decode_operation_receipt,
    encode_operation_receipt
);
stored_record!(
    StoredOperationStatusV1,
    OperationStatusRecord,
    StoredRecordFamily::OperationStatus,
    decode_operation_status,
    encode_operation_status
);
stored_record!(
    StoredObligationV1,
    ObligationRecord,
    StoredRecordFamily::Obligation,
    decode_obligation,
    encode_obligation
);
stored_record!(
    StoredRecoveryActionV1,
    RecoveryAttemptRecord,
    StoredRecordFamily::RecoveryAction,
    decode_recovery_attempt,
    encode_recovery_attempt
);
stored_record!(
    StoredRecoveryResultV1,
    RecoveryResultRecord,
    StoredRecordFamily::RecoveryResult,
    decode_recovery_result,
    encode_recovery_result
);
stored_record!(
    StoredShutdownPlanV1,
    ShutdownPlanRecord,
    StoredRecordFamily::ShutdownPlan,
    decode_shutdown_plan,
    encode_shutdown_plan
);
stored_record!(
    StoredShutdownTargetV1,
    ShutdownTargetRecord,
    StoredRecordFamily::ShutdownTarget,
    decode_shutdown_target,
    encode_shutdown_target
);

stored_record_value!(StoredOperationReceiptV1, OperationReceiptRecord);
stored_record_value!(StoredRecoveryResultV1, RecoveryResultRecord);
stored_record_value!(StoredShutdownTargetV1, ShutdownTargetRecord);

stored_record_update!(
    StoredOperationStatusV1,
    OperationStatusRecord,
    StoredRecordFamily::OperationStatus,
    encode_operation_status
);
stored_record_update!(
    StoredObligationV1,
    ObligationRecord,
    StoredRecordFamily::Obligation,
    encode_obligation
);
stored_record_update!(
    StoredRecoveryActionV1,
    RecoveryAttemptRecord,
    StoredRecordFamily::RecoveryAction,
    encode_recovery_attempt
);
stored_record_update!(
    StoredShutdownPlanV1,
    ShutdownPlanRecord,
    StoredRecordFamily::ShutdownPlan,
    encode_shutdown_plan
);
stored_record_update!(
    StoredShutdownTargetV1,
    ShutdownTargetRecord,
    StoredRecordFamily::ShutdownTarget,
    encode_shutdown_target
);

fn validated_object(
    family: StoredRecordFamily,
    raw: &str,
) -> Result<Map<String, Value>, StoredRecordCodecError> {
    let object = serde_json::from_str::<Value>(raw)
        .map_err(|_| StoredRecordCodecError::Malformed { family })?
        .as_object()
        .cloned()
        .ok_or(StoredRecordCodecError::Malformed { family })?;
    let schema = required_text(&object, family, "schema")?;
    if !allowed_schemas(family).contains(&schema) {
        return Err(StoredRecordCodecError::Incompatible {
            family,
            schema: schema.to_string(),
        });
    }
    Ok(object)
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

fn allowed_schemas(family: StoredRecordFamily) -> &'static [&'static str] {
    match family {
        StoredRecordFamily::OperationReceipt => &["application_quit_receipt_v1"],
        StoredRecordFamily::OperationStatus => &["application_quit_status_v1"],
        StoredRecordFamily::Obligation => &[
            "workflow_shutdown_effect_v1",
            "workflow_execution_projection_v1",
        ],
        StoredRecordFamily::RecoveryAction => &["recovery_action_attempt_v1"],
        StoredRecordFamily::RecoveryResult => &["recovery_action_result_v1"],
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

fn optional_string(object: &Map<String, Value>, field: &str) -> Option<String> {
    object
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
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

fn required_u64(
    object: &Map<String, Value>,
    family: StoredRecordFamily,
    field: &'static str,
) -> Result<u64, StoredRecordCodecError> {
    object
        .get(field)
        .and_then(Value::as_u64)
        .ok_or(StoredRecordCodecError::MissingReference { family, field })
}

fn optional_i64(object: &Map<String, Value>, field: &str) -> Option<i64> {
    object.get(field).and_then(Value::as_i64)
}

fn optional_u64(object: &Map<String, Value>, field: &str) -> Option<u64> {
    object.get(field).and_then(Value::as_u64)
}

fn hash_field(
    object: &Map<String, Value>,
    family: StoredRecordFamily,
    field: &'static str,
) -> Result<[u8; 32], StoredRecordCodecError> {
    let bytes = hex::decode(required_text(object, family, field)?)
        .map_err(|_| StoredRecordCodecError::Malformed { family })?;
    bytes
        .try_into()
        .map_err(|_| StoredRecordCodecError::Malformed { family })
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

fn encode_operation_receipt(
    value: &OperationReceiptRecord,
) -> Result<Value, StoredRecordCodecError> {
    let OperationReceiptRecord::ApplicationQuit {
        operation_id,
        plan,
        intent,
        t0_ms,
        deadline_ms,
        binding_hmac,
    } = value;
    let (intent, exit_code) = quit_fields(*intent);
    Ok(serde_json::json!({
        "schema":"application_quit_receipt_v1",
        "operation_id":operation_id,
        "shutdown_id":plan.shutdown_id,
        "intent":intent,
        "exit_code":exit_code,
        "t0_ms":t0_ms,
        "deadline_ms":deadline_ms,
        "binding_hmac":hex::encode(binding_hmac),
    }))
}

fn decode_operation_receipt(
    object: &Map<String, Value>,
) -> Result<OperationReceiptRecord, StoredRecordCodecError> {
    let family = StoredRecordFamily::OperationReceipt;
    Ok(OperationReceiptRecord::ApplicationQuit {
        operation_id: required_text(object, family, "operation_id")?.to_string(),
        plan: ShutdownPlanKey {
            shutdown_id: required_text(object, family, "shutdown_id")?.to_string(),
        },
        intent: decode_quit_intent(object, family)?,
        t0_ms: required_i64(object, family, "t0_ms")?,
        deadline_ms: required_i64(object, family, "deadline_ms")?,
        binding_hmac: hash_field(object, family, "binding_hmac")?,
    })
}

fn encode_operation_status(value: &OperationStatusRecord) -> Result<Value, StoredRecordCodecError> {
    if value.kind != OperationKind::ApplicationQuit {
        return Err(StoredRecordCodecError::Incompatible {
            family: StoredRecordFamily::OperationStatus,
            schema: "operation_kind".to_string(),
        });
    }
    let state = match &value.value {
        OperationStatusValue::Preparing => serde_json::json!({"type":"preparing"}),
        OperationStatusValue::Activated => serde_json::json!({"type":"activated"}),
        OperationStatusValue::Completed => serde_json::json!({"type":"completed"}),
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
            "type":"failed_before_activation",
            "failure":encode_failure(failure),
        }),
        OperationStatusValue::ReconciliationRequired { failure } => serde_json::json!({
            "type":"reconciliation_required",
            "failure":encode_failure(failure),
        }),
    };
    Ok(serde_json::json!({
        "schema":"application_quit_status_v1",
        "state":state,
    }))
}

fn decode_operation_status(
    object: &Map<String, Value>,
) -> Result<OperationStatusRecord, StoredRecordCodecError> {
    let family = StoredRecordFamily::OperationStatus;
    let state = object.get("state").and_then(Value::as_object).ok_or(
        StoredRecordCodecError::MissingReference {
            family,
            field: "state",
        },
    )?;
    let value = match required_text(state, family, "type")? {
        "preparing" => OperationStatusValue::Preparing,
        "activated" => OperationStatusValue::Activated,
        "completed" => OperationStatusValue::Completed,
        "outcome_unknown" => OperationStatusValue::OutcomeUnknown {
            operation_id: required_text(state, family, "operation_id")?.to_string(),
            plan: ShutdownPlanKey {
                shutdown_id: required_text(state, family, "shutdown_id")?.to_string(),
            },
            activation_commit_id: required_text(state, family, "activation_commit_id")?.to_string(),
        },
        "failed_before_activation" => OperationStatusValue::FailedBeforeActivation {
            failure: decode_failure(
                state
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
                state
                    .get("failure")
                    .ok_or(StoredRecordCodecError::MissingReference {
                        family,
                        field: "failure",
                    })?,
                family,
            )?,
        },
        other => {
            return Err(StoredRecordCodecError::Incompatible {
                family,
                schema: format!("application_quit_state.{other}"),
            })
        }
    };
    Ok(OperationStatusRecord {
        kind: OperationKind::ApplicationQuit,
        value,
    })
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
    use SessionOperationFailureKind as K;
    Some(match raw {
        "storage_unavailable" => K::StorageUnavailable,
        "storage_corrupt" => K::StorageCorrupt,
        "persist_failure" => K::PersistFailure,
        "protocol_incompatible" => K::ProtocolIncompatible,
        "provider_unavailable" => K::ProviderUnavailable,
        "external_effect_failed" => K::ExternalEffectFailed,
        "outcome_unknown" => K::OutcomeUnknown,
        "deadline_exceeded" => K::DeadlineExceeded,
        "capacity_exceeded" => K::CapacityExceeded,
        "stop_capacity_exceeded" => K::StopCapacityExceeded,
        "shutdown_authority_mismatch" => K::ShutdownAuthorityMismatch,
        "target_revision_changed" => K::TargetRevisionChanged,
        "owner_revision_changed" => K::OwnerRevisionChanged,
        "runtime_generation_changed" => K::RuntimeGenerationChanged,
        "invalid_effect_intent" => K::InvalidEffectIntent,
        "previous_shutdown_reconciliation_required" => K::PreviousShutdownReconciliationRequired,
        "internal" => K::Internal,
        _ => return None,
    })
}

fn encode_failure(value: &crate::domain::local_event::SafeOperationFailure) -> Value {
    serde_json::json!({
        "kind":failure_kind_label(value.kind),
        "retryable":value.retryable,
        "label":value.label.value(),
        "detail":value.detail.as_ref().map(|value| value.value()),
        "correlation_id":value.correlation_id,
    })
}

fn decode_failure(
    value: &Value,
    family: StoredRecordFamily,
) -> Result<crate::domain::local_event::SafeOperationFailure, StoredRecordCodecError> {
    let object = value
        .as_object()
        .ok_or(StoredRecordCodecError::Malformed { family })?;
    let kind = parse_failure_kind(required_text(object, family, "kind")?).ok_or_else(|| {
        StoredRecordCodecError::Incompatible {
            family,
            schema: "failure_kind".to_string(),
        }
    })?;
    let retryable = object.get("retryable").and_then(Value::as_bool).ok_or(
        StoredRecordCodecError::MissingReference {
            family,
            field: "retryable",
        },
    )?;
    let mut failure = crate::domain::local_event::SafeOperationFailure::new(
        kind,
        retryable,
        required_text(object, family, "label")?,
        required_text(object, family, "correlation_id")?,
    );
    if let Some(detail) = object.get("detail").and_then(Value::as_str) {
        failure = failure.with_detail(detail);
    }
    Ok(failure)
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
    Ok(match required_text(object, family, "state")? {
        "prepared" => ObligationStateRecord::Prepared,
        "pending" => ObligationStateRecord::Pending,
        "effect_reserved" => ObligationStateRecord::EffectReserved,
        "running" => ObligationStateRecord::Running,
        "waiting_approval" => ObligationStateRecord::WaitingApproval,
        "outcome_unknown" => ObligationStateRecord::OutcomeUnknown,
        "reconciliation_required" => ObligationStateRecord::ReconciliationRequired,
        "failed" => ObligationStateRecord::Failed,
        "completed" => ObligationStateRecord::Completed,
        "cancelled" => ObligationStateRecord::Cancelled,
        other => {
            return Err(StoredRecordCodecError::Incompatible {
                family,
                schema: format!("obligation_state.{other}"),
            })
        }
    })
}

fn encode_obligation(value: &ObligationRecord) -> Result<Value, StoredRecordCodecError> {
    match value {
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
    }
}

fn decode_obligation(
    object: &Map<String, Value>,
) -> Result<ObligationRecord, StoredRecordCodecError> {
    let family = StoredRecordFamily::Obligation;
    match required_text(object, family, "schema")? {
        "workflow_shutdown_effect_v1" => Ok(ObligationRecord::WorkflowShutdown {
            operation_id: required_text(object, family, "operation_id")?.to_string(),
            effect_identity: required_text(object, family, "effect_identity")?.to_string(),
            owner_revision: required_i64(object, family, "owner_revision")?,
            execution_id: required_text(object, family, "execution_id")?.to_string(),
            state: decode_obligation_state(object, family)?,
        }),
        _ => unreachable!("schema was validated"),
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

fn decode_recovery_action(
    raw: &str,
    family: StoredRecordFamily,
) -> Result<RecoveryActionKind, StoredRecordCodecError> {
    Ok(match raw {
        "read_again" => RecoveryActionKind::ReadAgain,
        "retry_same_effect" => RecoveryActionKind::RetrySameEffect,
        "use_observed_result" => RecoveryActionKind::UseObservedResult,
        "cancel_if_safe" => RecoveryActionKind::CancelIfSafe,
        "keep_for_manual_resolution" => RecoveryActionKind::KeepForManualResolution,
        other => {
            return Err(StoredRecordCodecError::Incompatible {
                family,
                schema: format!("recovery_action.{other}"),
            })
        }
    })
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
    Ok(match raw {
        "pending" => RecoveryResultClassification::Pending,
        "succeeded" => RecoveryResultClassification::Succeeded,
        "confirmed_no_effect" => RecoveryResultClassification::ConfirmedNoEffect,
        "ambiguous" => RecoveryResultClassification::Ambiguous,
        "cancelled_before_effect" => RecoveryResultClassification::CancelledBeforeEffect,
        "unchanged" => RecoveryResultClassification::Unchanged,
        other => {
            return Err(StoredRecordCodecError::Incompatible {
                family,
                schema: format!("recovery_classification.{other}"),
            })
        }
    })
}

fn encode_recovery_attempt(value: &RecoveryAttemptRecord) -> Result<Value, StoredRecordCodecError> {
    let RecoveryAttemptRecord::ShutdownTarget {
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
    } = value;
    let (intent, exit_code) = quit_fields(*intent);
    let mut object = serde_json::json!({
        "schema":"recovery_action_attempt_v1",
        "resource_ref":resource_ref,
        "shutdown_id":plan.shutdown_id,
        "ordinal":ordinal,
        "target_key":target_key,
        "origin_revision":origin_revision,
        "action":recovery_action_label(*action),
        "effect_identity_sha256":hex::encode(effect_identity_sha256),
        "intent":intent,
        "exit_code":exit_code,
        "state":obligation_state_label(*state),
    });
    if let Some(failure) = failure {
        object
            .as_object_mut()
            .expect("object")
            .insert("failure".to_string(), encode_failure(failure));
    }
    Ok(object)
}

fn decode_recovery_attempt(
    object: &Map<String, Value>,
) -> Result<RecoveryAttemptRecord, StoredRecordCodecError> {
    let family = StoredRecordFamily::RecoveryAction;
    Ok(RecoveryAttemptRecord::ShutdownTarget {
        resource_ref: required_text(object, family, "resource_ref")?.to_string(),
        plan: ShutdownPlanKey {
            shutdown_id: required_text(object, family, "shutdown_id")?.to_string(),
        },
        ordinal: required_i64(object, family, "ordinal")?,
        target_key: required_text(object, family, "target_key")?.to_string(),
        origin_revision: required_u64(object, family, "origin_revision")?,
        action: decode_recovery_action(required_text(object, family, "action")?, family)?,
        effect_identity_sha256: hash_field(object, family, "effect_identity_sha256")?,
        intent: decode_quit_intent(object, family)?,
        state: decode_obligation_state(object, family)?,
        failure: object
            .get("failure")
            .map(|value| decode_failure(value, family))
            .transpose()?,
    })
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
    Ok(match raw {
        "pending" => RecoveryResultOutcomeRecord::Pending,
        "terminal" => RecoveryResultOutcomeRecord::Terminal,
        "unchanged" => RecoveryResultOutcomeRecord::Unchanged,
        other => {
            return Err(StoredRecordCodecError::Incompatible {
                family,
                schema: format!("recovery_outcome.{other}"),
            })
        }
    })
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
    Ok(match raw {
        "prepared" => ShutdownTargetStateRecord::Prepared,
        "effect_reserved" => ShutdownTargetStateRecord::EffectReserved,
        "completed" => ShutdownTargetStateRecord::Completed,
        "failed" => ShutdownTargetStateRecord::Failed,
        "reconciliation_required" => ShutdownTargetStateRecord::ReconciliationRequired,
        other => {
            return Err(StoredRecordCodecError::Incompatible {
                family,
                schema: format!("shutdown_target_state.{other}"),
            })
        }
    })
}

fn encode_recovery_resource_view(
    value: &RecoveryResourceViewRecord,
) -> Result<String, StoredRecordCodecError> {
    let family = StoredRecordFamily::RecoveryResult;
    let value = match value {
        RecoveryResourceViewRecord::SafeSummary(value) => return Ok(value.clone()),
        RecoveryResourceViewRecord::ShutdownTarget {
            plan,
            ordinal,
            target_id,
            state,
        } => serde_json::json!({
            "schema":"shutdown_target_recovery_result_v1",
            "shutdown_id":plan.shutdown_id,
            "ordinal":ordinal,
            "target_key":target_id,
            "state":shutdown_target_state_label(*state),
        }),
    };
    serde_json::to_string(&value).map_err(|_| StoredRecordCodecError::Malformed { family })
}

fn canonical_recovery_result_sha256(
    outcome: RecoveryResultOutcomeRecord,
    classification: RecoveryResultClassification,
    resource_revision: u64,
    resource_view: &str,
) -> Result<[u8; 32], StoredRecordCodecError> {
    serde_json::to_vec(&serde_json::json!({
        "schema":"recovery_action_canonical_result_v1",
        "outcome":recovery_outcome_label(outcome),
        "classification":recovery_classification_label(classification),
        "resource_revision":resource_revision,
        "resource_view":resource_view,
    }))
    .map(|bytes| Sha256::digest(bytes).into())
    .map_err(|_| StoredRecordCodecError::Malformed {
        family: StoredRecordFamily::RecoveryResult,
    })
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
            schema: "recovery_action_result_v1.outcome_classification".to_string(),
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
        resource_view: RecoveryResourceViewRecord::SafeSummary(resource_view),
    }))
}

fn encode_recovery_result(value: &RecoveryResultRecord) -> Result<Value, StoredRecordCodecError> {
    let RecoveryResultRecord::Action(value) = value;
    let family = StoredRecordFamily::RecoveryResult;
    if !recovery_result_pair_is_valid(value.outcome, value.classification) {
        return Err(StoredRecordCodecError::Incompatible {
            family,
            schema: "recovery_action_result_v1.outcome_classification".to_string(),
        });
    }
    let resource_view = encode_recovery_resource_view(&value.resource_view)?;
    if canonical_recovery_result_sha256(
        value.outcome,
        value.classification,
        value.resource_revision,
        &resource_view,
    )? != value.canonical_result_sha256
    {
        return Err(StoredRecordCodecError::Integrity { family });
    }
    Ok(serde_json::json!({
        "schema":"recovery_action_result_v1",
        "outcome":recovery_outcome_label(value.outcome),
        "classification":recovery_classification_label(value.classification),
        "resource_revision":value.resource_revision,
        "canonical_result_sha256":hex::encode(value.canonical_result_sha256),
        "resource_view":resource_view,
    }))
}

fn decode_recovery_result(
    object: &Map<String, Value>,
) -> Result<RecoveryResultRecord, StoredRecordCodecError> {
    let family = StoredRecordFamily::RecoveryResult;
    let outcome = decode_recovery_outcome(required_text(object, family, "outcome")?, family)?;
    let classification =
        decode_recovery_classification(required_text(object, family, "classification")?, family)?;
    if !recovery_result_pair_is_valid(outcome, classification) {
        return Err(StoredRecordCodecError::Incompatible {
            family,
            schema: "recovery_action_result_v1.outcome_classification".to_string(),
        });
    }
    let resource_revision = required_u64(object, family, "resource_revision")?;
    let resource_view = required_text(object, family, "resource_view")?.to_string();
    let canonical_result_sha256 = hash_field(object, family, "canonical_result_sha256")?;
    if canonical_recovery_result_sha256(outcome, classification, resource_revision, &resource_view)?
        != canonical_result_sha256
    {
        return Err(StoredRecordCodecError::Integrity { family });
    }
    Ok(RecoveryResultRecord::Action(RecoveryActionResultRecord {
        outcome,
        classification,
        resource_revision,
        canonical_result_sha256,
        resource_view: RecoveryResourceViewRecord::SafeSummary(resource_view),
    }))
}

fn encode_shutdown_plan(value: &ShutdownPlanRecord) -> Result<Value, StoredRecordCodecError> {
    let (intent, exit_code) = quit_fields(value.intent);
    let mut object = Map::new();
    object.insert(
        "schema".to_string(),
        Value::String("shutdown_plan_summary_v1".to_string()),
    );
    object.insert(
        "operation_id".to_string(),
        Value::String(value.operation_id.clone()),
    );
    object.insert("intent".to_string(), Value::String(intent.to_string()));
    object.insert("exit_code".to_string(), Value::from(exit_code));
    object.insert("t0_ms".to_string(), Value::from(value.t0_ms));
    object.insert("deadline_ms".to_string(), Value::from(value.deadline_ms));
    for (key, field) in [("preparation_cutoff_ms", value.preparation_cutoff_ms)] {
        if let Some(field) = field {
            object.insert(key.to_string(), Value::from(field));
        }
    }
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
            object.insert(key.to_string(), Value::from(field));
        }
    }
    for (key, field) in [("recovery_snapshot_id", &value.recovery_snapshot_id)] {
        if let Some(field) = field {
            object.insert(key.to_string(), Value::String(field.clone()));
        }
    }
    object.insert(
        "process_instance_id".to_string(),
        Value::String(value.process_instance_id.clone()),
    );
    if let Some(outcome) = value.outcome {
        object.insert(
            "outcome".to_string(),
            Value::String(
                match outcome {
                    ShutdownOutcomeRecord::Completed => "completed",
                    ShutdownOutcomeRecord::AbortedBeforeActivation => "aborted_before_activation",
                    ShutdownOutcomeRecord::ReconciliationRequired => "reconciliation_required",
                }
                .to_string(),
            ),
        );
    }
    if let Some(failure) = &value.failure {
        object.insert("failure".to_string(), encode_failure(failure));
    }
    if let Some(value) = value.admission_open {
        object.insert("admission_open".to_string(), Value::Bool(value));
    }
    if let Some(value) = value.retry_quit_same_boot {
        object.insert("retry_quit_same_boot".to_string(), Value::Bool(value));
    }
    Ok(Value::Object(object))
}

fn decode_shutdown_plan(
    object: &Map<String, Value>,
) -> Result<ShutdownPlanRecord, StoredRecordCodecError> {
    let family = StoredRecordFamily::ShutdownPlan;
    let count = |key: &'static str| -> Result<Option<u64>, StoredRecordCodecError> {
        match object.get(key).filter(|value| !value.is_null()) {
            None => Ok(None),
            Some(value) => value
                .as_u64()
                .map(Some)
                .ok_or(StoredRecordCodecError::MissingReference { family, field: key }),
        }
    };
    let outcome = match object.get("outcome").and_then(Value::as_str) {
        None | Some("in_progress") => None,
        Some("completed") => Some(ShutdownOutcomeRecord::Completed),
        Some("aborted_before_activation") => Some(ShutdownOutcomeRecord::AbortedBeforeActivation),
        Some("reconciliation_required") => Some(ShutdownOutcomeRecord::ReconciliationRequired),
        Some(other) => {
            return Err(StoredRecordCodecError::Incompatible {
                family,
                schema: format!("shutdown_outcome.{other}"),
            })
        }
    };
    Ok(ShutdownPlanRecord {
        operation_id: required_text(object, family, "operation_id")?.to_string(),
        intent: decode_quit_intent(object, family)?,
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
            .map(|value| decode_failure(value, family))
            .transpose()?,
        shutdown_effect_count: count("shutdown_effect_count")?,
        admission_open: object.get("admission_open").and_then(Value::as_bool),
        retry_quit_same_boot: object.get("retry_quit_same_boot").and_then(Value::as_bool),
    })
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
            let mut object = serde_json::json!({
                "schema":"shutdown_target_v1",
                "target_id":target_id,
                "kind":match kind {
                    ShutdownTargetKindRecord::WorkflowExecution => "workflow_execution",
                    ShutdownTargetKindRecord::WorkflowNode => "workflow_node",
                },
                "state":shutdown_target_state_label(*state),
                "effect_identity":effect_identity,
            });
            let object = object.as_object_mut().expect("object");
            if let Some(value) = owner_operation_id {
                object.insert(
                    "owner_operation_id".to_string(),
                    Value::String(value.clone()),
                );
            }
            if let Some(value) = failure {
                object.insert("failure".to_string(), encode_failure(value));
            }
            if let Some(value) = recovery_action {
                object.insert(
                    "recovery_action".to_string(),
                    serde_json::json!({
                        "action_id":value.action_id,
                        "origin_revision":value.origin_revision,
                        "action":recovery_action_label(value.action),
                        "state":obligation_state_label(value.state),
                    }),
                );
            }
            Ok(Value::Object(object.clone()))
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

fn decode_shutdown_target(
    object: &Map<String, Value>,
) -> Result<ShutdownTargetRecord, StoredRecordCodecError> {
    let family = StoredRecordFamily::ShutdownTarget;
    match required_text(object, family, "schema")? {
        "shutdown_target_v1" => {
            let target_id = required_text(object, family, "target_id")?.to_string();
            Ok(ShutdownTargetRecord::Target {
                target_id: target_id.clone(),
                kind: match required_text(object, family, "kind")? {
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
                effect_identity: optional_string(object, "effect_identity").unwrap_or(target_id),
                owner_operation_id: optional_string(object, "owner_operation_id"),
                failure: object
                    .get("failure")
                    .map(|value| decode_failure(value, family))
                    .transpose()?,
                recovery_action: object
                    .get("recovery_action")
                    .map(|value| {
                        let value = value
                            .as_object()
                            .ok_or(StoredRecordCodecError::Malformed { family })?;
                        Ok(ShutdownTargetRecoveryRecord {
                            action_id: required_text(value, family, "action_id")?.to_string(),
                            origin_revision: required_u64(value, family, "origin_revision")?,
                            action: decode_recovery_action(
                                required_text(value, family, "action")?,
                                family,
                            )?,
                            state: decode_obligation_state(value, family)?,
                        })
                    })
                    .transpose()?,
            })
        }
        "shutdown_recovery_snapshot_v1" => Ok(ShutdownTargetRecord::RecoverySnapshot {
            obligation_id: required_text(object, family, "obligation_id")?.to_string(),
            ordered_key: optional_string(object, "ordered_key").unwrap_or_default(),
            owner: required_text(object, family, "owner")?.to_string(),
            revision: optional_u64(object, "revision").unwrap_or(0),
            record: Box::new(
                StoredObligationV1::decode(required_text(object, family, "record")?)?.into_value(),
            ),
        }),
        _ => unreachable!("schema was validated"),
    }
}
