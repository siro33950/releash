//! Versioned persistence envelope and the event payload codec registry.
//!
//! The gateway registry decides persistent `event_type` / `payload_version`
//! identities; Rust type names and serde tags are never persistent identity.
//! Unknown stored types are preserved raw as `StoredUnknownEvent`; readers
//! that need the meaning fail closed with `IncompatibleStoredEvent`.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use crate::adaptor::gateway::local_event_store::canonical_cbor::{
    decode_canonical, encode_canonical, CanonicalCborError, CborValue,
};
use crate::domain::local_event::{
    ApplicationDomainEvent, ApplicationShutdownPhase, LocalDomainEvent, QuitIntent,
    UncommittedDomainEvent,
};

/// Gateway-owned stored envelope, schema version 1.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredEventEnvelopeV1 {
    pub event_id: String,
    pub commit_id: String,
    pub stream_id: String,
    pub stream_sequence: i64,
    pub global_sequence: i64,
    pub event_type: String,
    pub payload_version: i64,
    /// ASCII decimal milliseconds since the Unix epoch, no leading zeros.
    pub occurred_at: String,
    /// Canonical CBOR payload bytes.
    pub payload: Vec<u8>,
    pub payload_sha256: [u8; 32],
}

/// Raw-preserved envelope for an unknown stored event type / version.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredUnknownEvent {
    pub envelope: StoredEventEnvelopeV1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventCodecError {
    /// No codec is registered for this domain event; the batch is rejected
    /// before any write happens.
    UnregisteredEvent { description: String },
    /// The payload cannot be represented canonically.
    Encoding(CanonicalCborError),
    /// The stored payload does not decode into the registered shape.
    MalformedPayload { event_type: String },
}

impl fmt::Display for EventCodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnregisteredEvent { description } => {
                write!(f, "no payload codec registered for event {description}")
            }
            Self::Encoding(inner) => write!(f, "canonical CBOR encoding failed: {inner}"),
            Self::MalformedPayload { event_type } => {
                write!(f, "stored payload for {event_type} is malformed")
            }
        }
    }
}

impl std::error::Error for EventCodecError {}

impl From<CanonicalCborError> for EventCodecError {
    fn from(inner: CanonicalCborError) -> Self {
        Self::Encoding(inner)
    }
}

/// Encoded payload with its persistent identity, ready for the envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedEventPayload {
    pub event_type: String,
    pub payload_version: i64,
    pub payload: Vec<u8>,
}

/// A total codec for one persistent event type.
pub trait LocalEventPayloadCodec: Send + Sync {
    /// Persistent event type identity, e.g. `application.quit_accepted`.
    fn event_type(&self) -> &'static str;

    /// Current payload version written for new events.
    fn payload_version(&self) -> i64;

    /// Whether this codec owns the given domain event.
    fn handles(&self, event: &LocalDomainEvent) -> bool;

    fn encode(&self, event: &LocalDomainEvent) -> Result<CborValue, EventCodecError>;

    fn decode(
        &self,
        payload_version: i64,
        value: &CborValue,
    ) -> Result<Option<LocalDomainEvent>, EventCodecError>;
}

/// Result of decoding a stored payload.
#[derive(Debug, Clone, PartialEq)]
pub enum DecodedStoredEvent {
    Known(Box<LocalDomainEvent>),
    /// The type or version is not registered; the raw envelope is preserved.
    Unknown,
}

/// Registry deciding persistent event identities. Agent-session and workflow
/// codecs are registered by the tasks that route those events through the
/// store; this module ships the application-stream codecs.
pub struct EventCodecRegistry {
    codecs: Vec<Arc<dyn LocalEventPayloadCodec>>,
    by_type: HashMap<&'static str, Arc<dyn LocalEventPayloadCodec>>,
}

impl EventCodecRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            codecs: Vec::new(),
            by_type: HashMap::new(),
        };
        registry.register(Arc::new(ApplicationEventCodec));
        registry.register(Arc::new(
            crate::adaptor::gateway::local_event_store::agent_session_codec::AgentSessionEventCodec,
        ));
        registry.register(Arc::new(
            crate::adaptor::gateway::local_event_store::workflow_codec::WorkflowDomainEventCodec,
        ));
        registry
    }

    pub fn register(&mut self, codec: Arc<dyn LocalEventPayloadCodec>) {
        self.by_type.insert(codec.event_type(), Arc::clone(&codec));
        // A caller-provided codec may intentionally refine a built-in domain
        // event mapping (for example, a versioned compatibility codec). Keep
        // decode lookup and encode dispatch on the same last-registration-wins
        // rule.
        self.codecs.insert(0, codec);
    }

    /// Encode an uncommitted event into canonical bytes plus its identity.
    pub fn encode(&self, event: &LocalDomainEvent) -> Result<EncodedEventPayload, EventCodecError> {
        let codec = self
            .codecs
            .iter()
            .find(|codec| codec.handles(event))
            .ok_or_else(|| EventCodecError::UnregisteredEvent {
                description: match event {
                    LocalDomainEvent::AgentSession(_) => "agent-session".to_string(),
                    LocalDomainEvent::Workflow(_) => "workflow".to_string(),
                    LocalDomainEvent::Application(_) => "application".to_string(),
                },
            })?;
        let value = codec.encode(event)?;
        let payload = encode_canonical(&value)?;
        Ok(EncodedEventPayload {
            event_type: codec.event_type().to_string(),
            payload_version: codec.payload_version(),
            payload,
        })
    }

    /// Decode a stored payload; unknown types / versions are preserved raw.
    pub fn decode(
        &self,
        event_type: &str,
        payload_version: i64,
        payload: &[u8],
    ) -> Result<DecodedStoredEvent, EventCodecError> {
        let Some(codec) = self.by_type.get(event_type) else {
            return Ok(DecodedStoredEvent::Unknown);
        };
        let value = decode_canonical(payload).map_err(|_| EventCodecError::MalformedPayload {
            event_type: event_type.to_string(),
        })?;
        match codec.decode(payload_version, &value)? {
            Some(event) => Ok(DecodedStoredEvent::Known(Box::new(event))),
            None => Ok(DecodedStoredEvent::Unknown),
        }
    }
}

impl Default for EventCodecRegistry {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) fn canonical_event_batch_identity_v1(
    registry: &EventCodecRegistry,
    events: &[UncommittedDomainEvent],
) -> Result<Vec<u8>, String> {
    fn field(bytes: &mut Vec<u8>, value: &[u8]) {
        bytes.extend_from_slice(&(value.len() as u64).to_be_bytes());
        bytes.extend_from_slice(value);
    }

    let mut bytes = b"local_event_batch_identity_v1".to_vec();
    bytes.extend_from_slice(&(events.len() as u64).to_be_bytes());
    for event in events {
        let payload = registry
            .encode(&event.event)
            .map_err(|error| format!("canonical event encode failed: {error}"))?;
        field(&mut bytes, event.stream_id.as_str().as_bytes());
        field(&mut bytes, payload.event_type.as_bytes());
        bytes.extend_from_slice(&payload.payload_version.to_be_bytes());
        field(&mut bytes, &payload.payload);
        bytes.extend_from_slice(&event.occurred_at_ms.to_be_bytes());
    }
    Ok(bytes)
}

// --- Application-stream codec (owned by this module) ---

const APPLICATION_EVENT_TYPE: &str = "application.lifecycle";
const APPLICATION_PAYLOAD_VERSION: i64 = 1;

struct ApplicationEventCodec;

fn text_entry(key: &str, value: &str) -> (CborValue, CborValue) {
    (
        CborValue::Text(key.to_string()),
        CborValue::Text(value.to_string()),
    )
}

fn int_entry(key: &str, value: i64) -> (CborValue, CborValue) {
    (CborValue::Text(key.to_string()), CborValue::int(value))
}

fn shutdown_phase_label(phase: ApplicationShutdownPhase) -> &'static str {
    match phase {
        ApplicationShutdownPhase::Prepared => "prepared",
        ApplicationShutdownPhase::Activated => "activated",
        ApplicationShutdownPhase::Quiescing => "quiescing",
        ApplicationShutdownPhase::Completed => "completed",
        ApplicationShutdownPhase::Failed => "failed",
        ApplicationShutdownPhase::Cancelled => "cancelled",
        ApplicationShutdownPhase::ReconciliationRequired => "reconciliation_required",
    }
}

fn parse_shutdown_phase(raw: &str) -> Option<ApplicationShutdownPhase> {
    match raw {
        "prepared" => Some(ApplicationShutdownPhase::Prepared),
        "activated" => Some(ApplicationShutdownPhase::Activated),
        "quiescing" => Some(ApplicationShutdownPhase::Quiescing),
        "completed" => Some(ApplicationShutdownPhase::Completed),
        "failed" => Some(ApplicationShutdownPhase::Failed),
        "cancelled" => Some(ApplicationShutdownPhase::Cancelled),
        "reconciliation_required" => Some(ApplicationShutdownPhase::ReconciliationRequired),
        _ => None,
    }
}

pub(crate) fn shutdown_phase_to_label(phase: ApplicationShutdownPhase) -> &'static str {
    shutdown_phase_label(phase)
}

pub(crate) fn label_to_shutdown_phase(raw: &str) -> Option<ApplicationShutdownPhase> {
    parse_shutdown_phase(raw)
}

fn map_get<'a>(entries: &'a [(CborValue, CborValue)], key: &str) -> Option<&'a CborValue> {
    entries
        .iter()
        .find_map(|(entry_key, value)| match entry_key {
            CborValue::Text(text) if text == key => Some(value),
            _ => None,
        })
}

fn map_text(entries: &[(CborValue, CborValue)], key: &str) -> Option<String> {
    match map_get(entries, key)? {
        CborValue::Text(text) => Some(text.clone()),
        _ => None,
    }
}

fn map_i64(entries: &[(CborValue, CborValue)], key: &str) -> Option<i64> {
    map_get(entries, key)?.as_i64()
}

impl LocalEventPayloadCodec for ApplicationEventCodec {
    fn event_type(&self) -> &'static str {
        APPLICATION_EVENT_TYPE
    }

    fn payload_version(&self) -> i64 {
        APPLICATION_PAYLOAD_VERSION
    }

    fn handles(&self, event: &LocalDomainEvent) -> bool {
        matches!(event, LocalDomainEvent::Application(_))
    }

    fn encode(&self, event: &LocalDomainEvent) -> Result<CborValue, EventCodecError> {
        let LocalDomainEvent::Application(event) = event else {
            return Err(EventCodecError::UnregisteredEvent {
                description: "non-application event given to application codec".to_string(),
            });
        };
        let entries = match event {
            ApplicationDomainEvent::ApplicationQuitAccepted {
                quit_operation_id,
                intent,
                at_ms,
            } => {
                let mut entries = vec![
                    text_entry("kind", "application_quit_accepted"),
                    text_entry("quit_operation_id", quit_operation_id),
                    int_entry("at_ms", *at_ms),
                ];
                match intent {
                    QuitIntent::Exit { code } => {
                        entries.push(text_entry("intent", "exit"));
                        entries.push(int_entry("exit_code", *code));
                    }
                    QuitIntent::Restart { code } => {
                        entries.push(text_entry("intent", "restart"));
                        entries.push(int_entry("exit_code", *code));
                    }
                }
                entries
            }
            ApplicationDomainEvent::ShutdownPhaseAdvanced {
                shutdown_id,
                phase,
                at_ms,
            } => vec![
                text_entry("kind", "shutdown_phase_advanced"),
                text_entry("shutdown_id", shutdown_id),
                text_entry("phase", shutdown_phase_label(*phase)),
                int_entry("at_ms", *at_ms),
            ],
            ApplicationDomainEvent::ShutdownDetailsCompacted { shutdown_id, at_ms } => vec![
                text_entry("kind", "shutdown_details_compacted"),
                text_entry("shutdown_id", shutdown_id),
                int_entry("at_ms", *at_ms),
            ],
        };
        Ok(CborValue::Map(entries))
    }

    fn decode(
        &self,
        payload_version: i64,
        value: &CborValue,
    ) -> Result<Option<LocalDomainEvent>, EventCodecError> {
        if payload_version != APPLICATION_PAYLOAD_VERSION {
            return Ok(None);
        }
        let malformed = || EventCodecError::MalformedPayload {
            event_type: APPLICATION_EVENT_TYPE.to_string(),
        };
        let CborValue::Map(entries) = value else {
            return Err(malformed());
        };
        let kind = map_text(entries, "kind").ok_or_else(malformed)?;
        let at_ms = map_i64(entries, "at_ms").ok_or_else(malformed)?;
        let event = match kind.as_str() {
            "application_quit_accepted" => {
                let intent = match map_text(entries, "intent").ok_or_else(malformed)?.as_str() {
                    "exit" => QuitIntent::Exit {
                        code: map_i64(entries, "exit_code").ok_or_else(malformed)?,
                    },
                    "restart" => QuitIntent::Restart {
                        code: map_i64(entries, "exit_code").ok_or_else(malformed)?,
                    },
                    _ => return Err(malformed()),
                };
                ApplicationDomainEvent::ApplicationQuitAccepted {
                    quit_operation_id: map_text(entries, "quit_operation_id")
                        .ok_or_else(malformed)?,
                    intent,
                    at_ms,
                }
            }
            "shutdown_phase_advanced" => ApplicationDomainEvent::ShutdownPhaseAdvanced {
                // Schema v1 events are immutable commit evidence. During the
                // supported v1 -> v2 schema step the aggregate identity is
                // unchanged (`plan_id == operation_id`), so decoding accepts
                // that prior field without retaining a second identity.
                shutdown_id: map_text(entries, "shutdown_id")
                    .or_else(|| map_text(entries, "plan_id"))
                    .ok_or_else(malformed)?,
                phase: parse_shutdown_phase(&map_text(entries, "phase").ok_or_else(malformed)?)
                    .ok_or_else(malformed)?,
                at_ms,
            },
            "shutdown_details_compacted" => ApplicationDomainEvent::ShutdownDetailsCompacted {
                shutdown_id: map_text(entries, "shutdown_id")
                    .or_else(|| map_text(entries, "plan_id"))
                    .ok_or_else(malformed)?,
                at_ms,
            },
            _ => return Ok(None),
        };
        Ok(Some(LocalDomainEvent::Application(event)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn application_events_round_trip_canonically() {
        let registry = EventCodecRegistry::new();
        let events = vec![
            ApplicationDomainEvent::ApplicationQuitAccepted {
                quit_operation_id: "quit-1".to_string(),
                intent: QuitIntent::Exit { code: 0 },
                at_ms: 1_700_000_000_000,
            },
            ApplicationDomainEvent::ApplicationQuitAccepted {
                quit_operation_id: "restart-1".to_string(),
                intent: QuitIntent::Restart { code: i64::MIN },
                at_ms: 1_700_000_000_001,
            },
            ApplicationDomainEvent::ShutdownPhaseAdvanced {
                shutdown_id: "plan-1".to_string(),
                phase: ApplicationShutdownPhase::Activated,
                at_ms: 5,
            },
            ApplicationDomainEvent::ShutdownDetailsCompacted {
                shutdown_id: "plan-1".to_string(),
                at_ms: 6,
            },
        ];
        for event in events {
            let domain = LocalDomainEvent::Application(event);
            let encoded = registry.encode(&domain).unwrap();
            assert_eq!(encoded.event_type, "application.lifecycle");
            let decoded = registry
                .decode(
                    &encoded.event_type,
                    encoded.payload_version,
                    &encoded.payload,
                )
                .unwrap();
            assert_eq!(decoded, DecodedStoredEvent::Known(Box::new(domain)));
        }
    }

    #[test]
    fn unknown_type_and_version_are_preserved_raw() {
        let registry = EventCodecRegistry::new();
        let payload = encode_canonical(&CborValue::Map(vec![])).unwrap();
        assert_eq!(
            registry.decode("future.event", 1, &payload).unwrap(),
            DecodedStoredEvent::Unknown
        );
        assert_eq!(
            registry
                .decode("application.lifecycle", 999, &payload)
                .unwrap(),
            DecodedStoredEvent::Unknown
        );
    }
}
