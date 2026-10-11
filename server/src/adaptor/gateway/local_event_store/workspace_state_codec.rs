use super::canonical_cbor::CborValue;
use super::envelope::{EventCodecError, LocalEventPayloadCodec};
use crate::adaptor::gateway::workspace_state::repository_impl::StoredWorkspaceState;
use crate::domain::local_event::LocalDomainEvent;

pub(crate) struct WorkspaceStateCodec;
impl LocalEventPayloadCodec for WorkspaceStateCodec {
    fn event_type(&self) -> &'static str {
        "workspace.state_changed"
    }
    fn payload_version(&self) -> i64 {
        1
    }
    fn handles(&self, event: &LocalDomainEvent) -> bool {
        matches!(event, LocalDomainEvent::WorkspaceStateChanged(_))
    }
    fn encode(&self, event: &LocalDomainEvent) -> Result<CborValue, EventCodecError> {
        let LocalDomainEvent::WorkspaceStateChanged(value) = event else {
            return Err(self.malformed());
        };
        serde_json::to_string(&StoredWorkspaceState::from(value.as_ref()))
            .map(CborValue::Text)
            .map_err(|_| self.malformed())
    }
    fn decode(
        &self,
        version: i64,
        value: &CborValue,
    ) -> Result<Option<LocalDomainEvent>, EventCodecError> {
        if version != 1 {
            return Ok(None);
        }
        let CborValue::Text(raw) = value else {
            return Err(self.malformed());
        };
        let stored: StoredWorkspaceState =
            serde_json::from_str(raw).map_err(|_| self.malformed())?;
        let state = stored.try_into().map_err(|_| self.malformed())?;
        Ok(Some(LocalDomainEvent::WorkspaceStateChanged(Box::new(
            state,
        ))))
    }
}
impl WorkspaceStateCodec {
    fn malformed(&self) -> EventCodecError {
        EventCodecError::MalformedPayload {
            event_type: self.event_type().into(),
        }
    }
}

pub(crate) struct RepositoryGroupCodec;
impl LocalEventPayloadCodec for RepositoryGroupCodec {
    fn event_type(&self) -> &'static str {
        "workspace.repository_group_changed"
    }
    fn payload_version(&self) -> i64 {
        1
    }
    fn handles(&self, event: &LocalDomainEvent) -> bool {
        matches!(event, LocalDomainEvent::RepositoryGroupChanged(_))
    }
    fn encode(&self, event: &LocalDomainEvent) -> Result<CborValue, EventCodecError> {
        let LocalDomainEvent::RepositoryGroupChanged(value) = event else {
            return Err(self.malformed());
        };
        serde_json::to_string(value)
            .map(CborValue::Text)
            .map_err(|_| self.malformed())
    }
    fn decode(
        &self,
        version: i64,
        value: &CborValue,
    ) -> Result<Option<LocalDomainEvent>, EventCodecError> {
        if version != 1 {
            return Ok(None);
        }
        let CborValue::Text(raw) = value else {
            return Err(self.malformed());
        };
        serde_json::from_str(raw)
            .map(LocalDomainEvent::RepositoryGroupChanged)
            .map(Some)
            .map_err(|_| self.malformed())
    }
}
impl RepositoryGroupCodec {
    fn malformed(&self) -> EventCodecError {
        EventCodecError::MalformedPayload {
            event_type: self.event_type().into(),
        }
    }
}

pub(crate) struct WorkspaceStateInvalidationCodec;

impl LocalEventPayloadCodec for WorkspaceStateInvalidationCodec {
    fn event_type(&self) -> &'static str {
        "workspace.state_invalidated"
    }
    fn payload_version(&self) -> i64 {
        1
    }
    fn handles(&self, event: &LocalDomainEvent) -> bool {
        matches!(event, LocalDomainEvent::WorkspaceStateInvalidated)
    }
    fn encode(&self, event: &LocalDomainEvent) -> Result<CborValue, EventCodecError> {
        if self.handles(event) {
            Ok(CborValue::Null)
        } else {
            Err(EventCodecError::MalformedPayload {
                event_type: self.event_type().into(),
            })
        }
    }
    fn decode(
        &self,
        version: i64,
        value: &CborValue,
    ) -> Result<Option<LocalDomainEvent>, EventCodecError> {
        if version != 1 {
            return Ok(None);
        }
        if *value == CborValue::Null {
            Ok(Some(LocalDomainEvent::WorkspaceStateInvalidated))
        } else {
            Err(EventCodecError::MalformedPayload {
                event_type: self.event_type().into(),
            })
        }
    }
}

#[cfg(test)]
#[path = "workspace_state_codec_test.rs"]
mod workspace_state_codec_tests;
