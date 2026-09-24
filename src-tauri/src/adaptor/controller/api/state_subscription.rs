use super::protocol::{
    client as wire,
    connect::{rpc, to_rpc},
};
use crate::domain::state_subscription::{Delivery, Event};
use crate::usecase::state_subscription::StateSubscriptionEvent;
use crate::usecase::state_subscription::StateValue;

fn payload(value: &StateValue) -> Result<wire::StatePayload, connectrpc::ConnectError> {
    Ok(wire::StatePayload {
        value: Some(match value {
            StateValue::Workspaces(value) => wire::state_payload::Value::Workspaces(
                crate::adaptor::controller::client::value(value.clone())
                    .map_err(crate::adaptor::protocol::connect::command_error)?,
            ),
            StateValue::Selection(value) => wire::state_payload::Value::Selection(
                crate::adaptor::controller::client::value(value.clone())
                    .map_err(crate::adaptor::protocol::connect::command_error)?,
            ),
            StateValue::NodeDetail(value) => wire::state_payload::Value::NodeDetail(
                crate::adaptor::controller::client::value(value.clone())
                    .map_err(crate::adaptor::protocol::connect::command_error)?,
            ),
            StateValue::AgentSession(value) => wire::state_payload::Value::AgentSession(
                crate::adaptor::controller::client::value(value.clone())
                    .map_err(crate::adaptor::protocol::connect::command_error)?,
            ),
            StateValue::SessionNode(value) => wire::state_payload::Value::SessionNode(
                crate::adaptor::controller::client::value(value.clone())
                    .map_err(crate::adaptor::protocol::connect::command_error)?,
            ),
            StateValue::SessionHistory(value) => wire::state_payload::Value::SessionHistory(
                crate::adaptor::controller::client::value(value.clone())
                    .map_err(crate::adaptor::protocol::connect::command_error)?,
            ),
            StateValue::Providers(value) => wire::state_payload::Value::Providers(
                crate::adaptor::controller::client::value(value.clone())
                    .map_err(crate::adaptor::protocol::connect::command_error)?,
            ),
            StateValue::Branches(value) => wire::state_payload::Value::Branches(
                crate::adaptor::controller::client::value(value.clone())
                    .map_err(crate::adaptor::protocol::connect::command_error)?,
            ),
            StateValue::BranchBase(value) => wire::state_payload::Value::BranchBase(
                crate::adaptor::controller::client::value(value.clone())
                    .map_err(crate::adaptor::protocol::connect::command_error)?,
            ),
            StateValue::BranchStatus(value) => wire::state_payload::Value::BranchStatus(
                crate::adaptor::controller::client::value(value.clone())
                    .map_err(crate::adaptor::protocol::connect::command_error)?,
            ),
            StateValue::CurrentBranch(value) => wire::state_payload::Value::CurrentBranch(
                crate::adaptor::controller::client::value(value.clone())
                    .map_err(crate::adaptor::protocol::connect::command_error)?,
            ),
            StateValue::Issues(value) => wire::state_payload::Value::Issues(
                crate::adaptor::controller::client::value(value.clone())
                    .map_err(crate::adaptor::protocol::connect::command_error)?,
            ),
            StateValue::Worktrees(value) => wire::state_payload::Value::Worktrees(
                crate::adaptor::controller::client::value(value.clone())
                    .map_err(crate::adaptor::protocol::connect::command_error)?,
            ),
            StateValue::RepositoryRoot(value) => wire::state_payload::Value::RepositoryRoot(
                crate::adaptor::controller::client::value(value.clone())
                    .map_err(crate::adaptor::protocol::connect::command_error)?,
            ),
            StateValue::StartupRepository(value) => wire::state_payload::Value::StartupRepository(
                crate::adaptor::controller::client::value(value.clone())
                    .map_err(crate::adaptor::protocol::connect::command_error)?,
            ),
            StateValue::WorkspaceState(value) => wire::state_payload::Value::WorkspaceState(
                crate::adaptor::controller::client::value(value.clone())
                    .map_err(crate::adaptor::protocol::connect::command_error)?,
            ),

            StateValue::RepositoryPaths(paths) => {
                wire::state_payload::Value::RepositoryPaths(wire::Liststring {
                    items: paths.clone(),
                })
            }
        }),
    })
}

pub(super) fn event(
    event: StateSubscriptionEvent,
) -> Result<rpc::StateSubscriptionEvent, connectrpc::ConnectError> {
    use wire::state_subscription_event::Event as WireEvent;
    let (target, args, version, event) = match event {
        StateSubscriptionEvent::Ready => {
            (String::new(), vec![], None, WireEvent::Ready(wire::Unit {}))
        }
        StateSubscriptionEvent::Item(target, event) => {
            let version = event.version();
            let version = Some(wire::StateVersion {
                epoch: version.epoch.clone(),
                sequence: version.sequence,
            });
            let event = match event {
                Event::Snapshot(_, value) => WireEvent::Snapshot(payload(&value)?),
                Event::Change(_, delivery, value) => WireEvent::Change(wire::StateChange {
                    delta: delivery == Delivery::Delta,
                    payload: Some(payload(&value)?),
                }),
                Event::Bookmark(_) => WireEvent::Bookmark(wire::Unit {}),
            };
            let target = crate::domain::state_subscription::SubscriptionTarget::parse(&target)
                .map_err(crate::adaptor::protocol::connect::classified_error)?;
            let (name, args) = target.parts();
            (name.into(), args, version, event)
        }
    };
    to_rpc(&wire::StateSubscriptionEvent {
        target,
        args,
        version,
        event: Some(event),
    })
}

#[cfg(test)]
#[path = "state_subscription_test.rs"]
mod state_subscription_tests;
