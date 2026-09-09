use super::*;

fn drafts() -> Vec<WorkflowEventDraft> {
    vec![
        WorkflowEventDraft {
            execution_id: "tree".into(),
            event_kind: "started".into(),
            payload: serde_json::json!({"root": {"repositoryRoot": "/repo"}}),
            timestamp: 1.0,
        },
        WorkflowEventDraft {
            execution_id: "tree".into(),
            event_kind: "artifact_produced".into(),
            payload: serde_json::json!({
                "nodeName": "work", "nodeExecutionId": "attempt", "attempt": 2,
                "contract": "result", "value": {"summary": "done"}, "requestId": "request",
            }),
            timestamp: 42.0,
        },
    ]
}

#[test]
fn test_隔離成果読取_提出情報と所有attemptの識別を同じpayloadから取得する() {
    // Given
    let events = drafts();
    // When
    let output = latest_isolated_artifact_from_drafts(&events, "work", "tree")
        .unwrap()
        .unwrap();
    // Then
    assert_eq!(output.contract.as_deref(), Some("result"));
    assert_eq!(output.request_id.as_deref(), Some("request"));
    assert_eq!(output.submitted_at, Some(42.0));
    assert_eq!(output.timestamp, 42.0);
    assert_eq!(
        output.value,
        serde_json::json!({
            "summary": "done",
            "worktree": {"branch": "releash/isolated/attempt-a2", "path": "/repo-worktrees/.releash-isolated/attempt-a2"},
        })
    );
}

#[test]
fn test_隔離成果読取_所有者やattemptが欠損または不正ならエラーを返す() {
    // Given
    for (key, value) in [
        ("nodeExecutionId", Value::Null),
        ("nodeExecutionId", serde_json::json!(2)),
        ("attempt", Value::Null),
        ("attempt", serde_json::json!(-1)),
        ("attempt", serde_json::json!(u64::MAX)),
    ] {
        let mut events = drafts();
        events[1].payload[key] = value;
        // When / Then
        assert!(latest_isolated_artifact_from_drafts(&events, "work", "tree").is_err());
        assert!(latest_artifact_produced_from_drafts(&events, "work").is_some());
    }
}
