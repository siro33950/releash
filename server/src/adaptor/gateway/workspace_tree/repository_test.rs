use super::*;

#[test]
fn test_workspace復元失敗_本番の変換でqueryとcodecの分類を保持する() {
    use crate::adaptor::gateway::local_event_store::{
        reader::storage_unavailable, test_helpers::ReadFailure,
    };
    use crate::adaptor::presenter::connect::classified_error;
    // Given
    for (failure, expected) in ReadFailure::cases() {
        let error = match failure {
            ReadFailure::Query(error) => error,
            ReadFailure::Sqlite(code) => storage_unavailable(&rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(code),
                None,
            )),
        };
        // When / Then
        assert_eq!(
            classified_error(fold_query_error(fact_log::FactReadError::Query(error))).code,
            expected
        );
    }
    assert_eq!(
        classified_error(fold_query_error(fact_log::FactReadError::Corrupt(
            "invalid stored value".into()
        )))
        .code,
        connectrpc::ErrorCode::DataLoss
    );
}

#[test]
fn test_session実行文脈_保存行からproviderと隔離先を復元し不正な値を拒否する() {
    use crate::domain::provider_lifecycle::ProviderKind;
    // Given
    let detail = serde_json::json!({"root": {
        "repositoryRoot": "/repo", "workspaceIdentity": "/workspace", "worktreePath": "/workspace",
        "launchedAs": "session", "definition": {"nodes": {"session": {"session": {"provider": "codex"}}}}
    }});
    let mut row = NodeEventRow {
        tree_id: "session".into(),
        seq: 1,
        node_execution_id: "session".into(),
        parent_id: None,
        node_name: "session".into(),
        kind: "session".into(),
        attempt: 1,
        event_type: "started".into(),
        session_id: Some("session".into()),
        detail: detail.to_string(),
        timestamp_ms: 1,
    };
    // When / Then
    let root = session_root_from_row(&row).unwrap();
    assert_eq!(root.1.provider, ProviderKind::Codex);
    assert_eq!(root.1.worktree_path, "/workspace");
    let mut isolated = detail.clone();
    isolated["root"]["definition"]["nodes"]["session"]["worktree"] = serde_json::json!("isolated");
    row.detail = isolated.to_string();
    let root = session_root_from_row(&row).unwrap();
    assert_eq!(
        root.1.worktree_path,
        crate::domain::workflow::IsolatedWorktree::for_attempt("/repo", "session", 1).path
    );
    for invalid in [
        serde_json::json!({}),
        serde_json::json!({"root": null}),
        {
            let mut invalid = detail.clone();
            invalid["root"]["definition"]["nodes"]["session"]["session"]["provider"] =
                serde_json::json!("unsupported");
            invalid
        },
        {
            let mut invalid = isolated.clone();
            invalid["root"]["repositoryRoot"] = serde_json::Value::Null;
            invalid
        },
        {
            let mut invalid = detail.clone();
            invalid["root"]["definition"]["nodes"]["session"]["worktree"] =
                serde_json::json!("unsupported");
            invalid
        },
    ] {
        row.detail = invalid.to_string();
        assert!(matches!(
            session_root_from_row(&row),
            Err(LocalEventQueryError::Corrupt { .. })
        ));
    }
    row.detail = detail.to_string();
    row.attempt = -1;
    assert!(session_root_from_row(&row).is_err());
}
