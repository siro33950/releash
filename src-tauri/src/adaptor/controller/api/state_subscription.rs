use super::protocol::{
    client as wire,
    connect::{rpc, to_rpc},
};
use crate::domain::state_subscription::StateValue;
use crate::domain::state_subscription::{Delivery, Event};
use crate::usecase::state_subscription::StateSubscriptionEvent;

fn payload(value: &StateValue) -> wire::StatePayload {
    wire::StatePayload {
        value: Some(match value {
            StateValue::RepositoryPaths(paths) => {
                wire::state_payload::Value::RepositoryPaths(wire::Liststring {
                    items: paths.clone(),
                })
            }
        }),
    }
}

pub(super) fn event(
    event: StateSubscriptionEvent,
) -> Result<rpc::StateSubscriptionEvent, connectrpc::ConnectError> {
    use wire::state_subscription_event::Event as WireEvent;
    let (target, version, event) = match event {
        StateSubscriptionEvent::Ready => (String::new(), None, WireEvent::Ready(wire::Unit {})),
        StateSubscriptionEvent::Item(target, event) => {
            let version = event.version();
            let version = Some(wire::StateVersion {
                epoch: version.epoch.clone(),
                sequence: version.sequence,
            });
            let event = match event {
                Event::Snapshot(_, value) => WireEvent::Snapshot(payload(&value)),
                Event::Change(_, delivery, value) => WireEvent::Change(wire::StateChange {
                    delta: delivery == Delivery::Delta,
                    payload: Some(payload(&value)),
                }),
                Event::Bookmark(_) => WireEvent::Bookmark(wire::Unit {}),
            };
            (target, version, event)
        }
    };
    to_rpc(&wire::StateSubscriptionEvent {
        target,
        version,
        event: Some(event),
    })
}
