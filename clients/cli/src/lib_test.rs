use super::*;

#[tokio::test]
async fn test_起動失敗_jsonと平文で理由と起動案内を返す() {
    // Given
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(discovery::discovery_file(dir.path()), "invalid json").unwrap();
    for json in [true, false] {
        let command = TopCommand::Server {
            command: server::ServerSubcommand::Start { json },
        };
        let (machine, guidance) = commands::output_options(&command);
        // When
        let error = commands::run(dir.path(), command).await.unwrap_err();
        let output = failure_output(&error, machine, guidance);
        // Then
        if json {
            let value: serde_json::Value = serde_json::from_str(&output).unwrap();
            assert_eq!(
                value,
                serde_json::json!({"error": {
                    "code": error.code.as_str(),
                    "message": error.message.as_deref().unwrap(),
                    "guidance": client::startup_guidance(),
                }}),
            );
        } else {
            assert_eq!(
                output,
                format!(
                    "error: {}: {}\n{}\n",
                    error.code.as_str(),
                    error.message.as_deref().unwrap(),
                    client::startup_guidance(),
                ),
            );
        }
    }
}

#[test]
fn test_失敗出力_messageなしではjsonと平文の既存の代替値を保つ() {
    // Given
    let mut error = connectrpc::ConnectError::unavailable("unused");
    error.message = None;
    for guidance in [None, Some(client::startup_guidance())] {
        // When
        let machine: serde_json::Value =
            serde_json::from_str(&failure_output(&error, true, guidance)).unwrap();
        let human = failure_output(&error, false, guidance);
        // Then
        let mut expected = serde_json::json!({"error": {
            "code": "unavailable", "message": error.to_string(),
        }});
        let mut expected_human = "error: unavailable: Request failed\n".to_string();
        if let Some(guidance) = guidance {
            expected["error"]["guidance"] = serde_json::json!(guidance);
            expected_human.push_str(&format!("{guidance}\n"));
        }
        assert_eq!(machine, expected);
        assert_eq!(human, expected_human);
    }
}
