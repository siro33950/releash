//! Strict public V1 DTOs for application quit and shutdown.
//!
//! These transport shapes deliberately contain no repository or usecase
//! behavior. Tauri and loopback WebSocket adapters share them through the
//! application-lifecycle presenter.

use serde::{Deserialize, Serialize};

use super::application_operation_v1::{
    RecoveryActionIdentityDtoV1, RecoveryActionKindDtoV1, SafeOperationFailureDtoV1,
};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ApplicationStartupOutcomeDtoV1 {
    Ready,
    Failed {
        kind: StartupFailureKindDtoV1,
        #[serde(rename = "safeDescription")]
        safe_description: String,
        #[serde(rename = "correlationId")]
        correlation_id: String,
        #[serde(rename = "retryOnNextLaunch")]
        retry_on_next_launch: bool,
        actions: [StartupFailureActionDtoV1; 1],
    },
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StartupFailureKindDtoV1 {
    StoreInUse,
    StorageUnavailable,
    UnsupportedRuntime,
    UnsupportedStoreVersion,
    InitializationStateInvalid,
    StoreValidationFailed,
    SchemaEvolutionFailed,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StartupFailureActionDtoV1 {
    Quit,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum StartupFailureQuitOutcomeDtoV1 {
    Accepted {
        #[serde(rename = "correlationId")]
        correlation_id: String,
    },
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ApplicationQuitIntentDtoV1 {
    Exit { code: i32 },
    Restart { code: i32 },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ApplicationQuitRequestDtoV1 {
    pub request_id: String,
    pub intent: ApplicationQuitIntentDtoV1,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ShutdownTargetActionRequestDtoV1 {
    pub action_id: String,
    pub shutdown_id: String,
    pub ordinal: String,
    pub target_key: String,
    pub origin_revision: String,
    pub action: RecoveryActionKindDtoV1,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ApplicationQuitReceiptDtoV1 {
    pub operation_id: String,
    pub shutdown_id: String,
    pub intent: String,
    pub exit_code: i32,
    pub t0_ms: String,
    pub deadline_ms: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ApplicationQuitStateDtoV1 {
    Preparing,
    Activated,
    Completed,
    OutcomeUnknown {
        operation_id: String,
        shutdown_id: String,
        activation_commit_id: String,
    },
    FailedBeforeActivation {
        correlation_id: String,
    },
    ReconciliationRequired {
        correlation_id: String,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ApplicationQuitOutcomeDtoV1 {
    Accepted {
        receipt: ApplicationQuitReceiptDtoV1,
        state: ApplicationQuitStateDtoV1,
    },
    RejectedBeforeCommit {
        correlation_id: String,
    },
    OutcomeUnknown {
        request_id: String,
        operation_id: String,
        intent: ApplicationQuitIntentDtoV1,
    },
    PreviousShutdownReconciliationRequired {
        blocking: Box<ShutdownPlanDtoV1>,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ApplicationQuitLookupDtoV1 {
    Found {
        receipt: ApplicationQuitReceiptDtoV1,
        state: ApplicationQuitStateDtoV1,
    },
    OutcomeUnknown {
        operation_id: String,
        intent: ApplicationQuitIntentDtoV1,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum CurrentShutdownResultDtoV1 {
    Current {
        plan: Option<Box<ShutdownPlanDtoV1>>,
    },
    OutcomeUnknown {
        operation_id: String,
        intent: ApplicationQuitIntentDtoV1,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ShutdownPlanDtoV1 {
    pub shutdown_id: String,
    pub phase: String,
    pub revision: String,
    pub details_state: String,
    pub operation_id: String,
    pub intent: String,
    pub exit_code: i32,
    pub t0_ms: String,
    pub preparation_cutoff_ms: String,
    pub deadline_ms: String,
    pub target_count: Option<String>,
    pub prepared_count: Option<String>,
    pub effect_reserved_count: Option<String>,
    pub terminal_count: Option<String>,
    pub completed_count: Option<String>,
    pub unresolved_count: Option<String>,
    pub recovery_snapshot_count: Option<String>,
    pub recovery_snapshot_id: Option<String>,
    pub outcome: Option<String>,
    pub safe_failure: Option<SafeOperationFailureDtoV1>,
    pub actions: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ShutdownTargetDtoV1 {
    pub ordinal: String,
    pub target_key: String,
    pub target_id: String,
    pub kind: String,
    pub effect_identity: String,
    pub state: String,
    pub observation: Option<SafeEffectObservationDtoV1>,
    pub revision: String,
    pub actions: Vec<String>,
    pub action_identities: Vec<RecoveryActionIdentityDtoV1>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum SafeEffectObservationDtoV1 {
    ExitCoupledOutcomeUnknown { shutdown_id: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ShutdownPlanPageDtoV1 {
    pub plan: ShutdownPlanDtoV1,
    pub targets: Vec<ShutdownTargetDtoV1>,
    pub next_cursor: Option<String>,
}

#[cfg(test)]
mod b075_tests {
    use super::*;

    fn assert_string_field<T>(_: fn(&T) -> &String) {}

    fn assert_optional_string_field<T>(_: fn(&T) -> &Option<String>) {}

    #[test]
    fn b071_startup_failure_quit_uses_the_fixed_closed_wire_shape() {
        let value = serde_json::to_value(StartupFailureQuitOutcomeDtoV1::Accepted {
            correlation_id: "startup-correlation".to_string(),
        })
        .expect("serialize startup failure Quit result");
        assert_eq!(
            value,
            serde_json::json!({
                "type": "accepted",
                "correlationId": "startup-correlation",
            })
        );
    }

    #[test]
    fn b075_all_application_lifecycle_struct_semantic_integer_fields_are_strings() {
        assert_string_field(|value: &ShutdownTargetActionRequestDtoV1| &value.ordinal);
        assert_string_field(|value: &ShutdownTargetActionRequestDtoV1| &value.origin_revision);
        assert_string_field(|value: &ApplicationQuitReceiptDtoV1| &value.t0_ms);
        assert_string_field(|value: &ApplicationQuitReceiptDtoV1| &value.deadline_ms);
        assert_string_field(|value: &ShutdownPlanDtoV1| &value.revision);
        assert_string_field(|value: &ShutdownPlanDtoV1| &value.t0_ms);
        assert_string_field(|value: &ShutdownPlanDtoV1| &value.preparation_cutoff_ms);
        assert_string_field(|value: &ShutdownPlanDtoV1| &value.deadline_ms);
        assert_optional_string_field(|value: &ShutdownPlanDtoV1| &value.target_count);
        assert_optional_string_field(|value: &ShutdownPlanDtoV1| &value.prepared_count);
        assert_optional_string_field(|value: &ShutdownPlanDtoV1| &value.effect_reserved_count);
        assert_optional_string_field(|value: &ShutdownPlanDtoV1| &value.terminal_count);
        assert_optional_string_field(|value: &ShutdownPlanDtoV1| &value.completed_count);
        assert_optional_string_field(|value: &ShutdownPlanDtoV1| &value.unresolved_count);
        assert_optional_string_field(|value: &ShutdownPlanDtoV1| &value.recovery_snapshot_count);
        assert_string_field(|value: &ShutdownTargetDtoV1| &value.ordinal);
        assert_string_field(|value: &ShutdownTargetDtoV1| &value.revision);
    }

    #[test]
    fn b075_shutdown_action_rejects_json_numbers_for_each_semantic_integer_field() {
        for field in ["ordinal", "origin_revision"] {
            let mut raw = serde_json::json!({
                "action_id": "action-1",
                "shutdown_id": "plan-1",
                "ordinal": "0",
                "target_key": "target-1",
                "origin_revision": "0",
                "action": "read_again",
            });
            raw[field] = serde_json::json!(0);
            assert!(
                serde_json::from_value::<ShutdownTargetActionRequestDtoV1>(raw).is_err(),
                "{field} accepted a JSON number"
            );
        }
    }

    #[test]
    fn b075_exit_code_remains_a_signed_json_integer() {
        for code in [i32::MIN, -1, 0, 1, i32::MAX] {
            let value = serde_json::json!({
                "request_id": "quit-1",
                "intent": { "type": "exit", "code": code },
            });
            let request = serde_json::from_value::<ApplicationQuitRequestDtoV1>(value).unwrap();
            let encoded = serde_json::to_value(request).unwrap();
            assert_eq!(encoded["intent"]["code"], code);
            assert!(encoded["intent"]["code"].is_i64());
        }
        for invalid in [serde_json::json!("1"), serde_json::json!(1.5)] {
            let value = serde_json::json!({
                "request_id": "quit-invalid",
                "intent": { "type": "exit", "code": invalid },
            });
            assert!(serde_json::from_value::<ApplicationQuitRequestDtoV1>(value).is_err());
        }
    }

    #[test]
    fn b075_application_lifecycle_uses_one_shutdown_identity() {
        let maximum = i64::MAX.to_string();
        let state = serde_json::to_value(ApplicationQuitStateDtoV1::OutcomeUnknown {
            operation_id: "quit-operation-1".to_string(),
            shutdown_id: "plan-1".to_string(),
            activation_commit_id: "commit-1".to_string(),
        })
        .unwrap();

        let observation =
            serde_json::to_value(SafeEffectObservationDtoV1::ExitCoupledOutcomeUnknown {
                shutdown_id: "plan-1".to_string(),
            })
            .unwrap();
        assert_eq!(state["shutdown_id"], "plan-1");
        assert_eq!(observation["shutdown_id"], "plan-1");
        assert!(state.get("plan_id").is_none());
        assert!(state.get("epoch").is_none());
        assert!(observation.get("epoch").is_none());
        assert_eq!(maximum, "9223372036854775807");
    }
}
