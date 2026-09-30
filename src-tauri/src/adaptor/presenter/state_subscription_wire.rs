use crate::adaptor::presenter::client as wire;
use crate::adaptor::presenter::connect_wire::{rpc, to_rpc};
use crate::adaptor::presenter::state_subscription::StateSubscriptionEvent;
use crate::infrastructure::state_subscription::{Delivery, Event};
use crate::usecase::state_subscription::StateValue;

pub(crate) fn payload(value: &StateValue) -> Result<wire::StatePayload, connectrpc::ConnectError> {
    Ok(wire::StatePayload {
        value: Some(match value {
            StateValue::Failures(records) => {
                wire::state_payload::Value::Failures(wire::FailureRecords {
                    next_offset: records.next_offset.map(|offset| offset as u32),
                    requires_attention: Some(records.requires_attention),
                    items: records
                        .items
                        .iter()
                        .map(|observation| {
                            let record = &observation.record;
                            wire::FailureRecord {
                                operation: Some(record.operation.clone()),
                                target: Some(record.target.clone()),
                                classification: Some(failure_classification(record.classification).into()),
                                message: Some(record.message.clone()),
                                count: Some(record.count),
                                first_observed_ms: Some(record.first_observed_ms),
                                last_observed_ms: Some(record.last_observed_ms),
                                requires_attention: Some(observation.requires_attention),
                            }
                        })
                        .collect(),
                })
            }
            StateValue::Terminal(value) => wire::state_payload::Value::Terminal(
                crate::adaptor::presenter::terminal::TerminalSurfaceStreamItemV1::from(
                    value.clone(),
                )
                .into(),
            ),
            StateValue::Workspaces(value) => wire::state_payload::Value::Workspaces(
                crate::adaptor::presenter::client::value(value.clone())
                    .map_err(crate::adaptor::presenter::connect::command_error)?,
            ),
            StateValue::Selection(value) => wire::state_payload::Value::Selection(
                crate::adaptor::presenter::client::value(value.clone())
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
                crate::adaptor::presenter::client::value(value.clone())
                    .map_err(crate::adaptor::presenter::connect::command_error)?,
            ),
            StateValue::CurrentBranch(value) => wire::state_payload::Value::CurrentBranch(
                crate::adaptor::presenter::client::value(value.clone())
                    .map_err(crate::adaptor::presenter::connect::command_error)?,
            ),
            StateValue::Issues(value) => wire::state_payload::Value::Issues(
                crate::adaptor::presenter::client::value(value.clone())
                    .map_err(crate::adaptor::presenter::connect::command_error)?,
            ),
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
                wire::state_payload::Value::ProviderHookHealth(
                    crate::adaptor::presenter::client::value(
                        value
                            .iter()
                            .cloned()
                            .map(crate::adaptor::presenter::agent_session::ProviderHookHealthWarningResponse::from)
                            .collect::<Vec<_>>(),
                    )
                    .map_err(crate::adaptor::presenter::connect::command_error)?,
                )
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

pub(crate) fn failure_classification(
    classification: crate::usecase::failure::FailureClassificationDto,
) -> &'static str {
    use crate::usecase::failure::FailureClassificationDto as C;
    match classification {
        C::VersionConflict => "VersionConflict",
        C::BusinessFailure => "BusinessFailure",
        C::Transient => "Transient",
        C::TimedOut => "TimedOut",
        C::Cancelled => "Cancelled",
        C::TechnicalFailure => "TechnicalFailure",
    }
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
                Event::Snapshot(_, value) => WireEvent::Snapshot((*value).clone()),
                Event::Change(_, delivery, value) => WireEvent::Change(wire::StateChange {
                    delta: delivery == Delivery::Delta,
                    payload: Some((*value).clone()),
                }),
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
