use serde::{Deserialize, Serialize};

use crate::adaptor::gateway::local_event_store::canonical_cbor::CborValue;
use crate::adaptor::gateway::local_event_store::envelope::{
    EventCodecError, LocalEventPayloadCodec,
};
use crate::domain::local_event::LocalDomainEvent;
use crate::domain::provider_lifecycle::{
    ProviderHookHealthEvent, ProviderKind, ProviderLifecycleUnavailableReason,
};

pub(crate) const PROVIDER_HOOK_HEALTH_EVENT_TYPE: &str = "provider.hook_health";
pub(crate) const PROVIDER_HOOK_HEALTH_PAYLOAD_VERSION: i64 = 1;

pub(crate) struct ProviderHookHealthEventCodec;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
enum StoredProviderHookHealthEventV1 {
    LaunchObserved {
        provider: String,
        launch_id: String,
    },
    WarningRecorded {
        provider: String,
        launch_id: String,
        reason: String,
    },
    SessionStartedObserved {
        provider: String,
        launch_id: String,
    },
}

fn malformed() -> EventCodecError {
    EventCodecError::MalformedPayload {
        event_type: PROVIDER_HOOK_HEALTH_EVENT_TYPE.to_string(),
    }
}

pub(crate) fn provider_label(provider: ProviderKind) -> &'static str {
    match provider {
        ProviderKind::Claude => "claude",
        ProviderKind::Codex => "codex",
    }
}

pub(crate) fn parse_provider(provider: &str) -> Result<ProviderKind, EventCodecError> {
    match provider {
        "claude" => Ok(ProviderKind::Claude),
        "codex" => Ok(ProviderKind::Codex),
        _ => Err(malformed()),
    }
}

pub(crate) fn warning_reason_label(reason: ProviderLifecycleUnavailableReason) -> &'static str {
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

pub(crate) fn parse_warning_reason(
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

fn stored_event(event: &ProviderHookHealthEvent) -> StoredProviderHookHealthEventV1 {
    match event {
        ProviderHookHealthEvent::LaunchObserved {
            provider,
            launch_id,
        } => StoredProviderHookHealthEventV1::LaunchObserved {
            provider: provider_label(*provider).to_string(),
            launch_id: launch_id.clone(),
        },
        ProviderHookHealthEvent::WarningRecorded {
            provider,
            launch_id,
            reason,
        } => StoredProviderHookHealthEventV1::WarningRecorded {
            provider: provider_label(*provider).to_string(),
            launch_id: launch_id.clone(),
            reason: warning_reason_label(*reason).to_string(),
        },
        ProviderHookHealthEvent::SessionStartedObserved {
            provider,
            launch_id,
        } => StoredProviderHookHealthEventV1::SessionStartedObserved {
            provider: provider_label(*provider).to_string(),
            launch_id: launch_id.clone(),
        },
    }
}

fn domain_event(
    event: StoredProviderHookHealthEventV1,
) -> Result<ProviderHookHealthEvent, EventCodecError> {
    Ok(match event {
        StoredProviderHookHealthEventV1::LaunchObserved {
            provider,
            launch_id,
        } if !launch_id.trim().is_empty() => ProviderHookHealthEvent::LaunchObserved {
            provider: parse_provider(&provider)?,
            launch_id,
        },
        StoredProviderHookHealthEventV1::WarningRecorded {
            provider,
            launch_id,
            reason,
        } if !launch_id.trim().is_empty() => ProviderHookHealthEvent::WarningRecorded {
            provider: parse_provider(&provider)?,
            launch_id,
            reason: parse_warning_reason(&reason)?,
        },
        StoredProviderHookHealthEventV1::SessionStartedObserved {
            provider,
            launch_id,
        } if !launch_id.trim().is_empty() => ProviderHookHealthEvent::SessionStartedObserved {
            provider: parse_provider(&provider)?,
            launch_id,
        },
        _ => return Err(malformed()),
    })
}

impl LocalEventPayloadCodec for ProviderHookHealthEventCodec {
    fn event_type(&self) -> &'static str {
        PROVIDER_HOOK_HEALTH_EVENT_TYPE
    }

    fn payload_version(&self) -> i64 {
        PROVIDER_HOOK_HEALTH_PAYLOAD_VERSION
    }

    fn handles(&self, event: &LocalDomainEvent) -> bool {
        matches!(event, LocalDomainEvent::ProviderHookHealth(_))
    }

    fn encode(&self, event: &LocalDomainEvent) -> Result<CborValue, EventCodecError> {
        let LocalDomainEvent::ProviderHookHealth(event) = event else {
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
        if payload_version != PROVIDER_HOOK_HEALTH_PAYLOAD_VERSION {
            return Ok(None);
        }
        let CborValue::Text(raw) = value else {
            return Err(malformed());
        };
        let stored = serde_json::from_str(raw).map_err(|_| malformed())?;
        domain_event(stored).map(|event| Some(LocalDomainEvent::ProviderHookHealth(event)))
    }
}
