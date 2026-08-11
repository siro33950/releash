//! Lossless application-lifecycle presentation shared by every transport.

use crate::adaptor::protocol::application_lifecycle_v1::{
    ApplicationQuitIntentDtoV1, ApplicationQuitLookupDtoV1, ApplicationQuitOutcomeDtoV1,
    ApplicationQuitReceiptDtoV1, ApplicationQuitStateDtoV1, ApplicationStartupOutcomeDtoV1,
    CurrentShutdownResultDtoV1, SafeEffectObservationDtoV1, ShutdownPlanDtoV1,
    ShutdownPlanPageDtoV1, ShutdownTargetDtoV1, StartupFailureActionDtoV1, StartupFailureKindDtoV1,
};
use crate::adaptor::protocol::application_operation_v1::{
    ApplicationQuitErrorDtoV1, ApplicationQuitLookupErrorDtoV1, CurrentShutdownErrorDtoV1,
    OperationApplicationErrorDtoV1, ShutdownDetailsMutationErrorDtoV1, ShutdownPlanQueryErrorDtoV1,
};
use crate::domain::local_event::{ApplicationShutdownPhase, ShutdownDetailsState};
use crate::usecase::shutdown_coordinator::{
    ApplicationProcessAction, ApplicationQuitIntent, ApplicationQuitOutcome,
    ApplicationQuitProjection, ApplicationQuitState, ApplicationShutdownPlanPageReadModel,
    ApplicationShutdownPlanReadModel, CurrentApplicationShutdownProjection,
};

pub(crate) fn application_startup_outcome(
    value: crate::usecase::application_startup::ApplicationStartupOutcome,
) -> ApplicationStartupOutcomeDtoV1 {
    use crate::usecase::application_startup::{
        ApplicationStartupOutcome as O, StartupFailureKind as K,
    };
    match value {
        O::Ready => ApplicationStartupOutcomeDtoV1::Ready,
        O::Failed(failure) => ApplicationStartupOutcomeDtoV1::Failed {
            kind: match failure.kind {
                K::StoreInUse => StartupFailureKindDtoV1::StoreInUse,
                K::StorageUnavailable => StartupFailureKindDtoV1::StorageUnavailable,
                K::UnsupportedRuntime => StartupFailureKindDtoV1::UnsupportedRuntime,
                K::UnsupportedStoreVersion => StartupFailureKindDtoV1::UnsupportedStoreVersion,
                K::InitializationStateInvalid => {
                    StartupFailureKindDtoV1::InitializationStateInvalid
                }
                K::StoreValidationFailed => StartupFailureKindDtoV1::StoreValidationFailed,
                K::SchemaEvolutionFailed => StartupFailureKindDtoV1::SchemaEvolutionFailed,
            },
            safe_description: failure.safe_description.to_string(),
            correlation_id: failure.correlation_id,
            retry_on_next_launch: failure.retry_on_next_launch,
            actions: [StartupFailureActionDtoV1::Quit],
        },
    }
}

pub(crate) fn presentation_error(context: &str, detail: &str) -> OperationApplicationErrorDtoV1 {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(format!(
        "application-lifecycle-presentation/v1\0{context}\0{detail}"
    ));
    OperationApplicationErrorDtoV1::Internal {
        correlation_id: format!("presentation-{}", &hex::encode(digest)[..24]),
    }
}

pub(crate) fn application_quit_error(
    error: crate::usecase::shutdown_coordinator::ApplicationQuitError,
) -> Option<ApplicationQuitErrorDtoV1> {
    use crate::usecase::shutdown_coordinator::ApplicationQuitError as E;
    match error {
        E::InvalidRequest => Some(ApplicationQuitErrorDtoV1::InvalidRequest),
        E::PayloadConflict => Some(ApplicationQuitErrorDtoV1::PayloadConflict),
        E::CapacityExceeded => Some(ApplicationQuitErrorDtoV1::CapacityExceeded),
        E::PreviousShutdownReconciliationRequired { .. } => None,
        E::Internal { correlation_id } => {
            Some(ApplicationQuitErrorDtoV1::Internal { correlation_id })
        }
    }
}

pub(crate) fn application_quit_failure(
    error: crate::usecase::shutdown_coordinator::ApplicationQuitError,
) -> Result<ApplicationQuitOutcomeDtoV1, ApplicationQuitErrorDtoV1> {
    use crate::usecase::shutdown_coordinator::ApplicationQuitError as E;
    match error {
        E::PreviousShutdownReconciliationRequired { blocking } => Ok(
            ApplicationQuitOutcomeDtoV1::PreviousShutdownReconciliationRequired {
                blocking: Box::new(plan(*blocking)),
            },
        ),
        other => Err(application_quit_error(other)
            .expect("non-blocking application quit errors always have a public error")),
    }
}

fn presentation_correlation(context: &str, detail: &str) -> String {
    match presentation_error(context, detail) {
        OperationApplicationErrorDtoV1::Internal { correlation_id } => correlation_id,
        _ => unreachable!("presentation_error always returns Internal"),
    }
}

fn query_correlation(error: crate::domain::local_event::LocalEventQueryError) -> String {
    use crate::domain::local_event::LocalEventQueryError as E;
    match error {
        E::StorageUnavailable { failure } => failure.correlation_id,
        E::Corrupt { correlation_id }
        | E::IncompatibleStoredEvent { correlation_id }
        | E::Internal { correlation_id } => correlation_id,
        other => presentation_correlation("application_lifecycle_query", &format!("{other:?}")),
    }
}

pub(crate) fn application_quit_lookup_error(
    error: crate::domain::local_event::LocalEventQueryError,
) -> ApplicationQuitLookupErrorDtoV1 {
    use crate::domain::local_event::LocalEventQueryError as E;
    match error {
        E::InvalidRequest | E::SnapshotMismatch => ApplicationQuitLookupErrorDtoV1::InvalidRequest,
        E::NotFound => ApplicationQuitLookupErrorDtoV1::NotFound,
        E::QueryBusy => ApplicationQuitLookupErrorDtoV1::QueryBusy,
        E::DeadlineExceeded => ApplicationQuitLookupErrorDtoV1::DeadlineExceeded,
        E::StorageUnavailable { failure } => ApplicationQuitLookupErrorDtoV1::StorageUnavailable {
            failure: failure.into(),
        },
        other => ApplicationQuitLookupErrorDtoV1::Internal {
            correlation_id: query_correlation(other),
        },
    }
}

pub(crate) fn current_shutdown_error(
    error: crate::domain::local_event::LocalEventQueryError,
) -> CurrentShutdownErrorDtoV1 {
    // The current-shutdown selector is an authority query, not a best-effort
    // resource lookup.  Any storage/decode/integrity/reference failure leaves
    // the current identity unknowable and therefore closes as Internal rather
    // than being confused with Current(None) or another recoverable surface.
    CurrentShutdownErrorDtoV1::Internal {
        correlation_id: query_correlation(error),
    }
}

pub(crate) fn shutdown_plan_query_error(
    error: crate::domain::local_event::LocalEventQueryError,
) -> ShutdownPlanQueryErrorDtoV1 {
    use crate::domain::local_event::LocalEventQueryError as E;
    match error {
        E::InvalidRequest | E::SnapshotMismatch => ShutdownPlanQueryErrorDtoV1::InvalidRequest,
        E::NotFound => ShutdownPlanQueryErrorDtoV1::NotFound,
        E::DetailsCompacted => ShutdownPlanQueryErrorDtoV1::DetailsCompacted,
        E::CursorMismatch => ShutdownPlanQueryErrorDtoV1::CursorMismatch,
        E::CursorExpired => ShutdownPlanQueryErrorDtoV1::CursorExpired,
        E::QueryBusy => ShutdownPlanQueryErrorDtoV1::QueryBusy,
        E::DeadlineExceeded => ShutdownPlanQueryErrorDtoV1::DeadlineExceeded,
        E::ResponseTooLarge => ShutdownPlanQueryErrorDtoV1::ResponseTooLarge,
        E::StorageUnavailable { failure } => ShutdownPlanQueryErrorDtoV1::StorageUnavailable {
            failure: failure.into(),
        },
        other => ShutdownPlanQueryErrorDtoV1::Internal {
            correlation_id: query_correlation(other),
        },
    }
}

pub(crate) fn shutdown_details_mutation_error(
    error: crate::usecase::shutdown_coordinator::ApplicationQuitError,
) -> ShutdownDetailsMutationErrorDtoV1 {
    match error {
        crate::usecase::shutdown_coordinator::ApplicationQuitError::InvalidRequest => {
            ShutdownDetailsMutationErrorDtoV1::InvalidRequest
        }
        crate::usecase::shutdown_coordinator::ApplicationQuitError::Internal { correlation_id } => {
            ShutdownDetailsMutationErrorDtoV1::Internal { correlation_id }
        }
        other => ShutdownDetailsMutationErrorDtoV1::Internal {
            correlation_id: presentation_correlation(
                "shutdown_details_mutation",
                &format!("{other:?}"),
            ),
        },
    }
}

pub(crate) fn state(value: ApplicationQuitState) -> ApplicationQuitStateDtoV1 {
    match value {
        ApplicationQuitState::Preparing => ApplicationQuitStateDtoV1::Preparing,
        ApplicationQuitState::Activated => ApplicationQuitStateDtoV1::Activated,
        ApplicationQuitState::Completed => ApplicationQuitStateDtoV1::Completed,
        ApplicationQuitState::OutcomeUnknown {
            operation_id,
            shutdown_id,
            activation_commit_id,
        } => ApplicationQuitStateDtoV1::OutcomeUnknown {
            operation_id,
            shutdown_id,
            activation_commit_id,
        },
        ApplicationQuitState::FailedBeforeActivation { failure } => {
            ApplicationQuitStateDtoV1::FailedBeforeActivation {
                correlation_id: failure.correlation_id,
            }
        }
        ApplicationQuitState::ReconciliationRequired { failure } => {
            ApplicationQuitStateDtoV1::ReconciliationRequired {
                correlation_id: failure.correlation_id,
            }
        }
    }
}

pub(crate) fn intent(value: ApplicationQuitIntent) -> ApplicationQuitIntentDtoV1 {
    match value {
        ApplicationQuitIntent::Exit { code } => ApplicationQuitIntentDtoV1::Exit { code },
        ApplicationQuitIntent::Restart { code } => ApplicationQuitIntentDtoV1::Restart { code },
    }
}

pub(crate) fn application_quit_outcome(
    value: ApplicationQuitOutcome,
) -> (
    ApplicationQuitOutcomeDtoV1,
    Option<ApplicationProcessAction>,
) {
    match value {
        ApplicationQuitOutcome::Accepted { receipt, state } => {
            let process_action = state
                .grants_exit_permit()
                .then(|| ApplicationProcessAction::from(receipt.intent));
            (
                ApplicationQuitOutcomeDtoV1::Accepted {
                    receipt: self::receipt(receipt),
                    state: self::state(state),
                },
                process_action,
            )
        }
        ApplicationQuitOutcome::RejectedBeforeCommit { failure } => (
            ApplicationQuitOutcomeDtoV1::RejectedBeforeCommit {
                correlation_id: failure.correlation_id,
            },
            None,
        ),
        ApplicationQuitOutcome::OutcomeUnknown {
            request_id,
            operation_id,
            intent: value,
        } => (
            ApplicationQuitOutcomeDtoV1::OutcomeUnknown {
                request_id,
                operation_id,
                intent: intent(value),
            },
            None,
        ),
    }
}

pub(crate) fn application_quit_lookup(
    value: ApplicationQuitProjection,
) -> ApplicationQuitLookupDtoV1 {
    match value {
        ApplicationQuitProjection::Shutdown { receipt, state } => {
            ApplicationQuitLookupDtoV1::Found {
                receipt: self::receipt(receipt),
                state: self::state(state),
            }
        }
        ApplicationQuitProjection::OutcomeUnknown {
            operation_id,
            intent: value,
        } => ApplicationQuitLookupDtoV1::OutcomeUnknown {
            operation_id,
            intent: intent(value),
        },
    }
}

pub(crate) fn current_shutdown(
    value: CurrentApplicationShutdownProjection,
) -> CurrentShutdownResultDtoV1 {
    match value {
        CurrentApplicationShutdownProjection::Current(value) => {
            CurrentShutdownResultDtoV1::Current {
                plan: value.map(|value| Box::new(plan(*value))),
            }
        }
        CurrentApplicationShutdownProjection::OutcomeUnknown {
            operation_id,
            intent: value,
        } => CurrentShutdownResultDtoV1::OutcomeUnknown {
            operation_id,
            intent: intent(value),
        },
    }
}

pub(crate) fn receipt(
    value: crate::usecase::shutdown_coordinator::ApplicationQuitReceipt,
) -> ApplicationQuitReceiptDtoV1 {
    let (intent, exit_code) = match value.intent {
        ApplicationQuitIntent::Exit { code } => ("exit".to_string(), code),
        ApplicationQuitIntent::Restart { code } => ("restart".to_string(), code),
    };
    ApplicationQuitReceiptDtoV1 {
        operation_id: value.operation_id,
        shutdown_id: value.shutdown_id,
        intent,
        exit_code,
        t0_ms: value.t0_ms.to_string(),
        deadline_ms: value.deadline_ms.to_string(),
    }
}

fn phase(value: ApplicationShutdownPhase) -> &'static str {
    match value {
        ApplicationShutdownPhase::Prepared => "preparing",
        ApplicationShutdownPhase::Activated => "activated",
        ApplicationShutdownPhase::Quiescing => "quiescing",
        ApplicationShutdownPhase::Completed => "completed",
        ApplicationShutdownPhase::Failed => "failed",
        ApplicationShutdownPhase::Cancelled => "cancelled",
        ApplicationShutdownPhase::ReconciliationRequired => "reconciliation_required",
    }
}

pub(crate) fn plan(value: ApplicationShutdownPlanReadModel) -> ShutdownPlanDtoV1 {
    ShutdownPlanDtoV1 {
        shutdown_id: value.plan.shutdown_id,
        phase: phase(value.phase).to_string(),
        revision: value.revision.value().to_string(),
        details_state: match value.details_state {
            ShutdownDetailsState::Available => "available",
            ShutdownDetailsState::Compacted => "compacted",
        }
        .to_string(),
        operation_id: value.operation_id,
        intent: value.intent,
        exit_code: value.exit_code,
        t0_ms: value.t0_ms.to_string(),
        preparation_cutoff_ms: value.preparation_cutoff_ms.to_string(),
        deadline_ms: value.deadline_ms.to_string(),
        target_count: value.target_count.map(|count| count.to_string()),
        prepared_count: value.prepared_count.map(|count| count.to_string()),
        effect_reserved_count: value.effect_reserved_count.map(|count| count.to_string()),
        terminal_count: value.terminal_count.map(|count| count.to_string()),
        completed_count: value.completed_count.map(|count| count.to_string()),
        unresolved_count: value.unresolved_count.map(|count| count.to_string()),
        recovery_snapshot_count: value.recovery_snapshot_count.map(|count| count.to_string()),
        recovery_snapshot_id: value.recovery_snapshot_id,
        outcome: value.outcome,
        safe_failure: value.safe_failure.map(Into::into),
        actions: value.actions,
    }
}

pub(crate) fn plan_page(value: ApplicationShutdownPlanPageReadModel) -> ShutdownPlanPageDtoV1 {
    let targets = value
        .targets
        .into_iter()
        .map(|target| ShutdownTargetDtoV1 {
            ordinal: target.ordinal.to_string(),
            target_key: target.target_key,
            target_id: target.target_id,
            kind: target.kind,
            effect_identity: target.effect_identity,
            observation: target.observation.map(|observation| match observation {
                crate::domain::local_event::SafeEffectObservation::ExitCoupledOutcomeUnknown {
                    shutdown_id,
                } => SafeEffectObservationDtoV1::ExitCoupledOutcomeUnknown { shutdown_id },
            }),
            actions: target.actions,
            action_identities: target
                .action_identities
                .into_iter()
                .map(|identity| {
                    crate::adaptor::protocol::application_operation_v1::RecoveryActionIdentityDtoV1 {
                        action_id: identity.action_id,
                        action: identity.action.into(),
                        origin_revision: identity.origin_revision.to_string(),
                    }
                })
                .collect(),
            state: target.state,
            revision: target.revision.value().to_string(),
        })
        .collect();
    ShutdownPlanPageDtoV1 {
        plan: plan(value.plan),
        targets,
        next_cursor: value.next_cursor,
    }
}

pub(crate) fn checked_plan_page(
    value: ApplicationShutdownPlanPageReadModel,
) -> Result<ShutdownPlanPageDtoV1, crate::domain::local_event::LocalEventQueryError> {
    const MAX_ENCODED_BYTES: usize = 1024 * 1024;
    let page = plan_page(value);
    let encoded = serde_json::to_vec(&page).map_err(|_| {
        crate::domain::local_event::LocalEventQueryError::Internal {
            correlation_id: match presentation_error("shutdown_plan_page", "serialization failed") {
                OperationApplicationErrorDtoV1::Internal { correlation_id } => correlation_id,
                _ => unreachable!("presentation errors are always Internal"),
            },
        }
    })?;
    if encoded.len() > MAX_ENCODED_BYTES {
        return Err(crate::domain::local_event::LocalEventQueryError::ResponseTooLarge);
    }
    Ok(page)
}

#[cfg(test)]
mod tests {
    use super::{application_quit_outcome, application_startup_outcome, plan};
    use crate::domain::local_event::{
        ApplicationShutdownPhase, Revision, SafeOperationFailure, SessionOperationFailureKind,
        ShutdownDetailsState, ShutdownPlanKey,
    };
    use crate::usecase::shutdown_coordinator::{
        ApplicationQuitIntent, ApplicationQuitOutcome, ApplicationQuitReceipt,
        ApplicationQuitState, ApplicationShutdownPlanReadModel,
    };

    #[test]
    fn b071_failed_startup_presents_only_safe_fields_and_process_local_quit() {
        use crate::usecase::application_startup::{
            ApplicationStartupAuthority, StartupFailureKind,
        };

        for kind in [
            StartupFailureKind::StoreInUse,
            StartupFailureKind::StorageUnavailable,
            StartupFailureKind::UnsupportedRuntime,
            StartupFailureKind::UnsupportedStoreVersion,
            StartupFailureKind::InitializationStateInvalid,
            StartupFailureKind::StoreValidationFailed,
            StartupFailureKind::SchemaEvolutionFailed,
        ] {
            let dto = application_startup_outcome(
                ApplicationStartupAuthority::failed_kind(kind).outcome(),
            );
            let value = serde_json::to_value(dto).expect("serialize startup failure");
            assert_eq!(value["type"], "failed");
            assert_eq!(value["actions"], serde_json::json!(["quit"]));
            let keys = value
                .as_object()
                .expect("startup failure DTO object")
                .keys()
                .map(String::as_str)
                .collect::<std::collections::BTreeSet<_>>();
            assert_eq!(
                keys,
                std::collections::BTreeSet::from([
                    "type",
                    "kind",
                    "safeDescription",
                    "correlationId",
                    "retryOnNextLaunch",
                    "actions",
                ]),
                "startup failure public representation must remain closed"
            );
            assert!(value.get("session").is_none());
            assert!(value.get("workflow").is_none());
            assert!(value.get("progress").is_none());
            assert!(value.get("operation_id").is_none());
            assert!(value.get("shutdown_id").is_none());
        }
    }

    #[test]
    fn f11_exit_permit_preserves_restart_as_a_distinct_process_destination() {
        let present = |intent| {
            application_quit_outcome(ApplicationQuitOutcome::Accepted {
                receipt: ApplicationQuitReceipt {
                    operation_id: "quit-operation-1".to_string(),
                    shutdown_id: "quit-plan-1".to_string(),
                    intent,
                    t0_ms: 10,
                    deadline_ms: 15_010,
                },
                state: ApplicationQuitState::Completed,
            })
            .1
        };

        let exit = present(ApplicationQuitIntent::Exit { code: 42 });
        let restart = present(ApplicationQuitIntent::Restart { code: 42 });

        assert_ne!(
            exit, restart,
            "the presenter must not collapse Exit and Restart into Option<i32>"
        );
    }

    #[test]
    fn b075_compacted_shutdown_presentation_preserves_all_semantic_integers_losslessly() {
        let maximum = i64::MAX;
        let dto = plan(ApplicationShutdownPlanReadModel {
            plan: ShutdownPlanKey {
                shutdown_id: "quit-operation-1".to_string(),
            },
            phase: ApplicationShutdownPhase::Failed,
            details_state: ShutdownDetailsState::Compacted,
            revision: Revision::new(maximum).expect("maximum public revision"),
            operation_id: "quit-operation-1".to_string(),
            intent: "restart".to_string(),
            exit_code: 7,
            t0_ms: maximum,
            preparation_cutoff_ms: maximum,
            deadline_ms: maximum,
            target_count: Some(maximum),
            prepared_count: Some(maximum),
            effect_reserved_count: Some(maximum),
            terminal_count: Some(maximum),
            completed_count: Some(maximum),
            unresolved_count: Some(maximum),
            recovery_snapshot_count: Some(maximum),
            recovery_snapshot_id: Some("snapshot-1".to_string()),
            outcome: Some("exited_with_recovery".to_string()),
            safe_failure: Some(SafeOperationFailure::new(
                SessionOperationFailureKind::DeadlineExceeded,
                true,
                "Shutdown exited with durable recovery work.",
                "shutdown-failure-1".to_string(),
            )),
            actions: Vec::new(),
        });

        assert_eq!(dto.shutdown_id, "quit-operation-1");
        let maximum = maximum.to_string();
        assert_eq!(dto.revision, maximum);
        assert_eq!(dto.details_state, "compacted");
        assert_eq!(dto.intent, "restart");
        assert_eq!(dto.t0_ms, maximum);
        assert_eq!(dto.preparation_cutoff_ms, maximum);
        assert_eq!(dto.deadline_ms, maximum);
        assert_eq!(dto.target_count.as_deref(), Some(maximum.as_str()));
        assert_eq!(dto.prepared_count.as_deref(), Some(maximum.as_str()));
        assert_eq!(dto.effect_reserved_count.as_deref(), Some(maximum.as_str()));
        assert_eq!(dto.terminal_count.as_deref(), Some(maximum.as_str()));
        assert_eq!(dto.completed_count.as_deref(), Some(maximum.as_str()));
        assert_eq!(dto.unresolved_count.as_deref(), Some(maximum.as_str()));
        assert_eq!(
            dto.recovery_snapshot_count.as_deref(),
            Some(maximum.as_str())
        );
        assert_eq!(dto.recovery_snapshot_id.as_deref(), Some("snapshot-1"));
        assert_eq!(dto.outcome.as_deref(), Some("exited_with_recovery"));
        assert!(dto.safe_failure.is_some());
    }
}
