//! Versioned canonical codec for workflow domain events.

use crate::adaptor::gateway::local_event_store::canonical_cbor::CborValue;
use crate::adaptor::gateway::local_event_store::envelope::{
    EventCodecError, LocalEventPayloadCodec,
};
use crate::domain::local_event::LocalDomainEvent;

pub(crate) const WORKFLOW_EVENT_TYPE: &str = "workflow.execution_event";
pub(crate) const WORKFLOW_PAYLOAD_VERSION: i64 = 1;

pub(crate) struct WorkflowDomainEventCodec;

impl LocalEventPayloadCodec for WorkflowDomainEventCodec {
    fn event_type(&self) -> &'static str {
        WORKFLOW_EVENT_TYPE
    }

    fn payload_version(&self) -> i64 {
        WORKFLOW_PAYLOAD_VERSION
    }

    fn handles(&self, event: &LocalDomainEvent) -> bool {
        matches!(event, LocalDomainEvent::Workflow(_))
    }

    fn encode(&self, event: &LocalDomainEvent) -> Result<CborValue, EventCodecError> {
        let LocalDomainEvent::Workflow(event) = event else {
            return Err(EventCodecError::MalformedPayload {
                event_type: WORKFLOW_EVENT_TYPE.to_string(),
            });
        };
        let stored =
            crate::adaptor::gateway::workflow::event::from_domain_event(event).map_err(|_| {
                EventCodecError::MalformedPayload {
                    event_type: WORKFLOW_EVENT_TYPE.to_string(),
                }
            })?;
        let raw =
            serde_json::to_string(&stored).map_err(|_| EventCodecError::MalformedPayload {
                event_type: WORKFLOW_EVENT_TYPE.to_string(),
            })?;
        Ok(CborValue::Text(raw))
    }

    fn decode(
        &self,
        payload_version: i64,
        value: &CborValue,
    ) -> Result<Option<LocalDomainEvent>, EventCodecError> {
        if payload_version != WORKFLOW_PAYLOAD_VERSION {
            return Ok(None);
        }
        let CborValue::Text(raw) = value else {
            return Err(EventCodecError::MalformedPayload {
                event_type: WORKFLOW_EVENT_TYPE.to_string(),
            });
        };
        let stored = serde_json::from_str(raw).map_err(|_| EventCodecError::MalformedPayload {
            event_type: WORKFLOW_EVENT_TYPE.to_string(),
        })?;
        crate::adaptor::gateway::workflow::event::to_domain_event(&stored)
            .map(|event| Some(LocalDomainEvent::Workflow(event)))
            .map_err(|_| EventCodecError::MalformedPayload {
                event_type: WORKFLOW_EVENT_TYPE.to_string(),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptor::gateway::local_event_store::canonical_cbor::{
        decode_canonical, encode_canonical,
    };
    use crate::domain::workflow::events::WorkflowDomainEvent;

    #[test]
    fn workflow_event_round_trips_through_the_versioned_canonical_payload() {
        let event = LocalDomainEvent::Workflow(WorkflowDomainEvent::WorkflowExecutionAborted {
            execution_id: "00000000-0000-4000-8000-000000000149".to_string(),
            aborted_node: Some("review".to_string()),
            timestamp: 1499.25,
        });
        let codec = WorkflowDomainEventCodec;

        let value = codec.encode(&event).unwrap();
        let bytes = encode_canonical(&value).unwrap();
        let decoded_value = decode_canonical(&bytes).unwrap();
        let decoded = codec
            .decode(WORKFLOW_PAYLOAD_VERSION, &decoded_value)
            .unwrap();

        assert_eq!(decoded, Some(event));
        assert_eq!(codec.decode(2, &decoded_value).unwrap(), None);
    }
}
