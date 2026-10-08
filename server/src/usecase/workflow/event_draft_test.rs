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
pub(crate) mod tests {
    use super::super::*;

    fn artifact_produced(node: &str, timestamp: f64, verdict: &str) -> WorkflowEventDraft {
        WorkflowEventDraft {
            execution_id: "execution-1".to_string(),
            event_kind: "artifact_produced".to_string(),
            timestamp,
            payload: serde_json::json!({
                "nodeExecutionId": format!("node-{timestamp}"),
                "nodeName": node,
                "kind": "session",
                "attempt": 1,
                "contract": "review-verdict",
                "value": {"verdict": verdict},
                "requestId": format!("req-{timestamp}")
            }),
        }
    }

    #[test]
    fn latest_artifact_produced_from_drafts_picks_latest_matching_node() {
        let events = vec![
            artifact_produced("review", 10.0, "NEEDS_FIX"),
            artifact_produced("other", 20.0, "BLOCKED"),
            artifact_produced("review", 30.0, "LGTM"),
        ];

        let snapshot = latest_artifact_produced_from_drafts(&events, "review")
            .expect("latest matching output should be returned");

        assert_eq!(snapshot.timestamp, 30.0);
        assert_eq!(snapshot.value["verdict"], "LGTM");
        assert_eq!(snapshot.submitted_at, Some(30.0));
        assert_eq!(snapshot.request_id.as_deref(), Some("req-30"));
    }

    #[test]
    fn latest_artifact_produced_from_drafts_rejects_noncanonical_payload() {
        let events = vec![WorkflowEventDraft {
            execution_id: "execution-1".to_string(),
            event_kind: "artifact_produced".to_string(),
            timestamp: 10.0,
            payload: serde_json::json!({
                "node_execution_id": "node-10",
                "nodeName": "review",
                "contract": "review-verdict",
                "structuredOutput": {"verdict": "LGTM"},
                "submittedAt": 9.0,
                "requestId": "req-10"
            }),
        }];

        assert!(latest_artifact_produced_from_drafts(&events, "review").is_none());
    }

    #[test]
    fn latest_artifact_produced_from_drafts_returns_none_without_matching_node() {
        let events = vec![artifact_produced("other", 20.0, "BLOCKED")];

        assert!(latest_artifact_produced_from_drafts(&events, "review").is_none());
        assert!(latest_artifact_produced_from_drafts(&[], "review").is_none());
    }

    #[test]
    fn latest_artifact_produced_from_drafts_returns_contractless_artifact() {
        let events = vec![WorkflowEventDraft {
            execution_id: "execution-1".to_string(),
            event_kind: "artifact_produced".to_string(),
            timestamp: 10.0,
            payload: serde_json::json!({
                "nodeExecutionId": "node-10",
                "nodeName": "review",
                "kind": "command",
                "attempt": 1,
                "value": {"stdout": "ok"}
            }),
        }];

        let snapshot = latest_artifact_produced_from_drafts(&events, "review")
            .expect("contractless standard artifact should be returned");

        assert_eq!(snapshot.contract, None);
        assert_eq!(snapshot.value["stdout"], "ok");
    }
}
