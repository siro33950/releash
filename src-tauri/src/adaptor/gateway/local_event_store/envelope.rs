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
use crate::domain::local_event::{LocalDomainEvent, UncommittedDomainEvent};

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
        registry.register(Arc::new(
            crate::adaptor::gateway::local_event_store::provider_lifecycle_codec::ProviderLifecycleEventCodec,
        ));
        registry.register(Arc::new(
            crate::adaptor::gateway::local_event_store::provider_hook_health_codec::ProviderHookHealthEventCodec,
        ));
        registry.register(Arc::new(
            crate::adaptor::gateway::local_event_store::provider_session_ownership_codec::ProviderSessionOwnershipEventCodec,
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
                    LocalDomainEvent::ProviderSessionOwnership(_) => {
                        "provider-session-ownership".to_string()
                    }
                    LocalDomainEvent::ProviderLifecycle(_) => "provider-lifecycle".to_string(),
                    LocalDomainEvent::ProviderHookHealth(_) => "provider-hook-health".to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;

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
