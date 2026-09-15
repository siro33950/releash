use super::*;
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
        let bytes = response(
            "request-1".into(),
            match result.clone() {
                Ok(value) => Ok(CommandResult::from_value(command, value)
                    .unwrap()
                    .command
                    .unwrap()),
                Err(error) => Err(to_message("releash.client.v1.CommandError", error).unwrap()),
            },
        )
        .encode_to_vec();
        let envelope::Body::Response(response) =
            Envelope::decode(bytes.as_slice()).unwrap().body.unwrap()
        else {
            panic!("response");
        };
        assert_eq!(response.request_id, "request-1");
        let decoded = match response.outcome.unwrap() {
            command_response::Outcome::Result(value) => Ok(from_value(value).unwrap()),
            command_response::Outcome::Error(value) => Err(from_value(value).unwrap()),
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
        instance_id: String::new(),
        recover: false,
        predecessors: Vec::new(),
        deadline_unix_ms: 0,
        user_retry: false,
        successors: Vec::new(),
        request_id: "id".into(),
        command: Some(command_request::Command::GetCurrentBranch(
            GetCurrentBranchRequest::default(),
        )),
    };
    assert!(request.into_value().is_err());
    assert!(Envelope::decode(&b"not-protobuf"[..]).is_err());
}

#[test]
fn test_型に合わない結果は成功応答にせず相関したerrorを返す() {
    // Given / When
    let envelope::Body::Response(response) = response(
        "id".into(),
        Err(
            crate::adaptor::controller::client::value::<_, WorkspaceCenterTab>(
                "invalid".to_string(),
            )
            .unwrap_err(),
        ),
    )
    .body
    .unwrap() else {
        panic!("response");
    };
    // Then
    assert_eq!(response.request_id, "id");
    let command_response::Outcome::Error(error) = response.outcome.unwrap() else {
        panic!("error");
    };
    assert_eq!(from_value(error).unwrap()["code"], "INVALID_RESPONSE");
}

#[test]
fn test_stream規約_attachmentと最大sequenceと分割終端を保持する() {
    // Given / When / Then
    for id in ["a", "b"] {
        let envelope::Body::Stream(frame) = Envelope::decode(
            stream_frame(id, u64::MAX, "日本語".as_bytes().to_vec(), true).as_slice(),
        )
        .unwrap()
        .body
        .unwrap() else {
            panic!("stream");
        };
        assert_eq!(frame.attachment_id, id);
        assert_eq!(frame.sequence, u64::MAX);
        assert_eq!(frame.data, "日本語".as_bytes());
        assert!(frame.end);
    }
    assert_eq!(MAX_STREAM_FRAME_BYTES, 65536);
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
        let envelope::Body::Push(frame) = Envelope {
            body: Some(envelope::Body::Push(
                Push::from_value(event, payload.clone()).unwrap(),
            )),
        }
        .body
        .unwrap() else {
            panic!("push");
        };
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
