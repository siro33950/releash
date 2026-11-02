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
    todo!()
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
            serde_json::json!({"providers":[{"provider":"codex","displayName":"Codex","defaultExecutable":"codex","configuredExecutable":null,"configurationRevision":0,"effectiveExecutable":"codex","available":false,"resolvedExecutable":null,"unavailableReason":"not_found"}]}),
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
                    session_key: "terminal".into(),
                    cwd: Some("/last".into()),
                    has_foreground_process: false,
                    sequence: 10,
                }),
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
