use std::io::Read;

use super::api_client::mutation;
use super::common::{cli_error_stderr, resolve_data_dir, CliError};
use super::HookProvider;
use crate::adaptor::gateway::provider_lifecycle::parse_provider_payload;
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
const WORKFLOW_EXECUTION_ID_ENV: &str = "RELEASH_PROVIDER_LIFECYCLE_WORKFLOW_EXECUTION_ID";
const NODE_EXECUTION_ID_ENV: &str = "RELEASH_PROVIDER_LIFECYCLE_NODE_EXECUTION_ID";
const ATTEMPT_ENV: &str = "RELEASH_PROVIDER_LIFECYCLE_ATTEMPT";

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
    let workflow_execution_id = required_environment(WORKFLOW_EXECUTION_ID_ENV)?;
    let node_execution_id = required_environment(NODE_EXECUTION_ID_ENV)?;
    let attempt = required_environment(ATTEMPT_ENV)?
        .parse::<u32>()
        .map_err(|_| {
            CliError::InvalidInput(format!("{ATTEMPT_ENV} must be an unsigned integer"))
        })?;
    let provider_kind = match provider {
        HookProvider::Claude => ProviderKind::Claude,
        HookProvider::Codex => ProviderKind::Codex,
    };
    let protocol_provider = match provider {
        HookProvider::Claude => ProviderLifecycleProvider::Claude,
        HookProvider::Codex => ProviderLifecycleProvider::Codex,
    };
    let scope = ProviderLifecycleScope::new(
        &agent_session_id,
        &workflow_execution_id,
        &node_execution_id,
        attempt,
    )
    .map_err(|error| CliError::InvalidInput(error.to_string()))?;
    let signal = parse_provider_payload(provider_kind, &binding_id, scope, &payload)
        .map_err(|error| CliError::InvalidInput(error.to_string()))?;
    let signal = match signal.into_kind() {
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
        workflow_execution_id,
        node_execution_id,
        attempt,
        signal,
    };
    let data_dir = resolve_data_dir().map_err(CliError::Other)?;
    let response = mutation(&data_dir, |client| {
        client.receive_provider_lifecycle(&request)
    })?;
    match response {
        ProviderLifecycleReceiveResponse::Applied | ProviderLifecycleReceiveResponse::Duplicate => {
            Ok("{}".to_string())
        }
        ProviderLifecycleReceiveResponse::Rejected { reason } => Err(CliError::Other(format!(
            "Provider lifecycle signal was rejected: {reason}"
        ))),
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
