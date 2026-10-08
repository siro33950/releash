use super::*;

#[test]
fn test_成果の事実変換_所有者と親とattemptと順序を保持する() {
    // Given
    let events = vec![
        WorkflowEventDraft {
            execution_id: "tree".into(),
            event_kind: "started".into(),
            timestamp: 1.0,
            payload: serde_json::json!({"nodeExecutionId": "child", "nodeName": "work", "kind": "session", "attempt": 2, "parent": {"parentId": "parent"}}),
        },
        WorkflowEventDraft {
            execution_id: "tree".into(),
            event_kind: "artifact_produced".into(),
            timestamp: 2.0,
            payload: serde_json::json!({"nodeExecutionId": "child", "nodeName": "work", "kind": "session", "attempt": 2, "contract": "result", "value": {"result": true}, "requestId": "request"}),
        },
    ];
    // When
    let records = records_from_drafts(&events).unwrap();
    // Then
    assert_eq!(records[0].meta, records[1].meta);
    assert_eq!(records[1].meta.tree_id, "tree");
    assert_eq!(records[1].meta.node_execution_id, "child");
    assert_eq!(records[1].meta.parent_id.as_deref(), Some("parent"));
    assert_eq!(records[1].meta.attempt, 2);
    assert_eq!(records[1].seq, 2);
    assert_eq!(records[1].timestamp_ms, 2000);
    let crate::domain::workflow::NodeFact::ArtifactProduced(artifact) = &records[1].fact else {
        panic!("artifact fact expected");
    };
    assert_eq!(artifact.request_id.as_deref(), Some("request"));
    assert_eq!(artifact.value, serde_json::json!({"result": true}));
}

#[test]
fn test_成果の事実変換_破損した識別情報と未知の事実を拒否する() {
    // Given
    let valid = serde_json::json!({"nodeExecutionId": "child", "nodeName": "work", "kind": "session", "attempt": 2});
    for (kind, payload) in [("started", serde_json::json!({})), ("unknown", valid)] {
        let event = WorkflowEventDraft {
            execution_id: "tree".into(),
            event_kind: kind.into(),
            timestamp: 1.0,
            payload,
        };
        // When / Then
        assert!(records_from_drafts(&[event]).is_err());
    }
}

#[test]
fn test_成果の事実変換_旧隔離事実を状態入力に復活させず順序を保持する() {
    // Given
    let events = ["isolated_worktree_created", "isolated_worktree_released", "isolated_worktree_lost", "submit_received"]
        .into_iter().map(|event_kind| WorkflowEventDraft {
            execution_id: "tree".into(), event_kind: event_kind.into(), timestamp: 1.0,
            payload: serde_json::json!({"nodeExecutionId": "child", "nodeName": "work", "kind": "session", "attempt": 1,
                "repositoryRoot": "/repo", "worktreePath": "/old", "branch": "old"}),
        }).collect::<Vec<_>>();
    // When
    let records = records_from_drafts(&events).unwrap();
    // Then
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].seq, 4);
    assert!(matches!(
        records[0].fact,
        crate::domain::workflow::NodeFact::SubmitReceived(_)
    ));
}
