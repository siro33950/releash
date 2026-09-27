use super::*;
use crate::infrastructure::state_subscription::Version;
use crate::usecase::state_subscription::SubscriptionTarget;
use std::sync::Arc;

fn wire_event(event: StateSubscriptionEvent) -> wire::StateSubscriptionEvent {
    crate::adaptor::presenter::connect_wire::to_wire(&super::event(event).unwrap()).unwrap()
}

#[test]
fn test_購読事象_対象名と日本語を含む引数を分離して配信する() {
    let target = SubscriptionTarget::BranchBase("/作業:repo".into(), "feature".into());
    let message = wire_event(StateSubscriptionEvent::Item(
        target.to_string(),
        Event::Snapshot(
            Version {
                epoch: "boot:1".into(),
                sequence: 7,
            },
            Arc::new(payload(&StateValue::BranchBase(Some("main".into()))).unwrap()),
        ),
    ));
    assert_eq!(
        message,
        wire::StateSubscriptionEvent {
            target: "branch-base".into(),
            args: vec!["/作業:repo".into(), "feature".into()],
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
    assert_eq!(
        SubscriptionTarget::from_parts(
            &message.target,
            &message.args.iter().map(String::as_str).collect::<Vec<_>>()
        ),
        Ok(target)
    );
}

#[test]
fn test_購読事象_準備変更と定期印の旧転送形式を保つ() {
    use wire::state_subscription_event::Event as WireEvent;

    assert_eq!(
        wire_event(StateSubscriptionEvent::Ready),
        wire::StateSubscriptionEvent {
            target: String::new(),
            args: vec![],
            version: None,
            event: Some(WireEvent::Ready(wire::Unit {})),
        }
    );

    for (delivery, delta, sequence) in [(Delivery::Full, false, 8), (Delivery::Delta, true, 9)] {
        assert_eq!(
            wire_event(StateSubscriptionEvent::Item(
                SubscriptionTarget::RepositoryPaths.to_string(),
                Event::Change(
                    Version {
                        epoch: "boot:1".into(),
                        sequence,
                    },
                    delivery,
                    Arc::new(payload(&StateValue::RepositoryPaths(vec!["/repo".into()])).unwrap()),
                ),
            )),
            wire::StateSubscriptionEvent {
                target: "repository-paths".into(),
                args: vec![],
                version: Some(wire::StateVersion {
                    epoch: "boot:1".into(),
                    sequence,
                }),
                event: Some(WireEvent::Change(wire::StateChange {
                    delta,
                    payload: Some(wire::StatePayload {
                        value: Some(wire::state_payload::Value::RepositoryPaths(
                            wire::Liststring {
                                items: vec!["/repo".into()],
                            },
                        )),
                    }),
                })),
            }
        );
    }

    assert_eq!(
        wire_event(StateSubscriptionEvent::Item(
            SubscriptionTarget::Issues("/repo".into()).to_string(),
            Event::Bookmark(Version {
                epoch: "boot:1".into(),
                sequence: 10,
            }),
        )),
        wire::StateSubscriptionEvent {
            target: "issues".into(),
            args: vec!["/repo".into()],
            version: Some(wire::StateVersion {
                epoch: "boot:1".into(),
                sequence: 10,
            }),
            event: Some(WireEvent::Bookmark(wire::Unit {})),
        }
    );
}

#[test]
fn test_購読payload_全種類を旧wire型とフィールドへ変換する() {
    use crate::domain::failure::{BusinessFailure, Failure};
    use crate::domain::failure_records::FailureRecord;
    use crate::usecase::agent_session::{
        AgentSessionHistoryCandidateDto, AgentSessionHistoryPageDto, AgentSessionItemDto,
        AgentSessionLifecycleDto, AgentSessionOperationsDto, AgentSessionProviderDto,
        AgentSessionTreeLocationDto,
    };
    use crate::usecase::git_host::dto::{IssueInfoDto, IssueLabelDto, MilestoneDto, PrAuthorDto};
    use crate::usecase::repository_dto::{BranchDto, WorktreeDisplayGroupsDto, WorktreeEntryDto};
    use crate::usecase::repository_state::snapshot::RepositoryBranchCardsSnapshotDto;
    use crate::usecase::terminal_surface::application::TerminalSurfaceStreamItem;
    use crate::usecase::work_queue::{FailureObservation, FailurePage};
    use crate::usecase::workflow::{
        WorkspaceNodeCapabilitiesDto, WorkspaceNodeContentDto, WorkspaceNodeDetailDto,
        WorkspaceSelectionReconciliationDto, WorkspaceSessionNodeContentDto,
        WorkspaceTreeSelectionSnapshotDto, WorkspaceTreeSnapshotDto,
    };
    use crate::usecase::workspace_state::dto::{
        WorkspaceLayoutStateDto, WorkspaceStateDto, WorkspaceTabsStateDto,
    };
    use crate::usecase::workspace_tree::{WorkspaceListSnapshotDto, WorkspaceListStatusDto};
    use wire::state_payload::Value as W;

    let provider = wire::AgentSessionProviderDto {
        value: Some(wire::agent_session_provider_dto::Value::Codex as i32),
    };
    let values = vec![
        (
            StateValue::Failures(FailurePage {
                items: vec![FailureObservation {
                    record: FailureRecord {
                        operation: "run".into(),
                        target: "node".into(),
                        kind: Failure::Business(BusinessFailure::Other),
                        message: "failed".into(),
                        active: true,
                        count: 2,
                        first_observed_ms: 3,
                        last_observed_ms: 4,
                    },
                    requires_attention: true,
                }],
                next_offset: Some(5),
                requires_attention: true,
            }),
            W::Failures(wire::FailureRecords {
                items: vec![wire::FailureRecord {
                    operation: Some("run".into()),
                    target: Some("node".into()),
                    classification: Some("BusinessFailure".into()),
                    message: Some("failed".into()),
                    count: Some(2),
                    first_observed_ms: Some(3),
                    last_observed_ms: Some(4),
                    requires_attention: Some(true),
                }],
                next_offset: Some(5),
                requires_attention: Some(true),
            }),
        ),
        (
            StateValue::Terminal(TerminalSurfaceStreamItem::Output {
                session_key: "terminal".into(),
                data: Arc::from("output"),
                sequence: 6,
            }),
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
            StateValue::Workspaces(WorkspaceListSnapshotDto {
                generation: 7,
                status: WorkspaceListStatusDto {
                    loaded: true,
                    state: "ready",
                    error: None,
                },
                repositories: vec![],
            }),
            W::Workspaces(wire::WorkspaceListSnapshotDto {
                generation: Some(7),
                status: Some(wire::WorkspaceListStatusDto {
                    loaded: Some(true),
                    state: Some("ready".into()),
                    error: None,
                }),
                repositories: Some(wire::ListWorkspaceRepositoryListDto { items: vec![] }),
            }),
        ),
        (
            StateValue::Selection(WorkspaceTreeSelectionSnapshotDto {
                snapshot: WorkspaceTreeSnapshotDto {
                    nodes: vec![],
                    archived_sessions: vec![],
                    preferred_node_id: Some("node".into()),
                },
                reconciliation: WorkspaceSelectionReconciliationDto {
                    selection_in_snapshot: true,
                },
            }),
            W::Selection(wire::WorkspaceTreeSelectionSnapshotDto {
                snapshot: Some(wire::WorkspaceTreeSnapshotDto {
                    nodes: Some(wire::ListWorkspaceTreeItemDto { items: vec![] }),
                    archived_sessions: Some(wire::ListAgentSessionItemDto { items: vec![] }),
                    preferred_node_id: Some("node".into()),
                }),
                reconciliation: Some(wire::WorkspaceSelectionReconciliationDto {
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
                status_classification: "active".into(),
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
                    status_classification: Some(wire::WorkspaceStatusClassification {
                        value: Some(wire::workspace_status_classification::Value::Active as i32),
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
                }),
            }),
        ),
        (
            StateValue::SessionNode(Some("node".into())),
            W::SessionNode(wire::Nullablestring {
                value: Some("node".into()),
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
            StateValue::BranchStatus(RepositoryBranchCardsSnapshotDto {
                version: 12,
                stale: true,
                loading: false,
                branches: vec![],
                worktree_display_groups: WorktreeDisplayGroupsDto::default(),
            }),
            W::BranchStatus(wire::RepositoryBranchCardsSnapshotDto {
                version: Some(12),
                stale: Some(true),
                loading: Some(false),
                branches: Some(wire::ListBranchCardDto { items: vec![] }),
                worktree_display_groups: Some(wire::WorktreeDisplayGroupsDto {
                    working_areas: Some(wire::ListBranchCardDto { items: vec![] }),
                }),
            }),
        ),
        (
            StateValue::CurrentBranch("feature".into()),
            W::CurrentBranch(wire::ResultString {
                value: Some("feature".into()),
            }),
        ),
        (
            StateValue::Issues(vec![IssueInfoDto {
                number: 13,
                default_branch_name: "issue-13".into(),
                title: "Fix".into(),
                state: "OPEN".into(),
                url: "https://example.test/13".into(),
                author: PrAuthorDto {
                    login: "author".into(),
                },
                created_at: "created".into(),
                updated_at: "updated".into(),
                labels: vec![IssueLabelDto {
                    name: "bug".into(),
                    color: "red".into(),
                }],
                assignees: vec![PrAuthorDto {
                    login: "assignee".into(),
                }],
                body: "body".into(),
                milestone: Some(MilestoneDto {
                    title: "next".into(),
                }),
            }]),
            W::Issues(wire::ListIssueInfoDto {
                items: vec![wire::IssueInfoDto {
                    number: Some(13),
                    default_branch_name: Some("issue-13".into()),
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
        ),
        (
            StateValue::Worktrees(vec![WorktreeEntryDto {
                name: "main".into(),
                path: "/repo".into(),
                branch: "main".into(),
                is_main: true,
                is_locked: false,
                dirty_count: 2,
                base_branch: Some("base".into()),
            }]),
            W::Worktrees(wire::ListWorktreeEntryDto {
                items: vec![wire::WorktreeEntryDto {
                    name: Some("main".into()),
                    path: Some("/repo".into()),
                    branch: Some("main".into()),
                    is_main: Some(true),
                    is_locked: Some(false),
                    dirty_count: Some(2),
                    base_branch: Some("base".into()),
                }],
            }),
        ),
        (
            StateValue::RepositoryRoot("/repo".into()),
            W::RepositoryRoot(wire::ResultString {
                value: Some("/repo".into()),
            }),
        ),
        (
            StateValue::StartupRepository("/startup".into()),
            W::StartupRepository(wire::ResultString {
                value: Some("/startup".into()),
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
    ];

    for (value, expected) in values {
        assert_eq!(payload(&value).unwrap().value, Some(expected), "{value:?}");
    }
}
