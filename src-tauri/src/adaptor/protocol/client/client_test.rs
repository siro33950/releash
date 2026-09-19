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
        id: id.into(),
        title: id.into(),
        status: "active".into(),
        error_reason: None,
        content_kind: "command",
        capabilities: dto::WorkspaceNodeCapabilitiesDto {
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
