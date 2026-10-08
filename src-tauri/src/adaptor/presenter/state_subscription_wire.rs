use crate::adaptor::presenter::client as wire;
use crate::adaptor::presenter::connect_wire::{rpc, to_rpc};
use crate::adaptor::presenter::state_subscription::{PublishedState, StateSubscriptionEvent};
use crate::infrastructure::state_subscription::{Delivery, Event};
use crate::usecase::state_subscription::StateValue;

pub fn payload(value: &StateValue) -> Result<wire::StatePayload, connectrpc::ConnectError> {
    Ok(wire::StatePayload {
        value: Some(match value {
            StateValue::DaemonInfo(info) => wire::state_payload::Value::DaemonInfo(
                crate::adaptor::presenter::daemon::server_info(info.clone()),
            ),
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
                crate::adaptor::presenter::client::selection(tree, *selected).map_err(|e| crate::adaptor::presenter::error::AppError::new(e).into())
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
                Ok::<_, crate::adaptor::presenter::client::CommandFailure>(crate::adaptor::presenter::client::branch_status(value.as_slice()))
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
            StateValue::NotionTasks(value) => wire::state_payload::Value::NotionTasks(wire::NotionTasksSnapshot {
                page: value.value.clone().map(crate::adaptor::presenter::notion::NotionTaskPageView::from).map(crate::adaptor::presenter::client::value).transpose().map_err(crate::adaptor::presenter::connect::command_error)?,
                read_error: value.error.as_ref().map(notion_read_failure),
            }),
            StateValue::NotionLabelOptions(value) => wire::state_payload::Value::NotionLabelOptions(wire::NotionLabelOptionsSnapshot {
                options: value.value.as_ref().map(|options| crate::adaptor::presenter::client::value(options.iter().cloned().map(crate::adaptor::presenter::notion::NotionLabelOptionView::from).collect::<Vec<_>>())).transpose().map_err(crate::adaptor::presenter::connect::command_error)?,
                read_error: value.error.as_ref().map(notion_read_failure),
            }),
            StateValue::Worktrees(value) => wire::state_payload::Value::Worktrees(
                crate::adaptor::presenter::client::value(value.clone())
                    .map_err(crate::adaptor::presenter::connect::command_error)?,
            ),
            StateValue::StartupRepository(value) => wire::state_payload::Value::StartupRepository(wire::NullableStartupWorktree {
                value: value.as_ref().map(|value| wire::StartupWorktree { path: Some(value.path.clone()), branch: Some(value.branch.clone()), repository_name: Some(value.repository_name.clone()) })
            }),
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
            StateValue::WorkflowExecution(value) => wire::state_payload::Value::WorkflowExecution(wire::NullableWorkflowExecutionView {
                value: value.as_ref().map(|tree| crate::adaptor::presenter::client::value(crate::adaptor::presenter::workflow::workflow_execution_to_view(tree.clone()))).transpose().map_err(crate::adaptor::presenter::connect::command_error)?,
            }),
            StateValue::WorkflowOutput(value) => wire::state_payload::Value::WorkflowOutput(wire::NullableWorkflowOutputView {
                value: value.clone().map(|output| crate::adaptor::presenter::client::value(crate::adaptor::presenter::workflow_wire::WorkflowGetOutputResponse::from(output))).transpose().map_err(crate::adaptor::presenter::connect::command_error)?,
            }),
            StateValue::ReviewSessionThreads(value) => wire::state_payload::Value::ReviewSessionThreads(wire::NullableListReviewThreadDto {
                value: value.clone().map(crate::adaptor::presenter::client::value).transpose().map_err(crate::adaptor::presenter::connect::command_error)?,
            }),
            StateValue::ReviewSessionThread(value) => wire::state_payload::Value::ReviewSessionThread(wire::NullableReviewThreadDto {
                value: value.clone().map(crate::adaptor::presenter::client::value).transpose().map_err(crate::adaptor::presenter::connect::command_error)?,
            }),
            StateValue::ReviewSessionThreadHistory(value) => wire::state_payload::Value::ReviewSessionThreadHistory(wire::NullableListReviewHistoryEntryDto {
                value: value.as_ref().map(|entries| entries.iter().cloned().map(wire::ReviewHistoryEntryDto::try_from).collect::<Result<Vec<_>, _>>().map(|items| wire::ListReviewHistoryEntryDto { items })).transpose().map_err(|error| crate::adaptor::presenter::connect::command_error(crate::adaptor::presenter::error::AppError::new(error).into()))?,
            }),
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

            StateValue::RepositoryPaths(paths) => {
                wire::state_payload::Value::RepositoryPaths(wire::Liststring {
                    items: paths.clone(),
                })
            }
        }),
    })
}

fn notion_read_failure(
    error: &crate::usecase::notion::error::NotionUsecaseError,
) -> wire::NotionReadFailure {
    use crate::adaptor::presenter::connect::ConnectFailure;
    wire::NotionReadFailure {
        code: error.connect_code().grpc_code() as i32,
        message: error.to_string(),
        config_missing: Some(matches!(
            error,
            crate::usecase::notion::error::NotionUsecaseError::ConfigNotFound
        )),
    }
}

pub(crate) fn event(
    event: StateSubscriptionEvent,
) -> Result<rpc::StateSubscriptionEvent, connectrpc::ConnectError> {
    let event = &event;
    use wire::state_subscription_event::Event as WireEvent;
    let (subscription_id, version, event) = match event {
        StateSubscriptionEvent::Ready => (String::new(), None, WireEvent::Ready(wire::Unit {})),
        StateSubscriptionEvent::Bookmark => {
            (String::new(), None, WireEvent::Bookmark(wire::Unit {}))
        }
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
                        delta: *delivery == Delivery::Delta,
                        payload: Some(value.as_ref().clone()),
                    }),
                },
                Event::Bookmark(_) => WireEvent::Bookmark(wire::Unit {}),
            };
            (target.clone(), version, event)
        }
    };
    to_rpc(&wire::StateSubscriptionEvent {
        subscription_id,
        version,
        event: Some(event),
    })
}

#[cfg(test)]
#[path = "state_subscription_wire_test.rs"]
pub(crate) mod state_subscription_wire_tests;
