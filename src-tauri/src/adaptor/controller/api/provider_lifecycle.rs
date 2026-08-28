use std::sync::Arc;

use axum::extract::rejection::JsonRejection;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};

use crate::domain::provider_lifecycle::{
    ProviderKind, ProviderLifecycleIngressResult, ProviderLifecycleRejection,
    ProviderLifecycleScope, ProviderLifecycleSignal, ProviderLifecycleSlotId,
    ProviderLifecycleUnavailableObservation, ProviderLifecycleUnavailableReason,
};
use crate::usecase::provider_lifecycle::ProviderLifecycleIngressUsecaseError;

use super::error::ApiError;
use crate::adaptor::protocol::provider_lifecycle::{
    ProviderActivityRequest, ProviderLifecycleProvider, ProviderLifecycleReceiveRequest,
    ProviderLifecycleReceiveResponse, ProviderLifecycleSignalRequest,
    ProviderLifecycleUnavailableReasonRequest, ProviderLifecycleUnavailableRequest,
};
#[derive(Clone)]
struct ProviderLifecycleApiState {
    usecase: Option<Arc<dyn crate::usecase::provider_lifecycle::ProviderLifecycleIngressPort>>,
}

pub(crate) fn router(
    usecase: Option<Arc<dyn crate::usecase::provider_lifecycle::ProviderLifecycleIngressPort>>,
) -> Router {
    Router::new()
        .route("/v1/provider-lifecycle/signals", post(receive))
        .route(
            "/v1/provider-lifecycle/unavailable",
            post(report_unavailable),
        )
        .with_state(ProviderLifecycleApiState { usecase })
}

async fn receive(
    State(state): State<ProviderLifecycleApiState>,
    payload: Result<Json<ProviderLifecycleReceiveRequest>, JsonRejection>,
) -> Result<Json<ProviderLifecycleReceiveResponse>, ApiError> {
    let ingress_started = std::time::Instant::now();
    let Json(payload) = payload.map_err(|error| ApiError::invalid_request(error.body_text()))?;
    let usecase = state.usecase.ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "provider_lifecycle_unavailable",
            "Provider lifecycle service is unavailable",
        )
    })?;
    let provider = match payload.provider {
        ProviderLifecycleProvider::Claude => ProviderKind::Claude,
        ProviderLifecycleProvider::Codex => ProviderKind::Codex,
    };
    let slot_id = ProviderLifecycleSlotId::new(&payload.slot_id)
        .map_err(|error| ApiError::invalid_request(error.to_string()))?;
    let scope = ProviderLifecycleScope::new(payload.agent_session_id)
        .map_err(|error| ApiError::invalid_request(error.to_string()))?;
    let is_session_started = matches!(
        &payload.signal,
        ProviderLifecycleSignalRequest::SessionStarted { .. }
    );
    let signal = match payload.signal {
        ProviderLifecycleSignalRequest::SessionStarted {
            provider_session_id,
            transcript_ref,
        } => ProviderLifecycleSignal::session_started(
            &payload.binding_id,
            provider,
            scope,
            provider_session_id,
            transcript_ref.as_deref(),
        ),
        ProviderLifecycleSignalRequest::StopObserved {
            provider_session_id,
            transcript_ref,
        } => ProviderLifecycleSignal::stop_observed(
            &payload.binding_id,
            provider,
            scope,
            provider_session_id,
            transcript_ref.as_deref(),
        ),
        ProviderLifecycleSignalRequest::StopFailed {
            provider_session_id,
            transcript_ref,
            reason,
        } => ProviderLifecycleSignal::stop_failed(
            &payload.binding_id,
            provider,
            scope,
            provider_session_id,
            transcript_ref.as_deref(),
            reason,
        ),
        ProviderLifecycleSignalRequest::ActivityObserved {
            provider_session_id,
            transcript_ref,
            activity,
        } => ProviderLifecycleSignal::activity_observed(
            &payload.binding_id,
            provider,
            scope,
            provider_session_id,
            transcript_ref.as_deref(),
            match activity {
                ProviderActivityRequest::Working => {
                    crate::domain::workflow::AgentSessionActivity::Working
                }
                ProviderActivityRequest::AwaitingAnswer => {
                    crate::domain::workflow::AgentSessionActivity::AwaitingAnswer
                }
                ProviderActivityRequest::AwaitingInstruction => {
                    crate::domain::workflow::AgentSessionActivity::AwaitingInstruction
                }
            },
        ),
    }
    .map_err(|error| ApiError::invalid_request(error.to_string()))?;

    let result = usecase
        .receive(&slot_id, &payload.capability, signal)
        .await
        .map_err(usecase_error)?;
    if is_session_started {
        crate::other::telemetry::record_terminal_launch(
            crate::other::telemetry::TerminalLaunch::HookIngress,
            ingress_started.elapsed(),
        );
    }
    Ok(Json(response(result)))
}

async fn report_unavailable(
    State(state): State<ProviderLifecycleApiState>,
    payload: Result<Json<ProviderLifecycleUnavailableRequest>, JsonRejection>,
) -> Result<Json<ProviderLifecycleReceiveResponse>, ApiError> {
    let Json(payload) = payload.map_err(|error| ApiError::invalid_request(error.body_text()))?;
    let usecase = state.usecase.ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "provider_lifecycle_unavailable",
            "Provider lifecycle service is unavailable",
        )
    })?;
    let provider = match payload.provider {
        ProviderLifecycleProvider::Claude => ProviderKind::Claude,
        ProviderLifecycleProvider::Codex => ProviderKind::Codex,
    };
    let slot_id = ProviderLifecycleSlotId::new(&payload.slot_id)
        .map_err(|error| ApiError::invalid_request(error.to_string()))?;
    let scope = ProviderLifecycleScope::new(payload.agent_session_id)
        .map_err(|error| ApiError::invalid_request(error.to_string()))?;
    let reason = match payload.reason {
        ProviderLifecycleUnavailableReasonRequest::SessionStartDeadlineExceeded => {
            ProviderLifecycleUnavailableReason::SessionStartDeadlineExceeded
        }
        ProviderLifecycleUnavailableReasonRequest::CodexHookDeliveryUnconfirmed => {
            ProviderLifecycleUnavailableReason::CodexHookDeliveryUnconfirmed
        }
        ProviderLifecycleUnavailableReasonRequest::ProviderHookConfigurationRejected => {
            ProviderLifecycleUnavailableReason::ProviderHookConfigurationRejected
        }
        ProviderLifecycleUnavailableReasonRequest::LocalApiUnavailable => {
            ProviderLifecycleUnavailableReason::LocalApiUnavailable
        }
    };
    let observation =
        ProviderLifecycleUnavailableObservation::new(payload.binding_id, provider, scope, reason)
            .map_err(|error| ApiError::invalid_request(error.to_string()))?;
    let result = usecase
        .report_unavailable(&slot_id, &payload.capability, observation)
        .await
        .map_err(usecase_error)?;
    Ok(Json(response(result)))
}

fn response(result: ProviderLifecycleIngressResult) -> ProviderLifecycleReceiveResponse {
    match result {
        ProviderLifecycleIngressResult::Applied => ProviderLifecycleReceiveResponse::Applied,
        ProviderLifecycleIngressResult::Duplicate => ProviderLifecycleReceiveResponse::Duplicate,
        ProviderLifecycleIngressResult::Rejected(reason) => {
            ProviderLifecycleReceiveResponse::Rejected {
                reason: rejection_reason(reason).to_string(),
            }
        }
    }
}

fn rejection_reason(reason: ProviderLifecycleRejection) -> &'static str {
    match reason {
        ProviderLifecycleRejection::BindingNotActive => "binding_not_active",
        ProviderLifecycleRejection::InvalidCapability => "invalid_capability",
        ProviderLifecycleRejection::BindingMismatch => "binding_mismatch",
        ProviderLifecycleRejection::ProviderMismatch => "provider_mismatch",
        ProviderLifecycleRejection::ScopeMismatch => "scope_mismatch",
        ProviderLifecycleRejection::BindingExpired => "binding_expired",
        ProviderLifecycleRejection::SessionAlreadyAssociated => "session_already_associated",
        ProviderLifecycleRejection::SessionNotAssociated => "session_not_associated",
        ProviderLifecycleRejection::ProviderSessionMismatch => "provider_session_mismatch",
        ProviderLifecycleRejection::TranscriptMismatch => "transcript_mismatch",
    }
}

fn usecase_error(error: ProviderLifecycleIngressUsecaseError) -> ApiError {
    match error {
        ProviderLifecycleIngressUsecaseError::InvalidInput => {
            ApiError::invalid_request("Provider lifecycle input is invalid")
        }
        ProviderLifecycleIngressUsecaseError::Conflict => ApiError::new(
            StatusCode::CONFLICT,
            "provider_lifecycle_conflict",
            "Provider lifecycle conflicts with current AgentSession ownership",
        ),
        ProviderLifecycleIngressUsecaseError::StorageUnavailable => ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "provider_lifecycle_storage_unavailable",
            "Provider lifecycle persistence is unavailable",
        ),
        ProviderLifecycleIngressUsecaseError::Corrupt => {
            ApiError::internal("Provider lifecycle state is corrupt")
        }
    }
}

#[cfg(test)]
#[path = "provider_lifecycle_controller_test.rs"]
mod provider_lifecycle_controller_tests;
