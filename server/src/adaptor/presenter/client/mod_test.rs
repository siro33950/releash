mod client_tests {
    use super::super::*;
    use prost::Message;
    use serde_json::json;

    #[test]
    fn test_型付き応答_protoが成功と失敗を排他的に保持する() {
        // Given / When / Then
        for (command, result) in [
            ("update_external_editor", Ok(Json::Null)),
            ("start_workflow", Ok(json!("/日本語"))),
            (
                "start_workflow",
                Err(json!({"code":"INVALID_REQUEST","message":"invalid"})),
            ),
            ("start_workflow", Err(json!("failure"))),
        ] {
            let decoded = match result.clone() {
                Ok(value) => {
                    let message = CommandResult::from_value(command, value).unwrap();
                    Ok(from_value(
                        CommandResult::decode(message.encode_to_vec().as_slice()).unwrap(),
                    )
                    .unwrap())
                }
                Err(error) => {
                    let message: CommandError =
                        to_message("releash.client.v1.CommandError", error).unwrap();
                    let error = crate::adaptor::presenter::connect::command_error(
                        super::super::CommandFailure {
                            message: None,
                            kind: connectrpc::ErrorCode::Internal,
                            detail: message,
                        },
                    );
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
    fn test_型に合わない結果は成功応答にせずerrorを返す() {
        let error = crate::adaptor::presenter::client::value::<_, WorkspaceCenterTab>(
            "invalid".to_string(),
        )
        .unwrap_err();
        assert_eq!(from_value(error).unwrap()["code"], "INVALID_RESPONSE");
    }

    #[test]
    fn test_terminal_eventは最大sequenceと日本語を保持する() {
        let message = TerminalEvent {
            item: Some(terminal_event::Item::Output(TerminalOutput {
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
    fn test_workflow応答_connectの詳細と一覧は三状態の値を保ち削除項目を含まない() {
        // Given / When / Then
        for status in ["running", "completed", "aborted"] {
            let value = json!({
                "id": "execution-1", "workflowName": "review", "status": status,
                "currentNode": "review", "worktreePath": "/repo",
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
}
