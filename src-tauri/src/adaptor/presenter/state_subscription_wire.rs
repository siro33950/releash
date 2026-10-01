use crate::adaptor::presenter::client as wire;
use crate::adaptor::presenter::connect_wire::{rpc, to_rpc};
use crate::adaptor::presenter::state_subscription::{PublishedState, StateSubscriptionEvent};
use crate::infrastructure::state_subscription::{Delivery, Event};
use crate::usecase::state_subscription::StateValue;

pub(crate) fn payload(value: &StateValue) -> Result<wire::StatePayload, connectrpc::ConnectError> {
    Ok(wire::StatePayload {
        value: Some(match value {
            StateValue::Terminal(value) => wire::state_payload::Value::Terminal(
                crate::adaptor::presenter::terminal::TerminalSurfaceStreamItemV1::from(
                    value.clone(),
                )
                .into(),
            ),
            StateValue::Workspaces(value) => wire::state_payload::Value::Workspaces(
                crate::adaptor::presenter::client::value(value)
                    .map_err(crate::adaptor::presenter::connect::command_error)?,
            ),
            StateValue::Selection(tree, selected) => wire::state_payload::Value::Selection(
                crate::adaptor::presenter::client::value((tree, *selected))
                    .map_err(crate::adaptor::presenter::connect::command_error)?,
            ),
            StateValue::NodeDetail(value) => wire::state_payload::Value::NodeDetail(
                crate::adaptor::presenter::client::value(value.clone())
                    .map_err(crate::adaptor::presenter::connect::command_error)?,
            ),
            StateValue::AgentSession(value) => wire::state_payload::Value::AgentSession(
                crate::adaptor::presenter::client::value(value.clone())
                    .map_err(crate::adaptor::presenter::connect::command_error)?,
            ),
            StateValue::SessionNode(value) => wire::state_payload::Value::SessionNode(
                crate::adaptor::presenter::client::value(value.clone())
                    .map_err(crate::adaptor::presenter::connect::command_error)?,
            ),
            StateValue::SessionHistory(value) => wire::state_payload::Value::SessionHistory(
                crate::adaptor::presenter::client::value(value.clone())
                    .map_err(crate::adaptor::presenter::connect::command_error)?,
            ),
            StateValue::Providers(value) => wire::state_payload::Value::Providers(
                crate::adaptor::presenter::client::value(value.clone())
                    .map_err(crate::adaptor::presenter::connect::command_error)?,
            ),
            StateValue::Branches(value) => wire::state_payload::Value::Branches(
                crate::adaptor::presenter::client::value(value.clone())
                    .map_err(crate::adaptor::presenter::connect::command_error)?,
            ),
            StateValue::BranchBase(value) => wire::state_payload::Value::BranchBase(
                crate::adaptor::presenter::client::value(value.clone())
                    .map_err(crate::adaptor::presenter::connect::command_error)?,
            ),
            StateValue::BranchStatus(value) => wire::state_payload::Value::BranchStatus(
                crate::adaptor::presenter::client::value(value.as_slice())
                    .map_err(crate::adaptor::presenter::connect::command_error)?,
            ),
            StateValue::CurrentBranch(value) => wire::state_payload::Value::CurrentBranch(
                crate::adaptor::presenter::client::value(value.clone())
                    .map_err(crate::adaptor::presenter::connect::command_error)?,
            ),
            StateValue::Issues(value) => wire::state_payload::Value::Issues(wire::IssuesSnapshot {
                issues: value.value.as_ref().map(|issues| crate::adaptor::presenter::client::value(
                    issues.iter().cloned().map(crate::usecase::git_host::IssueInfoDto::from).collect::<Vec<_>>()
                )).transpose().map_err(crate::adaptor::presenter::connect::command_error)?,
                read_error: value.error.as_ref().map(ToString::to_string),
            }),
            StateValue::Worktrees(value) => wire::state_payload::Value::Worktrees(
                crate::adaptor::presenter::client::value(value.clone())
                    .map_err(crate::adaptor::presenter::connect::command_error)?,
            ),
            StateValue::RepositoryRoot(value) => wire::state_payload::Value::RepositoryRoot(
                crate::adaptor::presenter::client::value(value.clone())
                    .map_err(crate::adaptor::presenter::connect::command_error)?,
            ),
            StateValue::StartupRepository(value) => wire::state_payload::Value::StartupRepository(
                crate::adaptor::presenter::client::value(value.clone())
                    .map_err(crate::adaptor::presenter::connect::command_error)?,
            ),
            StateValue::WorkspaceState(value) => wire::state_payload::Value::WorkspaceState(
                crate::adaptor::presenter::client::value(value.clone())
                    .map_err(crate::adaptor::presenter::connect::command_error)?,
            ),
            StateValue::ReviewSnapshot(value) => wire::state_payload::Value::ReviewSnapshot(
                crate::adaptor::presenter::client::value(value.clone())
                    .map_err(crate::adaptor::presenter::connect::command_error)?,
            ),
            StateValue::ReviewFileView(value) => wire::state_payload::Value::ReviewFileView(
                crate::adaptor::presenter::client::value(value.clone())
                    .map_err(crate::adaptor::presenter::connect::command_error)?,
            ),
            StateValue::ReviewThreads(value) => wire::state_payload::Value::ReviewThreads(
                crate::adaptor::presenter::client::value(value.clone())
                    .map_err(crate::adaptor::presenter::connect::command_error)?,
            ),
            StateValue::Workflows(value) => wire::state_payload::Value::Workflows(
                crate::adaptor::presenter::client::value(value.clone())
                    .map_err(crate::adaptor::presenter::connect::command_error)?,
            ),
            StateValue::Workflow(value) => wire::state_payload::Value::Workflow(
                crate::adaptor::presenter::client::value(value.clone())
                    .map_err(crate::adaptor::presenter::connect::command_error)?,
            ),
            StateValue::WorkflowSource(value) => wire::state_payload::Value::WorkflowSource(
                crate::adaptor::presenter::client::value(value.clone())
                    .map_err(crate::adaptor::presenter::connect::command_error)?,
            ),
            StateValue::Facets(value) => wire::state_payload::Value::Facets(
                crate::adaptor::presenter::client::value(value.clone())
                    .map_err(crate::adaptor::presenter::connect::command_error)?,
            ),
            StateValue::Facet(value) => wire::state_payload::Value::Facet(
                crate::adaptor::presenter::client::value(value.clone())
                    .map_err(crate::adaptor::presenter::connect::command_error)?,
            ),
            StateValue::Diagnostics(value) => wire::state_payload::Value::Diagnostics(
                crate::adaptor::presenter::client::value(value.clone())
                    .map_err(crate::adaptor::presenter::connect::command_error)?,
            ),
            StateValue::DesktopSettings(value) => {
                wire::state_payload::Value::DesktopSettings((*value).into())
            }
            StateValue::NotionConfig(value) => wire::state_payload::Value::NotionConfig(
                crate::adaptor::presenter::client::value(
                    value
                        .clone()
                        .map(crate::adaptor::presenter::notion::NotionRepoConfigView::from),
                )
                .map_err(crate::adaptor::presenter::connect::command_error)?,
            ),
            StateValue::ProviderAvailability(value) => {
                wire::state_payload::Value::ProviderAvailability(
                    crate::adaptor::presenter::client::value(
                        crate::adaptor::presenter::agent_session::ProviderAvailabilitySnapshotResponse::from(
                            value.clone(),
                        ),
                    )
                    .map_err(crate::adaptor::presenter::connect::command_error)?,
                )
            }
            StateValue::ExternalEditor(value) => {
                wire::state_payload::Value::ExternalEditor(wire::ExternalEditorState {
                    selected: Some(value.selected.clone()),
                    editors: Some(
                        crate::adaptor::presenter::client::value(value.editors.clone())
                            .map_err(crate::adaptor::presenter::connect::command_error)?,
                    ),
                })
            }
            StateValue::ReleashBase(value) => wire::state_payload::Value::ReleashBase(
                crate::adaptor::presenter::client::value(value.clone())
                    .map_err(crate::adaptor::presenter::connect::command_error)?,
            ),
            StateValue::WorkflowConfig(value) => wire::state_payload::Value::WorkflowConfig(
                crate::adaptor::presenter::client::value(*value)
                .map_err(crate::adaptor::presenter::connect::command_error)?,
            ),
            StateValue::PerformanceSwitches(value) => {
                wire::state_payload::Value::PerformanceSwitches(wire::PerformanceSwitchesV1 {
                    real_app_mode: Some(value.real_app_mode),
                    terminal: Some(
                        crate::adaptor::presenter::client::value(
                            crate::adaptor::presenter::terminal::TerminalPerformanceSwitchesV1::from(
                                value.terminal,
                            ),
                        )
                        .map_err(crate::adaptor::presenter::connect::command_error)?,
                    ),
                })
            }
            StateValue::ProviderHookHealth(value) => {
                wire::state_payload::Value::ProviderHookHealth(wire::ProviderHookHealthSnapshot {
                    warnings: Some(crate::adaptor::presenter::client::value(
                        value.warnings.iter().cloned().map(crate::adaptor::presenter::agent_session::ProviderHookHealthWarningResponse::from).collect::<Vec<_>>()
                    ).map_err(crate::adaptor::presenter::connect::command_error)?),
                    read_errors: value.failures.iter().map(|failure| match failure {
                        crate::usecase::provider_lifecycle::ProviderHookHealthFailureQueryError::Technical(failure) => failure.message.clone(),
                        crate::usecase::provider_lifecycle::ProviderHookHealthFailureQueryError::Corrupt => "Provider Hook health record is corrupt".into(),
                    }).collect(),
                })
            }

            StateValue::StartupOutcome(value) => wire::state_payload::Value::StartupOutcome(
                crate::adaptor::presenter::client::value(
                    crate::adaptor::presenter::application_lifecycle::application_startup_outcome(
                        value.clone(),
                    ),
                )
                .map_err(crate::adaptor::presenter::connect::command_error)?,
            ),

            StateValue::RepositoryPaths(paths) => {
                wire::state_payload::Value::RepositoryPaths(wire::Liststring {
                    items: paths.clone(),
                })
            }
        }),
    })
}

pub(crate) fn event(
    event: StateSubscriptionEvent,
) -> Result<rpc::StateSubscriptionEvent, connectrpc::ConnectError> {
    use wire::state_subscription_event::Event as WireEvent;
    let (target, args, version, event) = match event {
        StateSubscriptionEvent::Ready => {
            (String::new(), vec![], None, WireEvent::Ready(wire::Unit {}))
        }
        StateSubscriptionEvent::Bookmark => (
            String::new(),
            vec![],
            None,
            WireEvent::Bookmark(wire::Unit {}),
        ),
        StateSubscriptionEvent::Item(target, event) => {
            let version = event.version();
            let version = Some(wire::StateVersion {
                epoch: version.epoch.clone(),
                sequence: version.sequence,
            });
            let event = match event {
                Event::Snapshot(_, value) => match value.as_ref() {
                    PublishedState::Failure(failure) => WireEvent::Failure(failure.clone()),
                    PublishedState::Value(value) => WireEvent::Snapshot(value.as_ref().clone()),
                },
                Event::Change(_, delivery, value) => match value.as_ref() {
                    PublishedState::Failure(failure) => WireEvent::Failure(failure.clone()),
                    PublishedState::Value(value) => WireEvent::Change(wire::StateChange {
                        delta: delivery == Delivery::Delta,
                        payload: Some(value.as_ref().clone()),
                    }),
                },
                Event::Bookmark(_) => WireEvent::Bookmark(wire::Unit {}),
            };
            let target = crate::usecase::state_subscription::SubscriptionTarget::parse(&target)
                .map_err(crate::adaptor::presenter::connect::classified_error)?;
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
#[path = "state_subscription_wire_test.rs"]
mod state_subscription_tests;
