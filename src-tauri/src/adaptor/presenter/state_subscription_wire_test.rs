use super::*;
use crate::infrastructure::state_subscription::Version;
use crate::usecase::state_subscription::SubscriptionTarget;
use std::sync::Arc;

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
                payload(&StateValue::BranchBase(Some("main".into()))).unwrap(),
            )),
        ),
        // When
    ));
    // Then
    assert_eq!(
        message,
        wire::StateSubscriptionEvent {
            subscription_id: target.into(),
            version: Some(wire::StateVersion {
                epoch: "boot:1".into(),
                sequence: 7,
            }),
            event: Some(wire::state_subscription_event::Event::Snapshot(
                wire::StatePayload {
                    value: Some(wire::state_payload::Value::BranchBase(
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
            version: None,
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
            version: None,
            event: Some(WireEvent::Bookmark(wire::Unit {})),
        }
    );
}
#[test]
fn test_購読事象_full変更の旧転送形式を保つ() {
    // Given
    use wire::state_subscription_event::Event as WireEvent;
    let event = StateSubscriptionEvent::Item(
        SubscriptionTarget::RepositoryPaths.to_string(),
        Event::Change(
            Version {
                epoch: "boot:1".into(),
                sequence: 8,
            },
            Delivery::Full,
            Arc::new(PublishedState::from(
                payload(&StateValue::RepositoryPaths(vec!["/repo".into()])).unwrap(),
            )),
        ),
    );
    // When
    let actual = wire_event(event);
    // Then
    assert_eq!(
        actual,
        wire::StateSubscriptionEvent {
            subscription_id: "repository-paths".into(),
            version: Some(wire::StateVersion {
                epoch: "boot:1".into(),
                sequence: 8
            }),
            event: Some(WireEvent::Change(wire::StateChange {
                delta: false,
                payload: Some(wire::StatePayload {
                    value: Some(wire::state_payload::Value::RepositoryPaths(
                        wire::Liststring {
                            items: vec!["/repo".into()]
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
        SubscriptionTarget::RepositoryPaths.to_string(),
        Event::Change(
            Version {
                epoch: "boot:1".into(),
                sequence: 9,
            },
            Delivery::Delta,
            Arc::new(PublishedState::from(
                payload(&StateValue::RepositoryPaths(vec!["/repo".into()])).unwrap(),
            )),
        ),
    );
    // When
    let actual = wire_event(event);
    // Then
    assert_eq!(
        actual,
        wire::StateSubscriptionEvent {
            subscription_id: "repository-paths".into(),
            version: Some(wire::StateVersion {
                epoch: "boot:1".into(),
                sequence: 9
            }),
            event: Some(WireEvent::Change(wire::StateChange {
                delta: true,
                payload: Some(wire::StatePayload {
                    value: Some(wire::state_payload::Value::RepositoryPaths(
                        wire::Liststring {
                            items: vec!["/repo".into()]
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
            version: Some(wire::StateVersion {
                epoch: "boot:1".into(),
                sequence: 10
            }),
            event: Some(WireEvent::Bookmark(wire::Unit {})),
        }
    );
}

#[test]
fn test_購読payload_全種類を旧wire型とフィールドへ変換する() {
    use crate::usecase::agent_session::{
        AgentSessionHistoryCandidateDto, AgentSessionHistoryPageDto, AgentSessionItemDto,
        AgentSessionLifecycleDto, AgentSessionOperationsDto, AgentSessionTreeLocationDto,
    };
    use crate::usecase::provider_dto::AgentSessionProviderDto;
    use crate::usecase::repository_dto::{BranchDto, WorktreeEntryDto};
    use crate::usecase::terminal_surface::application::TerminalSurfaceStreamItem;
    use crate::usecase::workflow::{
        WorkspaceNodeCapabilitiesDto, WorkspaceNodeContentDto, WorkspaceNodeDetailDto,
        WorkspaceSessionNodeContentDto,
    };
    use crate::usecase::workspace_state::dto::{
        WorkspaceLayoutStateDto, WorkspaceStateDto, WorkspaceTabsStateDto,
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
                }
                .into(),
            ),
            W::Terminal(wire::TerminalEvent {
                item: Some(wire::terminal_event::Item::Output(wire::TerminalOutput {
                    session_key: "terminal".into(),
                    data: "output".into(),
                    sequence: 6,
                })),
            }),
        ),
        (
            StateValue::RepositoryPaths(vec!["/repo".into()]),
            W::RepositoryPaths(wire::Liststring {
                items: vec!["/repo".into()],
            }),
        ),
        (
            StateValue::Workspaces(WorkspaceList {
                repositories: vec![],
            }),
            W::Workspaces(wire::WorkspaceListSnapshot {
                status: Some(wire::WorkspaceListStatus {
                    loaded: Some(true),
                    state: Some("empty".into()),
                    error: None,
                }),
                repositories: Some(wire::ListWorkspaceRepositoryList { items: vec![] }),
            }),
        ),
        (
            StateValue::Selection(
                crate::domain::workspace_tree::WorkspaceTree::empty("/repo"),
                true,
            ),
            W::Selection(wire::WorkspaceTreeSelectionSnapshot {
                snapshot: Some(wire::WorkspaceTreeSnapshot {
                    nodes: Some(wire::ListWorkspaceTreeItem { items: vec![] }),
                    archived_sessions: Some(wire::ListAgentSessionItemDto { items: vec![] }),
                    preferred_node_id: None,
                }),
                reconciliation: Some(wire::WorkspaceSelectionReconciliation {
                    selection_in_snapshot: Some(true),
                }),
            }),
        ),
        (
            StateValue::NodeDetail(Some(WorkspaceNodeDetailDto {
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
                    process_presence: Some(wire::NodeProcessPresence {
                        value: Some(wire::node_process_presence::Value::Live as i32),
                    }),
                    worktree: None,
                    id: Some("node".into()),
                    title: Some("Main".into()),
                    status: Some(wire::WorkspaceNodeStatus {
                        value: Some(wire::workspace_node_status::Value::Running as i32),
                    }),
                    submit_received: Some(false),
                    stop_received: Some(false),
                    waiting_for: None,
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
            StateValue::AgentSession(Some(AgentSessionItemDto {
                id: "session".into(),
                workspace_identity: "/repo".into(),
                worktree_path: "/repo/worktree".into(),
                workspace_worktree_path: "/repo/worktree".into(),
                provider: AgentSessionProviderDto::Codex,
                tree_location: AgentSessionTreeLocationDto {
                    tree_id: "tree".into(),
                    node_execution_id: "node".into(),
                },
                lifecycle: AgentSessionLifecycleDto::Open,
                provider_session_id: Some("provider-session".into()),
                transcript_ref: None,
                operations: AgentSessionOperationsDto {
                    can_archive: true,
                    can_restore: false,
                    can_delete: false,
                },
                last_exit_abnormal: false,
                terminal_presence: None,
            })),
            W::AgentSession(wire::NullableAgentSessionItemDto {
                value: Some(wire::AgentSessionItemDto {
                    id: Some("session".into()),
                    workspace_identity: Some("/repo".into()),
                    worktree_path: Some("/repo/worktree".into()),
                    workspace_worktree_path: Some("/repo/worktree".into()),
                    provider: Some(provider.clone()),
                    tree_location: Some(wire::AgentSessionTreeLocationDto {
                        tree_id: Some("tree".into()),
                        node_execution_id: Some("node".into()),
                    }),
                    lifecycle: Some(wire::AgentSessionLifecycleDto {
                        value: Some(wire::agent_session_lifecycle_dto::Value::Open as i32),
                    }),
                    provider_session_id: Some("provider-session".into()),
                    transcript_ref: None,
                    operations: Some(wire::AgentSessionOperationsDto {
                        can_archive: Some(true),
                        can_restore: Some(false),
                        can_delete: Some(false),
                    }),
                    last_exit_abnormal: Some(false),
                    terminal_presence: None,
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
                        provider: Some(provider.clone()),
                        provider_session_id: Some("provider-session".into()),
                        label: Some("session".into()),
                        updated_at_ms: Some(11),
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
                    is_remote: Some(false),
                }],
            }),
        ),
        (
            StateValue::BranchBase(Some("main".into())),
            W::BranchBase(wire::Nullablestring {
                value: Some("main".into()),
            }),
        ),
        (
            StateValue::BranchStatus(vec![(
                crate::domain::repository::Branch::local("main"),
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
            StateValue::CurrentBranch("feature".into()),
            W::CurrentBranch(wire::ResultString {
                value: Some("feature".into()),
            }),
        ),
        (
            StateValue::Issues(crate::usecase::fetched::Fetched::ready(vec![
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
            ])),
            W::Issues(wire::IssuesSnapshot {
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
                                color: Some("red".into()),
                            }],
                        }),
                        assignees: Some(wire::ListPrAuthorDto {
                            items: vec![wire::PrAuthorDto {
                                login: Some("assignee".into()),
                            }],
                        }),
                        body: Some("body".into()),
                        milestone: Some(wire::MilestoneDto {
                            title: Some("next".into()),
                        }),
                    }],
                }),
            }),
        ),
        (
            StateValue::Worktrees(vec![WorktreeEntryDto {
                name: "main".into(),
                path: "/repo".into(),
                branch: "main".into(),
                is_main: true,
                is_locked: false,
            }]),
            W::Worktrees(wire::ListWorktreeEntryDto {
                items: vec![wire::WorktreeEntryDto {
                    name: Some("main".into()),
                    path: Some("/repo".into()),
                    branch: Some("main".into()),
                    is_main: Some(true),
                    is_locked: Some(false),
                }],
            }),
        ),
        (
            StateValue::StartupRepository(Some(crate::usecase::repository_dto::StartupWorktree {
                path: "/startup".into(),
                branch: "main".into(),
                repository_name: "startup".into(),
            })),
            W::StartupRepository(wire::NullableStartupWorktree {
                value: Some(wire::StartupWorktree {
                    path: Some("/startup".into()),
                    branch: Some("main".into()),
                    repository_name: Some("startup".into()),
                }),
            }),
        ),
        (
            StateValue::WorkspaceState(Some(WorkspaceStateDto {
                version: 1,
                tabs: WorkspaceTabsStateDto {
                    editors: vec![],
                    active_editor_path: Some("/file".into()),
                },
                layout: WorkspaceLayoutStateDto {
                    center_tab: "agent".into(),
                    active_view: "git".into(),
                    left_nav_collapsed: true,
                    right_collapsed: false,
                    right_bottom_collapsed: true,
                    right_bottom_active_tab: None,
                    selected_diff_file: None,
                },
            })),
            W::WorkspaceState(wire::NullableWorkspaceStateDto {
                value: Some(wire::WorkspaceStateDto {
                    version: Some(1),
                    tabs: Some(wire::WorkspaceTabsStateDto {
                        editors: Some(wire::ListWorkspaceTabEntryDto { items: vec![] }),
                        active_editor_path: Some("/file".into()),
                    }),
                    layout: Some(wire::WorkspaceLayoutStateDto {
                        center_tab: Some(wire::WorkspaceCenterTab {
                            value: Some(wire::workspace_center_tab::Value::Agent as i32),
                        }),
                        active_view: Some("git".into()),
                        left_nav_collapsed: Some(true),
                        right_collapsed: Some(false),
                        right_bottom_collapsed: Some(true),
                        right_bottom_active_tab: None,
                        selected_diff_file: None,
                        review_collapsed: None,
                        diff_only_mode: None,
                    }),
                }),
            }),
        ),
        (
            StateValue::Workflows(vec![crate::usecase::workflow::dto::WorkflowSummaryDto {
                failure: None,
                name: "dev".into(),
                description: "develop".into(),
                builtin: false,
                is_running: true,
                source_format: crate::usecase::workflow::dto::WorkflowSourceFormatDto::Lua,
            }]),
            W::Workflows(wire::ListWorkflowSummaryDto {
                items: vec![wire::WorkflowSummaryDto {
                    read_error: None,
                    name: Some("dev".into()),
                    description: Some("develop".into()),
                    builtin: Some(false),
                    is_running: Some(true),
                    source_format: Some(wire::WorkflowSourceFormat {
                        value: Some(wire::workflow_source_format::Value::Lua as i32),
                    }),
                }],
            }),
        ),
        (
            StateValue::Workflow(None),
            W::Workflow(wire::NullableWorkflowDto { value: None }),
        ),
        (
            StateValue::Workflow(Some(crate::usecase::workflow::dto::WorkflowDto {
                name: "dev".into(),
                description: "develop".into(),
                builtin: true,
                source_format: crate::usecase::workflow::dto::WorkflowSourceFormatDto::Yaml,
                schemas: Default::default(),
                nodes: vec![],
            })),
            W::Workflow(wire::NullableWorkflowDto {
                value: Some(wire::WorkflowDto {
                    name: Some("dev".into()),
                    description: Some("develop".into()),
                    builtin: Some(true),
                    source_format: Some(wire::WorkflowSourceFormat {
                        value: Some(wire::workflow_source_format::Value::Yaml as i32),
                    }),
                    schemas: Some(Default::default()),
                    nodes: Some(Default::default()),
                }),
            }),
        ),
        (
            StateValue::WorkflowSource(Some("name: dev".into())),
            W::WorkflowSource(wire::Nullablestring {
                value: Some("name: dev".into()),
            }),
        ),
        (
            StateValue::Facets(vec![crate::usecase::workflow::dto::FacetSummaryDto {
                key: "guide".into(),
                kind: "policy".into(),
                description: "guide".into(),
                builtin: false,
            }]),
            W::Facets(wire::ListFacetSummaryDto {
                items: vec![wire::FacetSummaryDto {
                    key: Some("guide".into()),
                    kind: Some("policy".into()),
                    description: Some("guide".into()),
                    builtin: Some(false),
                }],
            }),
        ),
        (
            StateValue::Facet("# guide".into()),
            W::Facet(wire::ResultString {
                value: Some("# guide".into()),
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
fn test_購読payload_review対象をreview欄へ変換する() {
    use crate::usecase::code_dto::{ReviewBinaryDto, ReviewFileViewDto, ReviewSnapshotDto};
    use wire::state_payload::Value as W;
    // Given
    let snapshot = ReviewSnapshotDto {
        version: 3,
        stale: false,
        loading: true,
        base: "head".into(),
        files: vec![],
        staged_files: vec![],
        changed_files: vec![],
        diff_stats: vec![],
        tree: vec![],
        staged_tree: vec![],
        changes_tree: vec![],
        staged_file_count: 0,
        changes_file_count: 0,
    };
    let view = ReviewFileViewDto::Binary(ReviewBinaryDto {
        version: 3,
        stale: false,
        file_id: "a.bin".into(),
        path: "a.bin".into(),
        original_size: None,
        modified_size: Some(4),
    });
    // When
    let snapshot_value = payload(&StateValue::ReviewSnapshot(snapshot))
        .unwrap()
        .value;
    let view_value = payload(&StateValue::ReviewFileView(view)).unwrap().value;
    let threads_value = payload(&StateValue::ReviewThreads(vec![])).unwrap().value;
    // Then
    assert!(matches!(
        snapshot_value,
        Some(W::ReviewSnapshot(value)) if value.version == Some(3) && value.loading == Some(true)
    ));
    assert!(matches!(
        view_value,
        Some(W::ReviewFileView(wire::ReviewFileViewDto {
            variant: Some(wire::review_file_view_dto::Variant::Binary(binary)),
        })) if binary.modified_size == Some(4)
    ));
    assert!(matches!(
        threads_value,
        Some(W::ReviewThreads(list)) if list.items.is_empty()
    ));
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
            serde_json::json!({"providers":[{"provider":"codex","displayName":"Codex","defaultExecutable":"codex","configuredExecutable":null,"configurationRevision":0,"effectiveExecutable":"codex","available":false,"resolvedExecutable":null,"unavailableReason":"not_found"}]}),
        ),
        (
            StateValue::WorkflowConfig(
                crate::usecase::app_config::query_service::WorkflowConfigDto {
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
            serde_json::json!({"warnings":[{"provider":"claude","launchId":"launch","reason":"local_api_unavailable"}],"readErrors":["unavailable","Provider Hook health record is corrupt"]}),
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
                session_key: "terminal".into(),
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
                session_key: "terminal".into(),
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
                session_key: "terminal".into(),
                cols: 120,
                rows: 40,
                sequence: 9,
            }),
        ),
        (
            Item::Exit {
                session_key: "terminal".into(),
                exit_code: Some(42),
                sequence: 10,
            },
            wire::terminal_event::Item::Exit(wire::TerminalExit {
                session_key: "terminal".into(),
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
        SubscriptionTarget::RepositoryPaths.to_string(),
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
        SubscriptionTarget::RepositoryPaths.to_string(),
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
        SubscriptionTarget::RepositoryPaths.to_string(),
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
fn test_notion購読配信_ラベルの前の選択肢と取得の失敗を共に送る() {
    // Given
    let options = vec![crate::domain::notion::NotionLabelOption {
        property_name: "Status".into(),
        property_type: "status".into(),
        options: vec!["Todo".into(), "Done".into()],
        option_ids: vec!["todo-id".into(), "done-id".into()],
    }];
    let value = StateValue::NotionLabelOptions(crate::usecase::fetched::Fetched {
        value: Some(options.clone()),
        error: Some(crate::usecase::notion::error::NotionUsecaseError::Notion(
            crate::domain::notion::NotionError::ApiError("offline".into()),
        )),
    });
    let expected = crate::adaptor::presenter::client::value(
        options
            .into_iter()
            .map(crate::adaptor::presenter::notion::NotionLabelOptionView::from)
            .collect::<Vec<_>>(),
    )
    .unwrap();
    // When
    let result = payload(&value).unwrap();
    // Then
    let Some(wire::state_payload::Value::NotionLabelOptions(snapshot)) = result.value else {
        panic!()
    };
    assert_eq!(snapshot.options, Some(expected));
    assert_eq!(snapshot.read_error.unwrap().code, 9);
}

#[test]
fn test_notion購読配信_設定不足の失敗に設定不足の印を付ける() {
    // Given
    let error = crate::usecase::notion::error::NotionUsecaseError::ConfigNotFound;
    let values = [
        StateValue::NotionTasks(crate::usecase::fetched::Fetched {
            value: None,
            error: Some(error.clone()),
        }),
        StateValue::NotionLabelOptions(crate::usecase::fetched::Fetched {
            value: None,
            error: Some(error),
        }),
    ];
    // When
    let results: Vec<_> = values.iter().map(|value| payload(value).unwrap()).collect();
    // Then
    for result in results {
        let failure = match result.value.unwrap() {
            wire::state_payload::Value::NotionTasks(snapshot) => snapshot.read_error.unwrap(),
            wire::state_payload::Value::NotionLabelOptions(snapshot) => {
                snapshot.read_error.unwrap()
            }
            _ => panic!(),
        };
        assert_eq!(failure.code, 9);
        assert_eq!(failure.config_missing, Some(true));
    }
}

#[test]
fn test_notion購読配信_notionの取得の失敗に設定不足の印を付けない() {
    // Given
    let error = crate::usecase::notion::error::NotionUsecaseError::Notion(
        crate::domain::notion::NotionError::ApiError("offline".into()),
    );
    let values = [
        StateValue::NotionTasks(crate::usecase::fetched::Fetched {
            value: None,
            error: Some(error.clone()),
        }),
        StateValue::NotionLabelOptions(crate::usecase::fetched::Fetched {
            value: None,
            error: Some(error),
        }),
    ];
    // When
    let results: Vec<_> = values.iter().map(|value| payload(value).unwrap()).collect();
    // Then
    for result in results {
        let failure = match result.value.unwrap() {
            wire::state_payload::Value::NotionTasks(snapshot) => snapshot.read_error.unwrap(),
            wire::state_payload::Value::NotionLabelOptions(snapshot) => {
                snapshot.read_error.unwrap()
            }
            _ => panic!(),
        };
        assert_eq!(failure.code, 9);
        assert_eq!(failure.config_missing, Some(false));
    }
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
    let repository = crate::adaptor::gateway::daemon::serving();
    let daemon = crate::usecase::daemon::DaemonUsecase::new(repository);
    let info = daemon.info().await;
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
