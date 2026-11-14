use super::*;
use crate::infrastructure::state_subscription::{Delivery, Version};
use crate::usecase::state_subscription::SubscriptionTarget;
use std::sync::Arc;

use crate::domain::agent_session::aggregates::{AgentSession, AgentSessionTreeLocation};
use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::workflow::{ExecutionStatus, ExecutionTreeArchiveRecord, ExecutionTreeLaunch};
use crate::domain::workspace_tree::{
    WorkspaceExecution, WorkspaceIdentity, WorkspaceNodeKind, WorkspaceNodeStatus, WorkspaceTree,
    WorkspaceTreeNode,
};

fn overview_tree(
    path: &str,
    provider: Option<ProviderKind>,
    owner_status: WorkspaceNodeStatus,
    child_status: WorkspaceNodeStatus,
) -> WorkspaceTree {
    let owner = WorkspaceTreeNode {
        worktree: None,
        id: "execution".into(),
        parent_id: None,
        sibling_order: 0,
        kind: WorkspaceNodeKind::Workflow,
        title: "Owner title".into(),
        status: owner_status,
        process_presence: Default::default(),
        status_classification:
            crate::domain::workspace_tree::WorkspaceNodeStatusClassification::Idle,
        delegate_waits_for_child: false,
        background_failure: false,
        activity: None,
        error_reason: None,
        updated_at_bits: 1.0f64.to_bits(),
        execution_id: Some("execution".into()),
        node_execution_id: None,
        node_name: None,
        attempt: None,
        retry_predecessor_id: None,
        past_attempt_ids: Vec::new(),
        is_retry_history: false,
        completion_signals: Default::default(),
        has_artifact: false,
        session_id: None,
        model: None,
        provider: None,
        can_rename: false,
        can_approve: false,
        can_retry: false,
        can_resume_session: false,
        can_abort: false,
        can_archive: false,
        display_command: None,
        shell: None,
        cwd: None,
        command_result: None,
        dynamic_fanout: false,
    };
    let mut child = WorkspaceTreeNode {
        id: "child".into(),
        parent_id: Some(owner.id.clone()),
        node_execution_id: Some("child-execution".into()),
        node_name: Some("main".into()),
        attempt: Some(0),
        kind: WorkspaceNodeKind::WorkflowCommand,
        status: child_status,
        ..owner.clone()
    };
    if provider.is_some() {
        child.kind = WorkspaceNodeKind::WorkflowSession;
        child.session_id = Some("session".into());
        child.activity = Some(if child_status == WorkspaceNodeStatus::Completed {
            crate::domain::workflow::AgentSessionActivity::AwaitingInstruction
        } else {
            crate::domain::workflow::AgentSessionActivity::Working
        });
    }
    let mut tree = WorkspaceTree::restore(path, vec![owner, child]).unwrap();
    tree.record_executions(vec![WorkspaceExecution {
        execution_id: "execution".into(),
        launched_as: if provider.is_some() {
            ExecutionTreeLaunch::Session
        } else {
            ExecutionTreeLaunch::Workflow
        },
        worktree_path: path.into(),
        workflow_name: "review-workflow".into(),
        status: ExecutionStatus::Running,
        updated_at: 1.0,
        archive: None,
        session: provider.map(|provider| {
            AgentSession::create(
                "session",
                WorkspaceIdentity::new(path),
                path,
                provider,
                AgentSessionTreeLocation::session_tree_root("session").unwrap(),
            )
            .unwrap()
        }),
    }]);
    tree
}

fn overview(trees: Vec<WorkspaceTree>) -> wire::ExecutionOverviewDto {
    let Some(wire::state_payload::Value::ExecutionOverview(overview)) =
        payload(&StateValue::ExecutionOverview((trees, vec![])))
            .unwrap()
            .value
    else {
        panic!("expected execution overview");
    };
    overview
}

#[test]
fn test_実行木一覧購読_集約と各実行木の三状態と表示名と選択先を転送する() {
    use wire::workspace_status_classification::Value::{Active, Attention, Idle};
    use WorkspaceNodeStatus::{Completed, Running, Waiting};
    // Given
    let trees = vec![
        overview_tree("/repo/c", Some(ProviderKind::Codex), Waiting, Running),
        overview_tree("/repo/b", None, Waiting, Completed),
        overview_tree("/repo/a", Some(ProviderKind::Claude), Running, Completed),
        overview_tree("/repo/d", None, Completed, Completed),
    ];
    // When
    let result = overview(trees);
    // Then
    assert_eq!(result.status.unwrap().value, Some(Attention as i32));
    assert_eq!(
        result
            .executions
            .iter()
            .map(|entry| (
                entry.worktree_path.as_str(),
                entry.worktree_name.as_str(),
                entry.name.as_str(),
                entry.status.as_ref().unwrap().value,
            ))
            .collect::<Vec<_>>(),
        vec![
            ("/repo/a", "a", "Claude", Some(Idle as i32)),
            ("/repo/b", "b", "review-workflow", Some(Attention as i32)),
            ("/repo/c", "c", "Codex", Some(Active as i32)),
            ("/repo/d", "d", "review-workflow", Some(Idle as i32)),
        ]
    );
    assert_eq!(
        overview(vec![overview_tree("/repo", None, Completed, Completed)])
            .status
            .unwrap()
            .value,
        Some(Idle as i32)
    );
    assert_eq!(
        overview(vec![overview_tree("/repo", None, Running, Completed)])
            .status
            .unwrap()
            .value,
        Some(Active as i32)
    );
}

#[test]
fn test_実行木一覧購読_空の一覧も集約状態とともに転送する() {
    // Given / When
    let result = overview(Vec::new());
    // Then
    assert!(result.executions.is_empty());
    assert_eq!(
        result.status.unwrap().value,
        Some(wire::workspace_status_classification::Value::Idle as i32)
    );
}

#[test]
fn test_実行木一覧購読_archiveされた木とsessionを集約から除く() {
    use WorkspaceNodeStatus::{Completed, Waiting};
    // Given
    let mut archived = overview_tree("/repo/archived", None, Waiting, Completed);
    let mut execution = archived.executions()[0].clone();
    execution.archive = Some(ExecutionTreeArchiveRecord {
        execution_id: execution.execution_id.clone(),
        archived_at: 2.0,
    });
    archived.record_executions(vec![execution]);
    let mut session = overview_tree(
        "/repo/session",
        Some(ProviderKind::Claude),
        Waiting,
        Waiting,
    );
    let mut execution = session.executions()[0].clone();
    crate::domain::agent_session::test_helpers::archive_derived(
        execution.session.as_mut().unwrap(),
    );
    session.record_executions(vec![execution]);
    // When
    let result = overview(vec![archived, session]);
    // Then
    assert!(result.executions.is_empty());
    assert_eq!(
        result.status.unwrap().value,
        Some(wire::workspace_status_classification::Value::Idle as i32)
    );
}

#[test]
fn test_実行木一覧購読_session未取得の木を除きルートパスを表示する() {
    use WorkspaceNodeStatus::{Completed, Waiting};
    // Given
    let mut pending = overview_tree("/repo/pending", Some(ProviderKind::Codex), Waiting, Waiting);
    let mut execution = pending.executions()[0].clone();
    execution.session = None;
    pending.record_executions(vec![execution]);
    // When
    let result = overview(vec![
        pending,
        overview_tree("/", None, Completed, Completed),
    ]);
    // Then
    assert_eq!(result.executions.len(), 1);
    assert_eq!(result.executions[0].worktree_path, "/");
    assert_eq!(result.executions[0].worktree_name, "/");
    assert_eq!(
        result.status.unwrap().value,
        Some(wire::workspace_status_classification::Value::Idle as i32)
    );
}

fn wire_event(event: StateSubscriptionEvent) -> wire::StateSubscriptionEvent {
    crate::adaptor::presenter::connect_wire::to_wire(&super::event(event).unwrap()).unwrap()
}

#[test]
fn test_購読事象_対象名と引数を載せず識別子で配信する() {
    // Given
    let target = "購読:1";
    let message = wire_event(StateSubscriptionEvent::Item(
        target.to_string(),
        Event::Snapshot(
            Version {
                epoch: "boot:1".into(),
                sequence: 7,
            },
            Arc::new(PublishedState::from(
                payload(&StateValue::ReleashBase(Some("main".into()))).unwrap(),
            )),
        ),
        // When
    ));
    // Then
    assert_eq!(
        message,
        wire::StateSubscriptionEvent {
            subscription_id: target.into(),
            event: Some(wire::state_subscription_event::Event::Snapshot(
                wire::StatePayload {
                    value: Some(wire::state_payload::Value::ReleashBase(
                        wire::Nullablestring {
                            value: Some("main".into()),
                        },
                    )),
                },
            )),
        }
    );
}

#[test]
fn test_購読事象_準備の旧転送形式を保つ() {
    // Given
    use wire::state_subscription_event::Event as WireEvent;
    let event = StateSubscriptionEvent::Ready;
    // When
    let actual = wire_event(event);
    // Then
    assert_eq!(
        actual,
        wire::StateSubscriptionEvent {
            subscription_id: String::new(),
            event: Some(WireEvent::Ready(wire::Unit {})),
        }
    );
}
#[test]
fn test_購読事象_全体の定期印の旧転送形式を保つ() {
    // Given
    use wire::state_subscription_event::Event as WireEvent;
    let event = StateSubscriptionEvent::Bookmark;
    // When
    let actual = wire_event(event);
    // Then
    assert_eq!(
        actual,
        wire::StateSubscriptionEvent {
            subscription_id: String::new(),
            event: Some(WireEvent::Bookmark(wire::Unit {})),
        }
    );
}
#[test]
fn test_購読事象_full変更の旧転送形式を保つ() {
    // Given
    use wire::state_subscription_event::Event as WireEvent;
    let event = StateSubscriptionEvent::Item(
        SubscriptionTarget::TerminalStartupCommand.to_string(),
        Event::Change(
            Version {
                epoch: "boot:1".into(),
                sequence: 8,
            },
            Delivery::Full,
            Arc::new(PublishedState::from(
                payload(&StateValue::TerminalStartupCommand("/repo".into())).unwrap(),
            )),
        ),
    );
    // When
    let actual = wire_event(event);
    // Then
    assert_eq!(
        actual,
        wire::StateSubscriptionEvent {
            subscription_id: "terminal-startup-command".into(),
            event: Some(WireEvent::Change(wire::StateChange {
                payload: Some(wire::StatePayload {
                    value: Some(wire::state_payload::Value::TerminalStartupCommand(
                        wire::ResultString {
                            value: Some("/repo".into())
                        }
                    )),
                })
            })),
        }
    );
}
#[test]
fn test_購読事象_delta変更の旧転送形式を保つ() {
    // Given
    use wire::state_subscription_event::Event as WireEvent;
    let event = StateSubscriptionEvent::Item(
        SubscriptionTarget::TerminalStartupCommand.to_string(),
        Event::Change(
            Version {
                epoch: "boot:1".into(),
                sequence: 9,
            },
            Delivery::Delta,
            Arc::new(PublishedState::from(
                payload(&StateValue::TerminalStartupCommand("/repo".into())).unwrap(),
            )),
        ),
    );
    // When
    let actual = wire_event(event);
    // Then
    assert_eq!(
        actual,
        wire::StateSubscriptionEvent {
            subscription_id: "terminal-startup-command".into(),
            event: Some(WireEvent::Change(wire::StateChange {
                payload: Some(wire::StatePayload {
                    value: Some(wire::state_payload::Value::TerminalStartupCommand(
                        wire::ResultString {
                            value: Some("/repo".into())
                        }
                    )),
                })
            })),
        }
    );
}
#[test]
fn test_購読事象_対象の定期印の旧転送形式を保つ() {
    // Given
    use wire::state_subscription_event::Event as WireEvent;
    let event = StateSubscriptionEvent::Item(
        SubscriptionTarget::Issues("/repo".into()).to_string(),
        Event::Bookmark(Version {
            epoch: "boot:1".into(),
            sequence: 10,
        }),
    );
    // When
    let actual = wire_event(event);
    // Then
    assert_eq!(
        actual,
        wire::StateSubscriptionEvent {
            subscription_id: SubscriptionTarget::Issues("/repo".into()).to_string(),
            event: Some(WireEvent::Bookmark(wire::Unit {})),
        }
    );
}

#[test]
fn test_購読payload_全種類を旧wire型とフィールドへ変換する() {
    use crate::domain::workspace_state::{
        value_objects::{WorkspaceLayoutState, WorkspaceTabsState},
        WorkspaceState,
    };
    use crate::usecase::agent_session::{
        AgentSessionHistoryCandidateDto, AgentSessionHistoryPageDto,
    };
    use crate::usecase::provider_dto::AgentSessionProviderDto;
    use crate::usecase::repository_dto::BranchDto;
    use crate::usecase::terminal_surface::application::TerminalSurfaceStreamItem;
    use crate::usecase::workflow::{
        WorkspaceNodeCapabilitiesDto, WorkspaceNodeContentDto, WorkspaceNodeDetailDto,
        WorkspaceSessionNodeContentDto,
    };
    use crate::usecase::workspace_tree::WorkspaceList;
    use wire::state_payload::Value as W;

    // Given
    let provider = wire::AgentSessionProviderDto {
        value: Some(wire::agent_session_provider_dto::Value::Codex as i32),
    };
    let values = vec![
        (
            StateValue::Terminal(
                TerminalSurfaceStreamItem::Output {
                    session_key: "terminal".into(),
                    data: Arc::from("output"),
                    sequence: 6,
                },
            ),
            W::Terminal(wire::TerminalEvent {
                item: Some(wire::terminal_event::Item::Output(wire::TerminalOutput {
                    data: "output".into(),
                    sequence: 6,
                })),
            }),
        ),
        (
            StateValue::TerminalStartupCommand("/repo".into()),
            W::TerminalStartupCommand(wire::ResultString { value: Some("/repo".into()) }),
        ),
        (
            StateValue::Workspaces(WorkspaceList {
                repositories: vec![],
            }),
            W::Workspaces(wire::WorkspaceListSnapshot {
                status: Some(wire::WorkspaceListStatus {
                    state: Some("empty".into()),
                    error: None,
                }),
                repositories: Some(wire::ListWorkspaceRepositoryList { items: vec![] }),
            }),
        ),
        (
            StateValue::NodeDetail(Some(WorkspaceNodeDetailDto {
                classification: "active".into(),
                node_execution_id: None,
                node_name: None,
                model: None,
                past_attempts: Vec::new(),
                execution_id: None,
                can_abort: false,
                process_presence: "live",
                worktree: None,
                id: "node".into(),
                title: "Main".into(),
                status: "running".into(),
                submit_received: false,
                stop_received: false,
                waiting_for: None,
                has_artifact: true,
                error_reason: None,
                capabilities: WorkspaceNodeCapabilitiesDto {
                    can_rename: true,
                    can_approve: false,
                    can_retry: false,
                    can_resume_session: true,
                },
                updated_at: 1.5,
                content: WorkspaceNodeContentDto::Session(WorkspaceSessionNodeContentDto {
                    session_id: Some("session".into()),
                }),
            })),
            W::NodeDetail(wire::NullableWorkspaceNodeDetailDto {
                value: Some(wire::WorkspaceNodeDetailDto {
                    classification: Some(wire::WorkspaceStatusClassification {
                        value: Some(wire::workspace_status_classification::Value::Active as i32),
                    }),
                    node_execution_id: None,
                    node_name: None,
                    model: None,
                    past_attempts: Vec::new(),
                    execution_id: None,
                    can_abort: Some(false),
                    process_presence: Some(wire::NodeProcessPresence {
                        value: Some(wire::node_process_presence::Value::Live as i32),
                    }),
                    worktree: None,
                    id: Some("node".into()),
                    title: Some("Main".into()),
                    status: Some(wire::WorkspaceNodeStatus {
                        value: Some(wire::workspace_node_status::Value::Running as i32),
                    }),
                    has_artifact: Some(true),
                    error_reason: None,
                    capabilities: Some(wire::WorkspaceNodeCapabilitiesDto {
                        can_rename: Some(true),
                        can_approve: Some(false),
                        can_retry: Some(false),
                        can_resume_session: Some(true),
                    }),
                    updated_at: Some(1.5),
                    content: Some(wire::WorkspaceNodeContentDto {
                        variant: Some(wire::workspace_node_content_dto::Variant::Session(
                            wire::WorkspaceSessionNodeContentDto {
                                session_id: Some("session".into()),
                            },
                        )),
                    }),
                }),
            }),
        ),
        (
            StateValue::SessionHistory(AgentSessionHistoryPageDto {
                items: vec![AgentSessionHistoryCandidateDto {
                    provider: AgentSessionProviderDto::Codex,
                    provider_session_id: "provider-session".into(),
                    label: "session".into(),
                    updated_at_ms: 11,
                }],
                has_more: true,
            }),
            W::SessionHistory(wire::AgentSessionHistoryPageDto {
                items: Some(wire::ListAgentSessionHistoryCandidateDto {
                    items: vec![wire::AgentSessionHistoryCandidateDto {
                        provider: Some(provider),
                        provider_session_id: Some("provider-session".into()),
                        label: Some("session".into()),
                    }],
                }),
                has_more: Some(true),
            }),
        ),
        (
            StateValue::Providers(vec![AgentSessionProviderDto::Codex]),
            W::Providers(wire::ListAgentSessionProviderDto {
                items: vec![provider],
            }),
        ),
        (
            StateValue::Branches(vec![BranchDto {
                name: "main".into(),
                is_remote: false,
            }]),
            W::Branches(wire::ListBranchDto {
                items: vec![wire::BranchDto {
                    name: Some("main".into()),
                }],
            }),
        ),
        (
            StateValue::BranchStatus(vec![(
                crate::domain::git::Branch::local("main"),
                true,
            )]),
            W::BranchStatus(wire::ListBranchStatus {
                items: vec![wire::BranchStatus {
                    name: Some("main".into()),
                    has_worktree: Some(true),
                }],
            }),
        ),
        (
            StateValue::Issues(crate::usecase::fetched::Fetched::ready(
                crate::usecase::git_host::git_host_usecase::IssueListing {
                    issues: vec![crate::domain::git_host::IssueInfo {
                        number: 13,
                        title: "Fix".into(),
                        state: "OPEN".into(),
                        url: "https://example.test/13".into(),
                        author: crate::domain::git_host::PrAuthor {
                            login: "author".into(),
                        },
                        created_at: "created".into(),
                        updated_at: "updated".into(),
                        labels: vec![crate::domain::git_host::IssueLabel {
                            name: "bug".into(),
                            color: "red".into(),
                        }],
                        assignees: vec![crate::domain::git_host::PrAuthor {
                            login: "assignee".into(),
                        }],
                        body: "body".into(),
                        milestone: Some(crate::domain::git_host::Milestone {
                            title: "next".into(),
                        }),
                    }],
                    options: crate::domain::git_host::IssueOptions::from_issues(&[
                        crate::domain::git_host::IssueInfo {
                            number: 13,
                            title: "Fix".into(),
                            state: "OPEN".into(),
                            url: "https://example.test/13".into(),
                            author: crate::domain::git_host::PrAuthor {
                                login: "author".into(),
                            },
                            created_at: "created".into(),
                            updated_at: "updated".into(),
                            labels: vec![crate::domain::git_host::IssueLabel {
                                name: "bug".into(),
                                color: "red".into(),
                            }],
                            assignees: vec![crate::domain::git_host::PrAuthor {
                                login: "assignee".into(),
                            }],
                            body: "body".into(),
                            milestone: Some(crate::domain::git_host::Milestone {
                                title: "next".into(),
                            }),
                        },
                    ]),
                },
            )),
            W::Issues(wire::IssuesSnapshot {
                labels: vec!["bug".into()],
                milestones: vec!["next".into()],
                read_error: None,
                issues: Some(wire::ListIssueInfoDto {
                    items: vec![wire::IssueInfoDto {
                        number: Some(13),
                        default_branch_name: Some("feat/issues/13".into()),
                        title: Some("Fix".into()),
                        state: Some("OPEN".into()),
                        url: Some("https://example.test/13".into()),
                        author: Some(wire::PrAuthorDto {
                            login: Some("author".into()),
                        }),
                        created_at: Some("created".into()),
                        updated_at: Some("updated".into()),
                        labels: Some(wire::ListIssueLabelDto {
                            items: vec![wire::IssueLabelDto {
                                name: Some("bug".into()),
                            }],
                        }),
                        body: Some("body".into()),
                    }],
                }),
            }),
        ),
        (
            StateValue::WorkspaceState(
                crate::domain::local_event::StreamVersion::new(7).unwrap(),
                Some(WorkspaceState {
                    panes: None,
                    version: 1,
                    tabs: Some(WorkspaceTabsState {
                        editors: vec![],
                    }),
                    layout: Some(WorkspaceLayoutState {
                        center_tab: "agent".into(),
                        active_view: "git".into(),
                        left_nav_collapsed: true,
                        right_collapsed: false,
                        right_bottom_collapsed: true,
                    }),
                }),
                vec![crate::usecase::workspace_tree::query_service::SessionNodeResolutionFailureDto {
                    reference_id: "session-node".into(),
                    failure: crate::domain::failure::WorkFailure {
                    kind: crate::domain::failure::Failure::Technical(
                        crate::domain::failure::TechnicalFailureNature::Other,
                    ),
                    message: "Session lookup failed".into(),
                }}],
                Some(crate::domain::workspace_state::WorkspaceStateError::Technical(
                    crate::domain::failure::TechnicalFailure {
                        nature: crate::domain::failure::TechnicalFailureNature::Other,
                        message: "Session tab migration write failed".into(),
                    },
                )),
            ),
            W::WorkspaceState(wire::NullableWorkspaceStateDto {
                value: Some(wire::WorkspaceStateDto {
                    read_failures: vec![wire::WorkspaceStateReadFailure { reference_id: "session-node".into(), message: "Session lookup failed".into() }],
                    pane_layout: None,
                    version: Some(1),
                    etag: Some("7".into()),
                    tabs: Some(wire::WorkspaceTabsStateDto {
                        editors: Some(wire::ListWorkspaceTabEntryDto { items: vec![] }),
                    }),
                    layout: Some(wire::WorkspaceLayoutStateDto {
                        center_tab: Some(wire::WorkspaceCenterTab {
                            value: Some(wire::workspace_center_tab::Value::Agent as i32),
                        }),
                        active_view: Some("git".into()),
                        left_nav_collapsed: Some(true),
                        right_collapsed: Some(false),
                        right_bottom_collapsed: Some(true),
                    }),
                }),
            }),
        ),


        (
            StateValue::Diagnostics(crate::usecase::workflow::diagnostic_dto::DiagnosticReport {
                items: vec![],
                workflow_summaries: Default::default(),
                facet_summaries: Default::default(),
                facet_usage: Default::default(),
            }),
            W::Diagnostics(wire::DiagnosticReport {
                items: Some(Default::default()),
                workflow_summaries: Some(Default::default()),
                facet_summaries: Some(Default::default()),
                facet_usage: Some(Default::default()),
            }),
        ),
    ];

    // When
    for (value, expected) in values {
        let actual = payload(&value).unwrap().value;
        // Then
        assert_eq!(actual, Some(expected), "{value:?}");
    }
}

#[test]
fn test_設定購読payload_entityの値を必須項目付きのprotoに写す() {
    use crate::domain::app_config::value_objects::{TelemetryConfig, WorkflowConfig};
    use crate::domain::app_config::AppConfig;
    use wire::state_payload::Value as W;

    for enabled in [false, true] {
        // Given
        let config = AppConfig::new(
            !enabled,
            enabled,
            vec![],
            String::new(),
            "printf '準備完了\\n'".into(),
            TelemetryConfig {
                crash_reporting: !enabled,
                performance_telemetry: enabled,
            },
            WorkflowConfig {
                approval_auto_approve: enabled,
            },
            Default::default(),
            Default::default(),
        );
        let workflow = config.workflow().clone();
        let command = config.terminal_startup_command().to_owned();

        // When
        let desktop = payload(&StateValue::DesktopSettings(config)).unwrap().value;
        let workflow = payload(&StateValue::WorkflowConfig(workflow))
            .unwrap()
            .value;
        let terminal = payload(&StateValue::TerminalStartupCommand(command.clone()))
            .unwrap()
            .value;

        // Then
        assert_eq!(
            desktop,
            Some(W::DesktopSettings(wire::DesktopSettings {
                start_minimized: Some(enabled),
                crash_reporting: Some(!enabled),
                performance_telemetry: Some(enabled),
                auto_launch: Some(!enabled),
            }))
        );
        assert_eq!(
            workflow,
            Some(W::WorkflowConfig(wire::WorkflowSection {
                approval_auto_approve: Some(enabled),
            }))
        );
        assert_eq!(
            terminal,
            Some(W::TerminalStartupCommand(wire::ResultString {
                value: Some(command),
            }))
        );
    }
}

#[test]
fn test_購読payload_設定とproviderの出力値を維持する() {
    use crate::usecase::agent_session::{
        ProviderAvailabilityItemDto, ProviderAvailabilitySnapshotDto,
    };
    use crate::usecase::notion::usecase::{
        NotionLabelPropertyDto, NotionPropertyMappingDto, NotionRepoConfigDto,
    };
    use crate::usecase::provider_dto::AgentSessionProviderDto;
    use crate::usecase::provider_lifecycle::ProviderHookHealthWarning;
    use wire::state_payload::Value as W;

    // Given
    let values = [
        (
            StateValue::NotionConfig(Some(NotionRepoConfigDto {
                api_token: "token".into(),
                database_id: "db".into(),
                property_mapping: NotionPropertyMappingDto {
                    title: "Name".into(),
                    labels: vec![NotionLabelPropertyDto {
                        name: "Status".into(),
                        property_type: "select".into(),
                    }],
                    branch_name: "Branch".into(),
                    branch_prefix: "feat/".into(),
                },
            })),
            "releash.client.v1.NullableNotionRepoConfigView",
            serde_json::json!({"api_token":"token","database_id":"db","property_mapping":{"title":"Name","labels":[{"name":"Status","property_type":"select"}],"branch_name":"Branch","branch_prefix":"feat/"}}),
        ),
        (
            StateValue::ProviderAvailability(ProviderAvailabilitySnapshotDto {
                providers: vec![ProviderAvailabilityItemDto {
                    provider: AgentSessionProviderDto::Codex,
                    display_name: "Codex".into(),
                    default_executable: "codex".into(),
                    configured_executable: None,
                    configuration_revision: 0,
                    effective_executable: "codex".into(),
                    available: false,
                    resolved_executable: None,
                    unavailable_reason: Some(crate::usecase::agent_session::ProviderUnavailableReasonDto::NotFound),
                }],
            }),
            "releash.client.v1.ProviderAvailabilitySnapshotResponse",
            serde_json::json!({"providers":[{"provider":"codex","displayName":"Codex","defaultExecutable":"codex","configuredExecutable":null,"effectiveExecutable":"codex","available":false,"resolvedExecutable":null,"unavailableReason":"not_found"}]}),
        ),
        (
            StateValue::WorkflowConfig(
                crate::domain::app_config::value_objects::WorkflowConfig {
                    approval_auto_approve: true,
                },
            ),
            "releash.client.v1.WorkflowSection",
            serde_json::json!({"approval_auto_approve":true}),
        ),
        (
            StateValue::ProviderHookHealth(crate::usecase::provider_lifecycle::ProviderHookHealthReadResult { warnings: vec![ProviderHookHealthWarning {
                provider: crate::domain::provider_lifecycle::ProviderKind::Claude,
                launch_id: "launch".into(),
                reason: crate::domain::provider_lifecycle::ProviderLifecycleUnavailableReason::LocalApiUnavailable,
            }], failures: vec![crate::usecase::provider_lifecycle::ProviderHookHealthFailureQueryError::Technical(crate::domain::failure::TechnicalFailure { nature: crate::domain::failure::TechnicalFailureNature::Transient, message: "unavailable".into() }), crate::usecase::provider_lifecycle::ProviderHookHealthFailureQueryError::Corrupt] }),
            "releash.client.v1.ProviderHookHealthSnapshot",
            serde_json::json!({"warnings":[{"provider":"claude","reason":"local_api_unavailable"}],"readErrors":["unavailable","Provider Hook health record is corrupt"]}),
        ),
    ];
    // When
    for (value, name, expected) in values {
        let actual = match payload(&value).unwrap().value.unwrap() {
            W::NotionConfig(value) => wire::from_message(name, &value),
            W::ProviderAvailability(value) => wire::from_message(name, &value),
            W::WorkflowConfig(value) => wire::from_message(name, &value),
            W::ProviderHookHealth(value) => wire::from_message(name, &value),
            other => panic!("unexpected payload: {other:?}"),
        }
        .unwrap();
        // Then
        assert_eq!(actual, expected, "{name}");
    }
}

#[test]
fn test_terminal購読payload_四種類の転送値を保つ() {
    use crate::usecase::terminal_surface::application::TerminalSurfaceStreamItem as Item;

    // Given
    let values = [
        (
            Item::Snapshot(
                crate::usecase::terminal_surface::application::TerminalSurfaceSnapshotDto {
                    shell_state: None,
                    session_key: "terminal".into(),
                    replay: "history".into(),
                    sequence: 7,
                    cols: 100,
                    rows: 30,
                    is_exited: true,
                    exit_code: Some(9),
                    label: Some("Shell".into()),
                },
            ),
            wire::terminal_event::Item::Snapshot(wire::TerminalSnapshot {
                status: None,
                processed_report_units:
                    crate::infrastructure::terminal::output_flow_control::OUTPUT_REPORT_UNITS as u32,
                replay: "history".into(),
                sequence: 7,
                cols: 100,
                rows: 30,
                is_exited: true,
                exit_code: Some(9),
            }),
        ),
        (
            Item::Output {
                session_key: "terminal".into(),
                data: Arc::from("output"),
                sequence: 8,
            },
            wire::terminal_event::Item::Output(wire::TerminalOutput {
                data: "output".into(),
                sequence: 8,
            }),
        ),
        (
            Item::Resize {
                session_key: "terminal".into(),
                cols: 120,
                rows: 40,
                sequence: 9,
            },
            wire::terminal_event::Item::Resize(wire::TerminalResize {
                cols: 120,
                rows: 40,
                sequence: 9,
            }),
        ),
        (
            Item::Exit {
                state: crate::domain::terminal_surface::TerminalShellState {
                    cwd: Some("/last".into()),
                    has_foreground_process: false,
                },
                session_key: "terminal".into(),
                exit_code: Some(42),
                sequence: 10,
            },
            wire::terminal_event::Item::Exit(wire::TerminalExit {
                status: Some(wire::TerminalStatus {
                    cwd: Some("/last".into()),
                    has_foreground_process: false,
                    sequence: 10,
                }),
                exit_code: Some(42),
                sequence: 10,
            }),
        ),
    ];

    // When
    for (item, expected) in values {
        let actual = payload(&StateValue::Terminal(item)).unwrap();
        // Then
        assert_eq!(
            actual,
            wire::StatePayload {
                value: Some(wire::state_payload::Value::Terminal(wire::TerminalEvent {
                    item: Some(expected)
                }))
            },
        );
    }
}

#[test]
fn test_購読失敗_snapshotの保持値をfailure事象へ変換する() {
    // Given
    let version = Version {
        epoch: "boot".into(),
        sequence: 1,
    };
    let value = Arc::new(PublishedState::Failure(wire::StateReadFailure {
        code: 13,
        message: "read failed".into(),
    }));
    // When
    let message = wire_event(StateSubscriptionEvent::Item(
        SubscriptionTarget::TerminalStartupCommand.to_string(),
        Event::Snapshot(version, value),
    ));
    // Then
    assert!(
        matches!(message.event, Some(wire::state_subscription_event::Event::Failure(failure)) if failure.code == 13 && failure.message == "read failed")
    );
}

#[test]
fn test_購読失敗_full変更の保持値をfailure事象へ変換する() {
    // Given
    let version = Version {
        epoch: "boot".into(),
        sequence: 1,
    };
    let value = Arc::new(PublishedState::Failure(wire::StateReadFailure {
        code: 13,
        message: "read failed".into(),
    }));
    // When
    let message = wire_event(StateSubscriptionEvent::Item(
        SubscriptionTarget::TerminalStartupCommand.to_string(),
        Event::Change(version, Delivery::Full, value),
    ));
    // Then
    assert!(
        matches!(message.event, Some(wire::state_subscription_event::Event::Failure(failure)) if failure.code == 13 && failure.message == "read failed")
    );
}

#[test]
fn test_購読失敗_terminal差分の保持値をfailure事象へ変換する() {
    // Given
    let version = Version {
        epoch: "boot".into(),
        sequence: 1,
    };
    let value = Arc::new(PublishedState::Failure(wire::StateReadFailure {
        code: 13,
        message: "read failed".into(),
    }));
    // When
    let message = wire_event(StateSubscriptionEvent::Item(
        SubscriptionTarget::TerminalStartupCommand.to_string(),
        Event::Change(version, Delivery::Delta, value),
    ));
    // Then
    assert!(
        matches!(message.event, Some(wire::state_subscription_event::Event::Failure(failure)) if failure.code == 13 && failure.message == "read failed")
    );
}

#[test]
fn test_notion購読配信_タスクの前の一覧と取得の失敗を共に送る() {
    // Given
    let page = crate::domain::notion::NotionTaskPage {
        tasks: vec![crate::domain::notion::NotionTask {
            id: "task-id".into(),
            title: "Task".into(),
            url: "https://notion.so/task-id".into(),
            labels: std::collections::HashMap::from([("Status".into(), vec!["Todo".into()])]),
            branch_name: "feat/task".into(),
            created_at: "created".into(),
            last_edited_at: "edited".into(),
        }],
        has_more: true,
        next_cursor: None,
    };
    let value = StateValue::NotionTasks(crate::usecase::fetched::Fetched {
        value: Some(page.clone()),
        error: Some(crate::usecase::notion::error::NotionUsecaseError::Notion(
            crate::domain::notion::NotionError::ApiError("offline".into()),
        )),
    });
    let expected = crate::adaptor::presenter::client::value(
        crate::adaptor::presenter::notion::NotionTaskPageView::from(page),
    )
    .unwrap();
    // When
    let result = payload(&value).unwrap();
    // Then
    let Some(wire::state_payload::Value::NotionTasks(snapshot)) = result.value else {
        panic!()
    };
    assert_eq!(snapshot.page, Some(expected));
    assert_eq!(snapshot.read_error.unwrap().code, 9);
}

#[test]
fn test_購読payload_提出済みoutputの全フィールドを保持する() {
    // Given
    let output = crate::usecase::workflow::WorkflowGetOutputResult::Submitted {
        contract: Some("review-result".into()),
        structured_output: serde_json::json!("accepted"),
        submitted_at: Some(123.5),
        request_id: Some("request-1".into()),
        timestamp: 124.5,
    };
    // When
    let message = payload(&StateValue::WorkflowOutput(Some(output))).unwrap();
    // Then
    let Some(wire::state_payload::Value::WorkflowOutput(output)) = message.value else {
        panic!("workflow output payload");
    };
    assert_eq!(
        output.value.unwrap().variant,
        Some(wire::workflow_output_view::Variant::Submitted(
            wire::WorkflowOutputViewSubmitted {
                contract: Some("review-result".into()),
                structured_output: Some(wire::WorkflowValue {
                    variant: Some(wire::workflow_value::Variant::StringValue(
                        wire::ResultString {
                            value: Some("accepted".into())
                        }
                    ))
                }),
                submitted_at: Some(123.5),
                request_id: Some("request-1".into()),
                timestamp: Some(124.5),
            }
        ))
    );
}

#[tokio::test]
async fn test_daemon状態payload_get_server_infoと同じ投影を使う() {
    // Given
    let identity = crate::domain::daemon::DaemonIdentity {
        daemon_id: "installation".into(),
        pid: 1,
        process_started_at: 1,
    };
    let repository = std::sync::Arc::new(
        crate::adaptor::gateway::daemon::InMemoryDaemonRepository::new(
            identity,
            "test".into(),
            1,
            Ok(crate::domain::installation::CliInstallation::ReadOnly),
        ),
    );
    let daemon = crate::usecase::daemon::DaemonUsecase::new(repository);
    let info = daemon.info().await;
    let result = crate::adaptor::presenter::daemon::server_info(info.clone())
        .cli_installation
        .unwrap();
    assert_eq!(result.status, wire::CliInstallationStatus::ReadOnly as i32);
    assert_eq!(
        result.reason,
        crate::domain::installation::CliInstallation::ReadOnly.reason()
    );
    // When
    let value = payload(&StateValue::DaemonInfo(info.clone())).unwrap();
    // Then
    assert_eq!(
        value.value,
        Some(wire::state_payload::Value::DaemonInfo(
            crate::adaptor::presenter::daemon::server_info(info)
        ))
    );
}

#[test]
fn test_sessionタブ購読_不在snapshotを専用payloadで配る() {
    // Given / When
    let snapshot = payload(&StateValue::SessionTab {
        session: None,
        node: None,
    })
    .unwrap();
    // Then
    assert_eq!(
        snapshot.value,
        Some(wire::state_payload::Value::SessionTab(
            wire::SessionTabSnapshot::default()
        ))
    );
}

#[test]
fn test_workspace購読payload_設定未保存のsession配置を転送する() {
    use crate::domain::workspace_state::WorkspaceState;
    // Given
    let mut state = WorkspaceState::default();
    state.add_session_tab("session", "node");
    // When
    let actual = payload(&StateValue::WorkspaceState(
        crate::domain::local_event::StreamVersion::zero(),
        Some(state.clone()),
        vec![],
        None,
    ))
    .unwrap();
    // Then
    let Some(wire::state_payload::Value::WorkspaceState(actual)) = actual.value else {
        panic!("workspace state payload")
    };
    let actual = actual.value.unwrap();
    assert!(actual.tabs.is_none());
    assert!(actual.layout.is_none());
    assert_eq!(WorkspaceState::try_from(actual).unwrap(), state);
}

#[test]
fn test_実行概要の部分成功_正常木と失敗idと原因を転送してattentionを示す() {
    // Given
    let tree = overview_tree(
        "/repo",
        None,
        WorkspaceNodeStatus::Completed,
        WorkspaceNodeStatus::Completed,
    );
    let failures = vec![(
        "broken".into(),
        crate::domain::workflow::WorkflowError::External("read failed".into()),
    )];
    // When / Then
    for trees in [vec![tree], vec![]] {
        let Some(wire::state_payload::Value::ExecutionOverview(value)) = payload(
            &StateValue::ExecutionOverview((trees.clone(), failures.clone())),
        )
        .unwrap()
        .value
        else {
            panic!("expected overview")
        };
        assert_eq!(value.executions.len(), trees.len());
        assert_eq!(
            value.status.unwrap().value,
            Some(wire::workspace_status_classification::Value::Attention as i32)
        );
        assert_eq!(value.failures["broken"].message, "read failed");
        assert_eq!(
            value.failures["broken"].code,
            connectrpc::ErrorCode::Internal.grpc_code() as i32
        );
    }
}

#[test]
fn test_絞込一覧_木を親ごとの順序に写し上限を伝える() {
    // Given
    use crate::domain::code::services::directory::GitStatus;
    use crate::domain::code::value_objects::file_list::{FileNode, FilteredFiles, WorktreeEntry};
    let leaf = FileNode {
        entry: WorktreeEntry {
            path: "src/file".into(),
            name: "file".into(),
            directory: false,
            ignored: true,
        },
        status: GitStatus::Modified,
        children: vec![],
    };
    let files = FilteredFiles {
        roots: vec![FileNode {
            entry: WorktreeEntry {
                path: "src".into(),
                name: "src".into(),
                directory: true,
                ignored: false,
            },
            status: GitStatus::Modified,
            children: vec![leaf],
        }],
        truncated: true,
    };
    // When
    let value = filtered_files(&files);
    // Then
    assert!(value.truncated);
    assert_eq!(value.directories.len(), 2);
    assert_eq!(value.directories[0].path, ".");
    assert_eq!(value.directories[0].entries[0].path, "src");
    assert_eq!(value.directories[1].path, "src");
    assert!(value.directories[1].entries[0].ignored);
    assert_eq!(value.directories[1].entries[0].status, "modified");
    assert!(filtered_files(&FilteredFiles {
        roots: vec![],
        truncated: false
    })
    .directories
    .is_empty());
}

#[test]
fn test_ファイル一覧配信_直下一覧と絞込結果が同じgit状態を返す() {
    use crate::domain::code::services::directory::GitStatus;
    use crate::domain::code::value_objects::file_list::{FileNode, FilteredFiles, WorktreeEntry};
    use crate::usecase::directory_query::DirectoryEntryDto;
    // Given
    for (status, expected) in [
        (GitStatus::None, "none"),
        (GitStatus::New, "new"),
        (GitStatus::Modified, "modified"),
        (GitStatus::Renamed, "renamed"),
        (GitStatus::Deleted, "deleted"),
        (GitStatus::Conflicted, "conflicted"),
    ] {
        let entry = WorktreeEntry {
            path: "file.rs".into(),
            name: "file.rs".into(),
            directory: false,
            ignored: true,
        };
        let directory = StateValue::Directory(vec![DirectoryEntryDto {
            path: entry.path.clone(),
            name: entry.name.clone(),
            directory: entry.directory,
            ignored: entry.ignored,
            status,
        }]);
        let files = StateValue::FilteredFiles(FilteredFiles {
            roots: vec![FileNode {
                entry,
                status,
                children: vec![],
            }],
            truncated: true,
        });
        // When
        let wire::state_payload::Value::Directory(directory) =
            payload(&directory).unwrap().value.unwrap()
        else {
            panic!("directory payload")
        };
        let wire::state_payload::Value::FilteredFiles(files) =
            payload(&files).unwrap().value.unwrap()
        else {
            panic!("filtered files payload")
        };
        // Then
        assert_eq!(directory.entries[0].status, expected);
        assert_eq!(files.directories[0].path, ".");
        assert_eq!(files.directories[0].entries, directory.entries);
        assert!(files.truncated);
    }
}

#[test]
fn test_comment画像配信_画像データと媒体型をconnectへ渡す() {
    // Given
    let image = crate::usecase::comment_image::CommentImageDto {
        data: vec![1, 2, 3],
        media_type: "image/png".into(),
    };
    // When
    let value = payload(&StateValue::CommentImage(image.clone())).unwrap();
    // Then
    let Some(wire::state_payload::Value::CommentImage(value)) = value.value else {
        panic!("image payload missing")
    };
    assert_eq!(value.data, image.data);
    assert_eq!(value.media_type, image.media_type);
}
