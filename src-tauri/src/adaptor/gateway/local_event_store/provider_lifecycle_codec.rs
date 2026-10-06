use serde::{Deserialize, Serialize};

use crate::adaptor::gateway::local_event_store::canonical_cbor::CborValue;
use crate::adaptor::gateway::local_event_store::envelope::{
    EventCodecError, LocalEventPayloadCodec,
};
use crate::domain::local_event::LocalDomainEvent;
use crate::domain::provider_lifecycle::{
    ProviderKind, ProviderLifecycleEvent, ProviderLifecycleScope,
    ProviderLifecycleUnavailableReason,
};

pub(crate) const PROVIDER_LIFECYCLE_EVENT_TYPE: &str = "provider.lifecycle";
pub(crate) const PROVIDER_LIFECYCLE_PAYLOAD_VERSION: i64 = 1;

pub(crate) struct ProviderLifecycleEventCodec;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
enum StoredProviderLifecycleEventV1 {
    BindingArmed {
        slot_id: String,
        binding_id: String,
        provider: String,
        agent_session_id: String,
    },
    SessionAssociated {
        binding_id: String,
        provider_session_id: String,
        transcript_ref: Option<String>,
    },
    TranscriptAssociated {
        binding_id: String,
        transcript_ref: String,
    },
    StopObserved {
        binding_id: String,
    },
    StopFailed {
        binding_id: String,
        reason: String,
    },
    LifecycleUnavailable {
        binding_id: String,
        provider: String,
        agent_session_id: String,
        reason: String,
    },
    BindingExpired {
        binding_id: String,
    },
}

fn malformed() -> EventCodecError {
    EventCodecError::MalformedPayload {
        event_type: PROVIDER_LIFECYCLE_EVENT_TYPE.to_string(),
    }
}

fn stored_provider(provider: ProviderKind) -> &'static str {
    match provider {
        ProviderKind::Claude => "claude",
        ProviderKind::Codex => "codex",
    }
}

fn domain_provider(provider: &str) -> Result<ProviderKind, EventCodecError> {
    match provider {
        "claude" => Ok(ProviderKind::Claude),
        "codex" => Ok(ProviderKind::Codex),
        _ => Err(malformed()),
    }
}

fn stored_unavailable_reason(reason: ProviderLifecycleUnavailableReason) -> &'static str {
    match reason {
        ProviderLifecycleUnavailableReason::SessionStartDeadlineExceeded => {
            "session_start_deadline_exceeded"
        }
        ProviderLifecycleUnavailableReason::CodexHookDeliveryUnconfirmed => {
            "codex_hook_delivery_unconfirmed"
        }
        ProviderLifecycleUnavailableReason::ProviderHookConfigurationRejected => {
            "provider_hook_configuration_rejected"
        }
        ProviderLifecycleUnavailableReason::LocalApiUnavailable => "local_api_unavailable",
    }
}

fn domain_unavailable_reason(
    reason: &str,
) -> Result<ProviderLifecycleUnavailableReason, EventCodecError> {
    match reason {
        "session_start_deadline_exceeded" => {
            Ok(ProviderLifecycleUnavailableReason::SessionStartDeadlineExceeded)
        }
        "codex_hook_delivery_unconfirmed" => {
            Ok(ProviderLifecycleUnavailableReason::CodexHookDeliveryUnconfirmed)
        }
        "provider_hook_configuration_rejected" => {
            Ok(ProviderLifecycleUnavailableReason::ProviderHookConfigurationRejected)
        }
        "local_api_unavailable" => Ok(ProviderLifecycleUnavailableReason::LocalApiUnavailable),
        _ => Err(malformed()),
    }
}

fn stored_event(event: &ProviderLifecycleEvent) -> StoredProviderLifecycleEventV1 {
    match event {
        ProviderLifecycleEvent::BindingArmed {
            slot_id,
            binding_id,
            provider,
            scope,
        } => StoredProviderLifecycleEventV1::BindingArmed {
            slot_id: slot_id.clone(),
            binding_id: binding_id.clone(),
            provider: stored_provider(*provider).to_string(),
            agent_session_id: scope.agent_session_id().to_string(),
        },
        ProviderLifecycleEvent::SessionAssociated {
            binding_id,
            provider_session_id,
            transcript_ref,
        } => StoredProviderLifecycleEventV1::SessionAssociated {
            binding_id: binding_id.clone(),
            provider_session_id: provider_session_id.clone(),
            transcript_ref: transcript_ref.clone(),
        },
        ProviderLifecycleEvent::TranscriptAssociated {
            binding_id,
            transcript_ref,
        } => StoredProviderLifecycleEventV1::TranscriptAssociated {
            binding_id: binding_id.clone(),
            transcript_ref: transcript_ref.clone(),
        },
        ProviderLifecycleEvent::StopObserved { binding_id } => {
            StoredProviderLifecycleEventV1::StopObserved {
                binding_id: binding_id.clone(),
            }
        }
        ProviderLifecycleEvent::StopFailed { binding_id, reason } => {
            StoredProviderLifecycleEventV1::StopFailed {
                binding_id: binding_id.clone(),
                reason: reason.clone(),
            }
        }
        ProviderLifecycleEvent::LifecycleUnavailable {
            binding_id,
            provider,
            scope,
            reason,
        } => StoredProviderLifecycleEventV1::LifecycleUnavailable {
            binding_id: binding_id.clone(),
            provider: stored_provider(*provider).to_string(),
            agent_session_id: scope.agent_session_id().to_string(),
            reason: stored_unavailable_reason(*reason).to_string(),
        },
        ProviderLifecycleEvent::BindingExpired { binding_id } => {
            StoredProviderLifecycleEventV1::BindingExpired {
                binding_id: binding_id.clone(),
            }
        }
    }
}

fn domain_event(
    event: StoredProviderLifecycleEventV1,
) -> Result<ProviderLifecycleEvent, EventCodecError> {
    Ok(match event {
        StoredProviderLifecycleEventV1::BindingArmed {
            slot_id,
            binding_id,
            provider,
            agent_session_id,
        } => ProviderLifecycleEvent::binding_armed(
            slot_id,
            binding_id,
            domain_provider(&provider)?,
            ProviderLifecycleScope::new(agent_session_id).map_err(|_| malformed())?,
        )
        .map_err(|_| malformed())?,
        StoredProviderLifecycleEventV1::SessionAssociated {
            binding_id,
            provider_session_id,
            transcript_ref,
        } => ProviderLifecycleEvent::session_associated(
            binding_id,
            provider_session_id,
            transcript_ref,
        )
        .map_err(|_| malformed())?,
        StoredProviderLifecycleEventV1::TranscriptAssociated {
            binding_id,
            transcript_ref,
        } => ProviderLifecycleEvent::transcript_associated(binding_id, transcript_ref)
            .map_err(|_| malformed())?,
        StoredProviderLifecycleEventV1::StopObserved { binding_id } => {
            ProviderLifecycleEvent::stop_observed(binding_id).map_err(|_| malformed())?
        }
        StoredProviderLifecycleEventV1::StopFailed { binding_id, reason } => {
            ProviderLifecycleEvent::stop_failed(binding_id, reason).map_err(|_| malformed())?
        }
        StoredProviderLifecycleEventV1::LifecycleUnavailable {
            binding_id,
            provider,
            agent_session_id,
            reason,
        } => ProviderLifecycleEvent::lifecycle_unavailable(
            binding_id,
            domain_provider(&provider)?,
            ProviderLifecycleScope::new(agent_session_id).map_err(|_| malformed())?,
            domain_unavailable_reason(&reason)?,
        )
        .map_err(|_| malformed())?,
        StoredProviderLifecycleEventV1::BindingExpired { binding_id } => {
            ProviderLifecycleEvent::binding_expired(binding_id).map_err(|_| malformed())?
        }
    })
}

impl LocalEventPayloadCodec for ProviderLifecycleEventCodec {
    fn event_type(&self) -> &'static str {
        PROVIDER_LIFECYCLE_EVENT_TYPE
    }

    fn payload_version(&self) -> i64 {
        PROVIDER_LIFECYCLE_PAYLOAD_VERSION
    }

    fn handles(&self, event: &LocalDomainEvent) -> bool {
        matches!(event, LocalDomainEvent::ProviderLifecycle(_))
    }

    fn encode(&self, event: &LocalDomainEvent) -> Result<CborValue, EventCodecError> {
        let LocalDomainEvent::ProviderLifecycle(event) = event else {
            return Err(malformed());
        };
        serde_json::to_string(&stored_event(event))
            .map(CborValue::Text)
            .map_err(|_| malformed())
    }

    fn decode(
        &self,
        payload_version: i64,
        value: &CborValue,
    ) -> Result<Option<LocalDomainEvent>, EventCodecError> {
        if payload_version != PROVIDER_LIFECYCLE_PAYLOAD_VERSION {
            return Ok(None);
        }
        let CborValue::Text(raw) = value else {
            return Err(malformed());
        };
        let stored = serde_json::from_str(raw).map_err(|_| malformed())?;
        domain_event(stored).map(|event| Some(LocalDomainEvent::ProviderLifecycle(event)))
    }
}

#[cfg(test)]
#[path = "provider_lifecycle_codec_test.rs"]
mod provider_lifecycle_codec_tests;
