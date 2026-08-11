use serde::{Deserialize, Serialize};

use crate::adaptor::gateway::local_event_store::canonical_cbor::CborValue;
use crate::adaptor::gateway::local_event_store::envelope::{
    EventCodecError, LocalEventPayloadCodec,
};
use crate::domain::agent_session::ProviderSessionOwnershipEvent;
use crate::domain::local_event::LocalDomainEvent;
use crate::domain::provider_lifecycle::ProviderKind;

pub(crate) const PROVIDER_SESSION_OWNERSHIP_EVENT_TYPE: &str =
    "agent_session.provider_session_ownership";
pub(crate) const PROVIDER_SESSION_OWNERSHIP_PAYLOAD_VERSION: i64 = 1;

pub(crate) struct ProviderSessionOwnershipEventCodec;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
enum StoredProviderSessionOwnershipEventV1 {
    Claimed {
        provider: String,
        provider_session_id: String,
        agent_session_id: String,
    },
    Released {
        provider: String,
        provider_session_id: String,
        agent_session_id: String,
    },
}

fn malformed() -> EventCodecError {
    EventCodecError::MalformedPayload {
        event_type: PROVIDER_SESSION_OWNERSHIP_EVENT_TYPE.to_string(),
    }
}

fn provider_label(provider: ProviderKind) -> &'static str {
    match provider {
        ProviderKind::Claude => "claude",
        ProviderKind::Codex => "codex",
    }
}

fn parse_provider(provider: &str) -> Result<ProviderKind, EventCodecError> {
    match provider {
        "claude" => Ok(ProviderKind::Claude),
        "codex" => Ok(ProviderKind::Codex),
        _ => Err(malformed()),
    }
}

fn stored_event(event: &ProviderSessionOwnershipEvent) -> StoredProviderSessionOwnershipEventV1 {
    match event {
        ProviderSessionOwnershipEvent::Claimed {
            provider,
            provider_session_id,
            agent_session_id,
        } => StoredProviderSessionOwnershipEventV1::Claimed {
            provider: provider_label(*provider).to_string(),
            provider_session_id: provider_session_id.clone(),
            agent_session_id: agent_session_id.clone(),
        },
        ProviderSessionOwnershipEvent::Released {
            provider,
            provider_session_id,
            agent_session_id,
        } => StoredProviderSessionOwnershipEventV1::Released {
            provider: provider_label(*provider).to_string(),
            provider_session_id: provider_session_id.clone(),
            agent_session_id: agent_session_id.clone(),
        },
    }
}

fn domain_event(
    event: StoredProviderSessionOwnershipEventV1,
) -> Result<ProviderSessionOwnershipEvent, EventCodecError> {
    let (claimed, provider, provider_session_id, agent_session_id) = match event {
        StoredProviderSessionOwnershipEventV1::Claimed {
            provider,
            provider_session_id,
            agent_session_id,
        } => (true, provider, provider_session_id, agent_session_id),
        StoredProviderSessionOwnershipEventV1::Released {
            provider,
            provider_session_id,
            agent_session_id,
        } => (false, provider, provider_session_id, agent_session_id),
    };
    if provider_session_id.trim().is_empty() || agent_session_id.trim().is_empty() {
        return Err(malformed());
    }
    let provider = parse_provider(&provider)?;
    Ok(if claimed {
        ProviderSessionOwnershipEvent::Claimed {
            provider,
            provider_session_id,
            agent_session_id,
        }
    } else {
        ProviderSessionOwnershipEvent::Released {
            provider,
            provider_session_id,
            agent_session_id,
        }
    })
}

impl LocalEventPayloadCodec for ProviderSessionOwnershipEventCodec {
    fn event_type(&self) -> &'static str {
        PROVIDER_SESSION_OWNERSHIP_EVENT_TYPE
    }

    fn payload_version(&self) -> i64 {
        PROVIDER_SESSION_OWNERSHIP_PAYLOAD_VERSION
    }

    fn handles(&self, event: &LocalDomainEvent) -> bool {
        matches!(event, LocalDomainEvent::ProviderSessionOwnership(_))
    }

    fn encode(&self, event: &LocalDomainEvent) -> Result<CborValue, EventCodecError> {
        let LocalDomainEvent::ProviderSessionOwnership(event) = event else {
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
        if payload_version != PROVIDER_SESSION_OWNERSHIP_PAYLOAD_VERSION {
            return Ok(None);
        }
        let CborValue::Text(raw) = value else {
            return Err(malformed());
        };
        let stored = serde_json::from_str(raw).map_err(|_| malformed())?;
        domain_event(stored).map(|event| Some(LocalDomainEvent::ProviderSessionOwnership(event)))
    }
}

#[cfg(test)]
#[path = "provider_session_ownership_codec_test.rs"]
mod provider_session_ownership_codec_tests;
