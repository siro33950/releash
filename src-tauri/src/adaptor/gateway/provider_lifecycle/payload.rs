use serde::Deserialize;
use thiserror::Error;

use crate::domain::provider_lifecycle::{
    ProviderKind, ProviderLifecycleInputError, ProviderLifecycleScope, ProviderLifecycleSignal,
};

#[derive(Debug, Error)]
pub(crate) enum ProviderLifecycleGatewayError {
    #[error("Provider lifecycle payload is invalid")]
    InvalidPayload,
    #[error("unsupported Provider lifecycle event: {0}")]
    UnsupportedEvent(String),
    #[error(transparent)]
    InvalidSignal(#[from] ProviderLifecycleInputError),
}

pub(crate) fn parse_provider_payload(
    provider: ProviderKind,
    binding_id: &str,
    scope: ProviderLifecycleScope,
    payload: &[u8],
) -> Result<ProviderLifecycleSignal, ProviderLifecycleGatewayError> {
    let payload = serde_json::from_slice::<ProviderPayload>(payload)
        .map_err(|_| ProviderLifecycleGatewayError::InvalidPayload)?;
    let transcript_ref = payload.transcript_path.as_deref();

    match (provider, payload.hook_event_name.as_str()) {
        (ProviderKind::Claude | ProviderKind::Codex, "SessionStart") => {
            ProviderLifecycleSignal::session_started(
                binding_id,
                provider,
                scope,
                payload.session_id,
                transcript_ref,
            )
            .map_err(Into::into)
        }
        (ProviderKind::Claude | ProviderKind::Codex, "Stop") => {
            ProviderLifecycleSignal::stop_observed(
                binding_id,
                provider,
                scope,
                payload.session_id,
                transcript_ref,
            )
            .map_err(Into::into)
        }
        (ProviderKind::Claude, "StopFailure") => {
            let error = payload
                .error
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .ok_or(ProviderLifecycleGatewayError::InvalidPayload)?;
            let reason = match payload
                .error_details
                .as_deref()
                .filter(|value| !value.trim().is_empty())
            {
                Some(details) => format!("{error}: {details}"),
                None => error.to_string(),
            };
            ProviderLifecycleSignal::stop_failed(
                binding_id,
                provider,
                scope,
                payload.session_id,
                transcript_ref,
                reason,
            )
            .map_err(Into::into)
        }
        (_, event) => Err(ProviderLifecycleGatewayError::UnsupportedEvent(
            event.to_string(),
        )),
    }
}

#[derive(Debug, Deserialize)]
struct ProviderPayload {
    session_id: String,
    #[serde(default)]
    transcript_path: Option<String>,
    hook_event_name: String,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_details: Option<String>,
}
