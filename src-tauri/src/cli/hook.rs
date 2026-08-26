use std::io::Read;

use super::api_client::{request_classified, ApiRequestError};
use super::common::{cli_error_stderr, resolve_data_dir, CliError};
use super::HookProvider;
use crate::adaptor::gateway::provider_lifecycle::{
    parse_provider_payload, ProviderLifecycleGatewayError,
};
use crate::adaptor::protocol::provider_lifecycle::{
    ProviderLifecycleProvider, ProviderLifecycleReceiveRequest, ProviderLifecycleReceiveResponse,
    ProviderLifecycleSignalRequest,
};
use crate::domain::provider_lifecycle::{
    ProviderKind, ProviderLifecycleScope, ProviderLifecycleSignalKind,
};
use crate::infrastructure::provider_lifecycle::{read_bounded, BoundedReadError};

const MAX_PAYLOAD_BYTES: usize = 65_536;
const SLOT_ID_ENV: &str = "RELEASH_PROVIDER_LIFECYCLE_SLOT_ID";
const BINDING_ID_ENV: &str = "RELEASH_PROVIDER_LIFECYCLE_BINDING_ID";
const CAPABILITY_ENV: &str = "RELEASH_PROVIDER_LIFECYCLE_CAPABILITY";
const AGENT_SESSION_ID_ENV: &str = "RELEASH_PROVIDER_LIFECYCLE_AGENT_SESSION_ID";
const HEALTH_FILE_ENV: &str = "RELEASH_PROVIDER_LIFECYCLE_HEALTH_FILE";

pub(super) fn cmd_receive(provider: HookProvider) -> Result<String, CliError> {
    if let Err(error) = receive_from(std::io::stdin().lock(), provider) {
        eprintln!("{}", cli_error_stderr(&error));
    }
    Ok("{}".to_string())
}

fn receive_from(_reader: impl Read, provider: HookProvider) -> Result<String, CliError> {
    let payload = read_bounded(_reader, MAX_PAYLOAD_BYTES).map_err(|error| match error {
        BoundedReadError::LimitExceeded { limit } => CliError::InvalidInput(format!(
            "Provider lifecycle payload exceeds the {limit} byte limit"
        )),
        BoundedReadError::Read(error) => CliError::Other(format!(
            "Provider lifecycle payload could not be read: {error}"
        )),
    })?;
    let slot_id = required_environment(SLOT_ID_ENV)?;
    let binding_id = required_environment(BINDING_ID_ENV)?;
    let capability = required_environment(CAPABILITY_ENV)?;
    let agent_session_id = required_environment(AGENT_SESSION_ID_ENV)?;
    let provider_kind = match provider {
        HookProvider::Claude => ProviderKind::Claude,
        HookProvider::Codex => ProviderKind::Codex,
    };
    let protocol_provider = match provider {
        HookProvider::Claude => ProviderLifecycleProvider::Claude,
        HookProvider::Codex => ProviderLifecycleProvider::Codex,
    };
    let scope = ProviderLifecycleScope::new(&agent_session_id)
        .map_err(|error| CliError::InvalidInput(error.to_string()))?;
    let signal = match parse_provider_payload(provider_kind, &binding_id, scope, &payload) {
        Ok(signal) => signal,
        Err(ProviderLifecycleGatewayError::SubagentPayload) => return Ok("{}".to_string()),
        Err(error) => return Err(CliError::InvalidInput(error.to_string())),
    };
    let signal = signal.into_kind();
    let session_started = matches!(&signal, ProviderLifecycleSignalKind::SessionStarted { .. });
    let signal = match signal {
        ProviderLifecycleSignalKind::SessionStarted {
            provider_session_id,
            transcript_ref,
        } => ProviderLifecycleSignalRequest::SessionStarted {
            provider_session_id,
            transcript_ref,
        },
        ProviderLifecycleSignalKind::StopObserved {
            provider_session_id,
            transcript_ref,
        } => ProviderLifecycleSignalRequest::StopObserved {
            provider_session_id,
            transcript_ref,
        },
        ProviderLifecycleSignalKind::StopFailed {
            provider_session_id,
            transcript_ref,
            reason,
        } => ProviderLifecycleSignalRequest::StopFailed {
            provider_session_id,
            transcript_ref,
            reason,
        },
    };
    let request = ProviderLifecycleReceiveRequest {
        slot_id,
        binding_id,
        capability,
        provider: protocol_provider,
        agent_session_id,
        signal,
    };
    let data_dir = resolve_data_dir().map_err(CliError::Other)?;
    let response = request_classified(&data_dir, |client| {
        client.receive_provider_lifecycle(&request)
    })
    .map_err(|error| {
        record_delivery_failure(&data_dir, provider, &request.slot_id);
        match error {
            ApiRequestError::Unavailable => {
                CliError::Other("この操作には Releash アプリの起動が必要です".to_string())
            }
            ApiRequestError::Cli(error) => error,
        }
    })?;
    match response {
        ProviderLifecycleReceiveResponse::Applied | ProviderLifecycleReceiveResponse::Duplicate => {
            if session_started {
                if let Ok(marker_path) = std::env::var(HEALTH_FILE_ENV) {
                    if let Err(error) = crate::infrastructure::provider_lifecycle::clear_provider_hook_local_api_failure(
                        &data_dir,
                        std::path::Path::new(&marker_path),
                    ) {
                        log::warn!("failed to clear Provider Hook delivery health: {error}");
                    }
                }
            }
            Ok("{}".to_string())
        }
        ProviderLifecycleReceiveResponse::Rejected { reason } => Err(CliError::Other(format!(
            "Provider lifecycle signal was rejected: {reason}"
        ))),
    }
}

fn record_delivery_failure(data_dir: &std::path::Path, provider: HookProvider, launch_id: &str) {
    let Ok(marker_path) = std::env::var(HEALTH_FILE_ENV) else {
        return;
    };
    let provider = match provider {
        HookProvider::Claude => "claude",
        HookProvider::Codex => "codex",
    };
    if let Err(error) =
        crate::infrastructure::provider_lifecycle::write_provider_hook_local_api_failure(
            data_dir,
            std::path::Path::new(&marker_path),
            provider,
            launch_id,
        )
    {
        log::warn!("failed to persist Provider Hook delivery health: {error}");
    }
}

fn required_environment(name: &'static str) -> Result<String, CliError> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| CliError::InvalidInput(format!("{name} is required")))
}

#[cfg(test)]
#[path = "hook_test.rs"]
mod hook_tests;
