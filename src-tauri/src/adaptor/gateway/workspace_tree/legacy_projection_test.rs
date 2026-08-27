use super::*;
use crate::adaptor::gateway::local_event_store::layout::StoreLayout;
use crate::adaptor::gateway::local_event_store::store::LocalEventStoreConfig;

#[tokio::test]
async fn legacy_agent_projection_row_is_ignored_by_canonical_session_and_workspace_queries() {
    let root = tempfile::TempDir::new().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(root.path().to_path_buf()))
        .unwrap();

    let connection =
        rusqlite::Connection::open(StoreLayout::new(root.path()).database_path()).unwrap();
    connection
        .execute(
            "INSERT INTO logical_commits (
                    commit_id, installation_id, operation_kind, idempotency_key, payload_hash,
                    state, first_global_sequence, last_global_sequence, event_count,
                    mutation_count, stream_heads_json, result_hash, committed_at_ms
                 ) VALUES (?1, 'legacy-install', 'projection', 'legacy-key', ?2,
                    'sealed', NULL, NULL, 0, 1, '[]', ?2, 0)",
            rusqlite::params!["legacy-commit", [0_u8; 32].as_slice()],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO session_projection (
                    session_id, projection, revision, commit_id, workspace_identity,
                    public_list_kind, public_sort_key_bits, public_summary
                 ) VALUES (?1, ?2, 0, ?3, ?4, 'active', 0, ?5)",
            rusqlite::params![
                "legacy-session-1",
                r#"{"schema":"legacy_agent_session_projection_v0"}"#,
                "legacy-commit",
                "/repo",
                r#"{"schema":"legacy_session_public_summary_v0"}"#,
            ],
        )
        .unwrap();
    drop(connection);

    let repository = SqliteWorkspaceTreeRepository::new(store);
    let trees = repository.folded_workspace_trees("/repo").unwrap();
    assert!(repository
        .workspace_tree_from_folded("/repo", &trees)
        .unwrap()
        .is_none());
}
