use super::*;
use prost::Message;
use serde_json::json;

#[test]
fn test_型付き応答_protoが成功と失敗を排他的に保持する() {
    // Given / When / Then
    for (command, result) in [
        ("update_external_editor", Ok(Json::Null)),
        ("get_language_from_path", Ok(json!("日本語"))),
        ("start_workflow", Ok(json!("/a"))),
        (
            "get_language_from_path",
            Err(json!({"code":"INVALID_REQUEST","message":"invalid"})),
        ),
        ("get_language_from_path", Err(json!("failure"))),
    ] {
        let decoded = match result.clone() {
            Ok(value) => {
                let message = CommandResult::from_value(command, value).unwrap();
                Ok(
                    from_value(CommandResult::decode(message.encode_to_vec().as_slice()).unwrap())
                        .unwrap(),
                )
            }
            Err(error) => {
                let message: CommandError =
                    to_message("releash.client.v1.CommandError", error).unwrap();
                let error =
                    crate::adaptor::presenter::connect::command_error(super::CommandFailure {
                        message: None,
                        kind: connectrpc::ErrorCode::Internal,
                        detail: message,
                    });
                use base64::Engine;
                let bytes = base64::engine::general_purpose::STANDARD_NO_PAD
                    .decode(error.details[0].value.as_ref().unwrap())
                    .unwrap();
                Err(from_value(CommandError::decode(bytes.as_slice()).unwrap()).unwrap())
            }
        };
        assert_eq!(decoded, result);
    }
}

#[test]
fn test_workflow値_整数境界と浮動小数を区別して保持する() {
    let value = json!({"integers": [i64::MIN, u64::MAX, 3], "numbers": [3.0, 0.25]});
    let wire = WorkflowValue::try_from(value.clone()).unwrap();
    let decoded = WorkflowValue::decode(wire.encode_to_vec().as_slice()).unwrap();
    assert_eq!(Json::try_from(decoded).unwrap(), value);
}

#[test]
fn test_クライアント引数_必須フィールドと整数型を検証する() {
    // Given / When / Then
    assert!(CommandRequest::from_value("build_diff_file_tree", json!({})).is_err());
    assert!(CommandRequest::from_value(
        "write_terminal_surface",
        json!({"owner":{"kind":"workspace","workspacePath":"/repo"},"attachmentId":"a", "sequence": 1.5,"data":"x","clientStartedAtUnixMs":null})
    )
    .is_err());
    let args = json!({"owner":{"kind":"workspace","workspacePath":"/repo"},"attachmentId":"a", "sequence": u64::MAX,"data":"x","clientStartedAtUnixMs":null});
    let request = CommandRequest::from_value("write_terminal_surface", args.clone()).unwrap();
    let decoded = CommandRequest::decode(request.encode_to_vec().as_slice())
        .unwrap()
        .into_value()
        .unwrap();
    assert_eq!(decoded, ("write_terminal_surface", args));
    let request = CommandRequest {
        command: Some(command_request::Command::BuildDiffFileTree(
            BuildDiffFileTreeRequest::default(),
        )),
    };
    assert!(request.into_value().is_err());
    assert!(CommandRequest::decode(&b"not-protobuf"[..]).is_err());
}

#[test]
fn test_型に合わない結果は成功応答にせずerrorを返す() {
    let error =
        crate::adaptor::controller::client::value::<_, WorkspaceCenterTab>("invalid".to_string())
            .unwrap_err();
    assert_eq!(from_value(error).unwrap()["code"], "INVALID_RESPONSE");
}

#[test]
fn test_terminal_eventは最大sequenceと日本語を保持する() {
    let message = TerminalEvent {
        item: Some(terminal_event::Item::Output(TerminalOutput {
            session_key: "a".into(),
            sequence: u64::MAX,
            data: "日本語".into(),
        })),
    };
    assert_eq!(
        TerminalEvent::decode(message.encode_to_vec().as_slice()).unwrap(),
        message
    );
}

#[test]
fn test_workflow状態_protoは削除した番号と名前を予約し残る三状態を保持する() {
    // Given
    let pool = descriptor::pool();
    // When / Then
    for (name, numbers) in [
        ("ExecutionStatusView", [1, 4]),
        ("WorkspaceHistoryStatus", [7, 8]),
    ] {
        let status = pool
            .get_enum_by_name(&format!("releash.client.v1.{name}.Value"))
            .unwrap();
        assert_eq!(
            status.reserved_names().collect::<Vec<_>>(),
            if name == "WorkspaceHistoryStatus" {
                vec![
                    "paused",
                    "failed",
                    "waiting_approval",
                    "interrupted",
                    "unresolved",
                ]
            } else {
                vec!["waiting_approval", "interrupted"]
            }
        );
        for number in numbers {
            assert!(status
                .reserved_ranges()
                .any(|range| range.contains(&number)));
            assert!(status.get_value(number).is_none());
        }
        if name != "WorkspaceHistoryStatus" {
            assert_eq!(
                status
                    .values()
                    .map(|value| (value.name().to_string(), value.number()))
                    .collect::<Vec<_>>(),
                [
                    ("running".into(), 0),
                    ("completed".into(), 2),
                    ("aborted".into(), 3)
                ]
            );
        }
    }
    for name in ["WorkflowExecutionView"] {
        let message = pool
            .get_message_by_name(&format!("releash.client.v1.{name}"))
            .unwrap();
        assert_eq!(
            message.reserved_names().collect::<Vec<_>>(),
            ["interruption_reason", "resume_from_node"]
        );
        for number in [11, 12] {
            assert!(message
                .reserved_ranges()
                .any(|range| range.contains(&number)));
            assert!(message.get_field(number).is_none());
        }
    }
    for value in ["waiting_approval", "interrupted"] {
        assert!(ExecutionStatusView::try_from(value).is_err());
        assert!(WorkspaceHistoryStatus::try_from(value).is_err());
        assert!(serde_json::from_value::<
            crate::adaptor::presenter::workflow_wire::ExecutionStatusView,
        >(json!(value))
        .is_err());
    }
    assert!(NodeExecutionStatusView::try_from("waiting_approval").is_ok());
}

#[test]
fn test_workflow応答_connectの詳細と一覧は三状態の値を保ち削除項目を含まない() {
    // Given / When / Then
    for status in ["running", "completed", "aborted"] {
        let value = json!({
            "id": "execution-1", "workflowName": "review", "status": status,
            "currentNode": "review", "worktreePath": "/repo", "createdFrom": "cli",
            "startedAt": 1.0, "updatedAt": 2.0, "completedAt": null, "errorReason": null,
            "totalTokenUsage": {"inputTokens": 13, "outputTokens": 8},
            "nodeExecutions": [], "artifacts": [], "fanouts": [], "approvalTarget": null
        });
        let view: crate::adaptor::presenter::workflow_wire::WorkflowExecutionView =
            serde_json::from_value(value.clone()).unwrap();
        let wire = WorkflowExecutionView::try_from(view).unwrap();
        let decoded = WorkflowExecutionView::decode(wire.encode_to_vec().as_slice()).unwrap();
        assert_eq!(
            from_message("releash.client.v1.WorkflowExecutionView", &decoded).unwrap(),
            value
        );
    }
}

#[test]
fn removed_workflow_commands_and_node_fields_cannot_reuse_their_wire_tags() {
    let pool = descriptor::pool();
    for name in ["CommandRequest", "CommandResult"] {
        let message = pool
            .get_message_by_name(&format!("releash.client.v1.{name}"))
            .unwrap();
        for (number, field) in [
            (30, "delete_branch"),
            (121, "resume_agent_session"),
            (123, "stop_workflow"),
            (137, "resume_workflow"),
        ] {
            assert!(message
                .reserved_ranges()
                .any(|range| range.contains(&number)));
            assert!(message.reserved_names().any(|name| name == field));
            assert!(message.get_field(number).is_none());
        }
    }
    let node = pool
        .get_message_by_name("releash.client.v1.NodeExecutionView")
        .unwrap();
    assert!(node.reserved_names().any(|name| name == "failure"));
    assert!(node.get_field(20).is_none());
    assert!(node.reserved_ranges().any(|range| range.contains(&20)));
    let capabilities = pool
        .get_message_by_name("releash.client.v1.WorkspaceWorkflowCapabilities")
        .unwrap();
    for (number, name) in [(1, "can_stop"), (2, "can_resume")] {
        assert!(capabilities
            .reserved_ranges()
            .any(|range| range.contains(&number)));
        assert!(capabilities
            .reserved_names()
            .any(|reserved| reserved == name));
        assert!(capabilities.get_field(number).is_none());
    }
    let operations = pool
        .get_message_by_name("releash.client.v1.AgentSessionOperationsDto")
        .unwrap();
    assert!(operations.reserved_ranges().any(|range| range.contains(&4)));
    assert!(operations
        .reserved_names()
        .any(|reserved| reserved == "can_resume"));
    assert!(operations.get_field(4).is_none());
    for name in [
        "NodeExecutionStatusView",
        "WorkspaceNodeStatus",
        "WorkspaceHistoryStatus",
    ] {
        let status = pool
            .get_enum_by_name(&format!("releash.client.v1.{name}.Value"))
            .unwrap();
        let numbers = if name == "NodeExecutionStatusView" {
            [2, 5]
        } else {
            [2, 3]
        };
        for number in numbers {
            assert!(status
                .reserved_ranges()
                .any(|range| range.contains(&number)));
            assert!(status.get_value(number).is_none());
        }
        for removed in ["paused", "failed"] {
            assert!(status.reserved_names().any(|name| name == removed));
            assert!(status.get_value_by_name(removed).is_none());
        }
    }
}

#[test]
fn test_実行状態_削除した未解決状態のwire値を受け入れない() {
    // Given / When / Then
    assert!(NodeExecutionStatusView::try_from("unresolved").is_err());
    assert!(json::from_message(
        "releash.client.v1.NodeExecutionStatusView",
        &NodeExecutionStatusView { value: Some(0) }
    )
    .is_err());
    assert!(json::to_message::<NodeExecutionStatusView>(
        "releash.client.v1.NodeExecutionStatusView",
        serde_json::json!("unresolved")
    )
    .is_err());
}

#[test]
fn test_状態分類_到達不能なfailureを公開せず番号と名前を予約する() {
    // Given
    let pool = descriptor::pool();
    let status = pool
        .get_enum_by_name("releash.client.v1.WorkspaceStatusClassification.Value")
        .unwrap();
    // When / Then
    assert!(status.reserved_ranges().any(|range| range.contains(&2)));
    assert!(status.reserved_names().any(|name| name == "failure"));
    assert!(status.get_value(2).is_none());
    assert!(status.get_value_by_name("failure").is_none());
    assert!(status.reserved_ranges().any(|range| range.contains(&4)));
    assert!(status.reserved_names().any(|name| name == "unbound"));
    assert!(status.get_value(4).is_none());
}

#[test]
fn test_一覧更新_引数なしの要求と空の応答をprotobuf往復で保持する() {
    // Given / When
    let request = CommandRequest::from_value("refresh_workspaces", json!({})).unwrap();
    let decoded = CommandRequest::decode(request.encode_to_vec().as_slice()).unwrap();
    // Then
    assert_eq!(
        decoded.into_value().unwrap(),
        ("refresh_workspaces", json!({}))
    );
    assert_eq!(
        CommandResult::from_value("refresh_workspaces", Json::Null)
            .unwrap()
            .into_value()
            .unwrap(),
        ("refresh_workspaces", Json::Null)
    );
}
