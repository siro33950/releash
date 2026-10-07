use std::sync::Arc;

use axum::extract::rejection::JsonRejection;
use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};

use crate::domain::provider_lifecycle::{
    ProviderKind, ProviderLifecycleIngressResult, ProviderLifecycleScope, ProviderLifecycleSignal,
    ProviderLifecycleSlotId, ProviderLifecycleUnavailableObservation,
    ProviderLifecycleUnavailableReason,
};

use super::error::ApiError;
use crate::adaptor::controller::api::protocol::{
    ProviderActivityRequest, ProviderLifecycleProvider, ProviderLifecycleReceiveRequest,
    ProviderLifecycleSignalRequest, ProviderLifecycleUnavailableReasonRequest,
    ProviderLifecycleUnavailableRequest,
};
use crate::adaptor::presenter::provider_lifecycle_response::ProviderLifecycleReceiveResponse;
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

pub(super) fn ingress_context(
    provider: impl Into<ProviderKind>,
    slot_id: &str,
    agent_session_id: &str,
) -> Result<
    (
        ProviderKind,
        ProviderLifecycleSlotId,
        ProviderLifecycleScope,
    ),
    crate::domain::provider_lifecycle::ProviderLifecycleInputError,
> {
    Ok((
        provider.into(),
        ProviderLifecycleSlotId::new(slot_id)?,
        ProviderLifecycleScope::new(agent_session_id)?,
    ))
}
impl From<ProviderLifecycleProvider> for ProviderKind {
    fn from(value: ProviderLifecycleProvider) -> Self {
        match value {
            ProviderLifecycleProvider::Claude => Self::Claude,
            ProviderLifecycleProvider::Codex => Self::Codex,
        }
    }
}
impl From<crate::adaptor::presenter::client::agent_session_provider_dto::Value> for ProviderKind {
    fn from(value: crate::adaptor::presenter::client::agent_session_provider_dto::Value) -> Self {
        match value {
            crate::adaptor::presenter::client::agent_session_provider_dto::Value::Claude => {
                Self::Claude
            }
            crate::adaptor::presenter::client::agent_session_provider_dto::Value::Codex => {
                Self::Codex
            }
        }
    }
}
pub(super) fn record_ingress<E>(
    result: &Result<(ProviderLifecycleIngressResult, bool), E>,
    elapsed: std::time::Duration,
) {
    if matches!(result, Ok((ProviderLifecycleIngressResult::Applied, true))) {
        crate::infrastructure::telemetry::metrics::record_terminal_launch(
            crate::infrastructure::telemetry::metrics::TerminalLaunch::HookIngress,
            elapsed,
        );
    }
}

async fn receive(
    State(state): State<ProviderLifecycleApiState>,
    payload: Result<Json<ProviderLifecycleReceiveRequest>, JsonRejection>,
) -> Result<Json<ProviderLifecycleReceiveResponse>, ApiError> {
    crate::common::telemetry::observe_result_async(receive_inner(state, payload), record_ingress)
        .await
        .map(|(result, _)| Json(result.into()))
}

async fn receive_inner(
    state: ProviderLifecycleApiState,
    payload: Result<Json<ProviderLifecycleReceiveRequest>, JsonRejection>,
) -> Result<(ProviderLifecycleIngressResult, bool), ApiError> {
    let Json(payload) = payload.map_err(|error| ApiError::invalid_request(error.body_text()))?;
    let usecase = state
        .usecase
        .ok_or_else(ApiError::provider_lifecycle_unavailable)?;
    let (provider, slot_id, scope) = ingress_context(
        payload.provider,
        &payload.slot_id,
        &payload.agent_session_id,
    )
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
        .map_err(ApiError::from)?;
    Ok((result, is_session_started))
}

async fn report_unavailable(
    State(state): State<ProviderLifecycleApiState>,
    payload: Result<Json<ProviderLifecycleUnavailableRequest>, JsonRejection>,
) -> Result<Json<ProviderLifecycleReceiveResponse>, ApiError> {
    let Json(payload) = payload.map_err(|error| ApiError::invalid_request(error.body_text()))?;
    let usecase = state
        .usecase
        .ok_or_else(ApiError::provider_lifecycle_unavailable)?;
    let (provider, slot_id, scope) = ingress_context(
        payload.provider,
        &payload.slot_id,
        &payload.agent_session_id,
    )
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
        .map_err(ApiError::from)?;
    Ok(Json(result.into()))
}
