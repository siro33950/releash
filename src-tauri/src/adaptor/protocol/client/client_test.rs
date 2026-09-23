use super::*;
use prost::Message;
use serde_json::json;

#[test]
fn test_型付き応答_protoが成功と失敗を排他的に保持する() {
    // Given / When / Then
    for (command, result) in [
        ("ack_terminal_surface_output", Ok(Json::Null)),
        ("get_current_branch", Ok(json!("日本語"))),
        ("get_repo_paths", Ok(json!(["/a", "/b"]))),
        (
            "get_workflow_execution_log",
            Ok(
                json!([{"event":"artifact_produced","execution_id":"e","timestampMs":42.0,"node_name":"review","value":{"items":[null,true,3,"日本語"]},"submittedAtMs":40.0}]),
            ),
        ),
        ("get_workflow_execution_log", Ok(Json::Null)),
        (
            "get_current_branch",
            Err(json!({"code":"INVALID_REQUEST","message":"invalid"})),
        ),
        ("get_current_branch", Err(json!("failure"))),
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
                    crate::adaptor::controller::api::protocol::connect::command_error(message);
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
    assert!(CommandRequest::from_value("get_current_branch", json!({})).is_err());
    assert!(CommandRequest::from_value(
        "ack_terminal_surface_output",
        json!({"attachmentId":"a", "sequence": 1.5})
    )
    .is_err());
    let args = json!({"attachmentId":"a", "sequence": u64::MAX});
    let request = CommandRequest::from_value("ack_terminal_surface_output", args.clone()).unwrap();
    let decoded = CommandRequest::decode(request.encode_to_vec().as_slice())
        .unwrap()
        .into_value()
        .unwrap();
    assert_eq!(decoded, ("ack_terminal_surface_output", args));
    let request = CommandRequest {
        command: Some(command_request::Command::GetCurrentBranch(
            GetCurrentBranchRequest::default(),
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
fn test_push_protoが既存payloadを保持し未定義eventを拒否する() {
    // Given / When / Then
    for (event, payload) in [
        ("repo-paths-changed", json!(["/a", "/b"])),
        ("branch-list-sync", Json::Null),
        ("review-comments-changed", json!("worktree")),
        ("git-status-changed", json!({"repo_path":"/repo"})),
    ] {
        let message = Push::from_value(event, payload.clone()).unwrap();
        let frame = Push::decode(message.encode_to_vec().as_slice()).unwrap();
        assert_eq!(frame.into_value().unwrap(), (event, payload));
    }
    assert!(Push::from_value("unknown", Json::Null).is_err());
}

#[test]
fn test_workspace過去試行_両commandでnodeタグとchildren省略を保持する() {
    use crate::usecase::workflow as dto;
    // Given
    let node = |id: &str| dto::WorkspaceNodeDto {
        process_presence: "unknown",
        id: id.into(),
        title: id.into(),
        status: "active".into(),
        error_reason: None,
        content_kind: "command",
        capabilities: dto::WorkspaceNodeCapabilitiesDto {
            can_resume_session: false,
            can_rename: false,
            can_approve: false,
            can_retry: true,
        },
        workflow_capabilities: None,
        session_capabilities: None,
        children: vec![],
        past_attempts: vec![],
        past_attempts_collapsed: true,
        updated_at: 1.0,
    };
    let mut current = node("current");
    current.past_attempts.push(node("past"));
    let snapshot = dto::WorkspaceTreeSnapshotDto {
        nodes: vec![dto::WorkspaceTreeItemDto::Node(current)],
        archived_sessions: vec![],
        preferred_node_id: None,
    };
    let selection = dto::WorkspaceTreeSelectionSnapshotDto {
        snapshot: snapshot.clone(),
        reconciliation: dto::WorkspaceSelectionReconciliationDto {
            selection_in_snapshot: true,
        },
    };
    // When / Then
    for (result, expected) in [
        (
            command_result::Command::ListWorkspaceWorktreeNodes(
                snapshot.clone().try_into().unwrap(),
            ),
            serde_json::to_value(snapshot).unwrap(),
        ),
        (
            command_result::Command::GetWorkspaceTreeSelectionReconciliation(
                selection.clone().try_into().unwrap(),
            ),
            serde_json::to_value(selection).unwrap(),
        ),
    ] {
        let result = CommandResult {
            command: Some(result),
        };
        let decoded = CommandResult::decode(result.encode_to_vec().as_slice()).unwrap();
        assert_eq!(from_value(decoded).unwrap(), expected);
    }
}

#[test]
fn test_workflowログ_固定metadataを型付きで生成messageへ渡す() {
    // Given
    let view = crate::usecase::workflow::WorkflowEventView {
        event: "artifact_produced".into(),
        execution_id: "execution".into(),
        timestamp_ms: 1234.5,
        payload: serde_json::from_value(
            json!({"value":{"items":[null, true, 42]},"submittedAtMs":1000.0}),
        )
        .unwrap(),
    };
    let expected = serde_json::to_value(&view).unwrap();
    // When
    let entry = DurableWorkflowFactLogEntry::try_from(view).unwrap();
    let decoded = DurableWorkflowFactLogEntry::decode(entry.encode_to_vec().as_slice()).unwrap();
    // Then
    assert_eq!(
        from_message("releash.client.v1.DurableWorkflowFactLogEntry", &decoded).unwrap(),
        expected
    );
}

#[test]
fn test_workflow状態_protoは削除した番号と名前を予約し残る三状態を保持する() {
    // Given
    let pool = prost_reflect::DescriptorPool::decode(
        include_bytes!(concat!(env!("OUT_DIR"), "/client_descriptor.bin")).as_slice(),
    )
    .unwrap();
    // When / Then
    for (name, numbers) in [
        ("ExecutionStatusView", [1, 4]),
        ("ExecutionStatusDto", [1, 4]),
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
    for name in ["WorkflowExecutionView", "WorkflowExecutionSummaryDto"] {
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
        assert!(ExecutionStatusDto::try_from(value).is_err());
        assert!(WorkspaceHistoryStatus::try_from(value).is_err());
        assert!(
            serde_json::from_value::<crate::adaptor::protocol::workflow::ExecutionStatusView>(
                json!(value)
            )
            .is_err()
        );
        assert!(
            serde_json::from_value::<crate::usecase::workflow::dto::ExecutionStatusDto>(json!(
                value
            ))
            .is_err()
        );
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
        let view: crate::adaptor::protocol::workflow::WorkflowExecutionView =
            serde_json::from_value(value.clone()).unwrap();
        let wire = WorkflowExecutionView::try_from(view).unwrap();
        let decoded = WorkflowExecutionView::decode(wire.encode_to_vec().as_slice()).unwrap();
        assert_eq!(
            from_message("releash.client.v1.WorkflowExecutionView", &decoded).unwrap(),
            value
        );

        let value = json!({
            "executionId": "execution-1", "workflowName": "review", "status": status,
            "currentNode": "review", "worktreePath": "/repo", "createdFrom": "cli",
            "startedAt": 1.0, "updatedAt": 2.0,
            "totalTokenUsage": {"inputTokens": 13, "outputTokens": 8}
        });
        let summary: crate::usecase::workflow::dto::WorkflowExecutionSummaryDto =
            serde_json::from_value(value.clone()).unwrap();
        let wire = WorkflowExecutionSummaryDto::try_from(summary).unwrap();
        let decoded = WorkflowExecutionSummaryDto::decode(wire.encode_to_vec().as_slice()).unwrap();
        assert_eq!(
            from_message("releash.client.v1.WorkflowExecutionSummaryDto", &decoded).unwrap(),
            value
        );
    }
}

#[test]
fn removed_workflow_commands_and_node_fields_cannot_reuse_their_wire_tags() {
    let pool = prost_reflect::DescriptorPool::decode(
        include_bytes!(concat!(env!("OUT_DIR"), "/client_descriptor.bin")).as_slice(),
    )
    .unwrap();
    for name in ["CommandRequest", "CommandResult"] {
        let message = pool
            .get_message_by_name(&format!("releash.client.v1.{name}"))
            .unwrap();
        for (number, field) in [(123, "stop_workflow"), (137, "resume_workflow")] {
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
        .get_message_by_name("releash.client.v1.WorkspaceWorkflowCapabilitiesDto")
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
    let pool = prost_reflect::DescriptorPool::decode(
        include_bytes!(concat!(env!("OUT_DIR"), "/client_descriptor.bin")).as_slice(),
    )
    .unwrap();
    let status = pool
        .get_enum_by_name("releash.client.v1.WorkspaceStatusClassification.Value")
        .unwrap();
    // When / Then
    assert!(status.reserved_ranges().any(|range| range.contains(&2)));
    assert!(status.reserved_names().any(|name| name == "failure"));
    assert!(status.get_value(2).is_none());
    assert!(status.get_value_by_name("failure").is_none());
}

#[test]
fn test_workspaces一覧_保持した情報と取得状態がwireを往復する() {
    // Given
    use crate::usecase::workspace_tree::{
        WorkspaceBranchDto, WorkspaceListSnapshotDto, WorkspaceListStatusDto,
        WorkspaceRepositoryListDto, WorkspaceWorktreeListDto,
    };
    let snapshot = WorkspaceListSnapshotDto {
        generation: 3,
        status: WorkspaceListStatusDto {
            state: "ready",
            loaded: true,
            error: None,
        },
        repositories: vec![WorkspaceRepositoryListDto {
            path: "/repo".into(),
            status: WorkspaceListStatusDto {
                state: "refreshFailed",
                loaded: true,
                error: Some("scan failed".into()),
            },
            branches: vec![WorkspaceBranchDto {
                branch: crate::usecase::repository_dto::BranchCardDto {
                    name: "main".into(),
                    is_main_worktree: true,
                    is_deleting: false,
                    worktree_path: Some("/repo".into()),
                    dirty_count: 0,
                    is_merged: false,
                    ahead: 0,
                    behind: 0,
                    has_upstream: false,
                    base_ahead: 0,
                },
                has_pr: true,
                pr_number: Some(7),
                pr_url: Some("https://example.com/pr/7".into()),
            }],
            worktrees: vec![WorkspaceWorktreeListDto {
                path: "/repo".into(),
                status: WorkspaceListStatusDto {
                    state: "initialFailed",
                    loaded: false,
                    error: Some("nodes failed".into()),
                },
                snapshot: None,
                workflow_history: vec![],
            }],
        }],
    };
    let expected = serde_json::to_value(&snapshot).unwrap();
    // When
    let result = CommandResult {
        command: Some(command_result::Command::RefreshWorkspaces(
            snapshot.try_into().unwrap(),
        )),
    };
    let decoded = CommandResult::decode(result.encode_to_vec().as_slice()).unwrap();
    // Then
    let from_json = CommandResult::from_value("refresh_workspaces", expected.clone()).unwrap();
    assert_eq!(from_json.encode_to_vec(), result.encode_to_vec());
    assert_eq!(
        decoded.into_value().unwrap(),
        ("refresh_workspaces", expected)
    );
}

#[test]
fn test_一覧更新引数_全体とrepositoryとworktree指定をprotobuf往復で保持する() {
    // Given / When / Then
    for (worktree, repo) in [
        (None, None),
        (Some("/repo/worktree"), None),
        (None, Some("/repo")),
    ] {
        let args = json!({"worktreePath": worktree, "repoPath": repo});
        let request = CommandRequest::from_value("refresh_workspaces", args.clone()).unwrap();
        let decoded = CommandRequest::decode(request.encode_to_vec().as_slice()).unwrap();
        assert_eq!(decoded.into_value().unwrap(), ("refresh_workspaces", args));
    }
    assert!(CommandRequest::from_value("refresh_workspaces", json!({})).is_ok());
    assert!(CommandRequest::from_value("refresh_workspaces", json!({"worktreePath": 42})).is_err());
    assert!(CommandRequest::from_value("refresh_workspaces", json!({"repoPath": 42})).is_err());
}
