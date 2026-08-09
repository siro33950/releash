use std::fs;

use super::LocalProviderAgentSessionHistoryGateway;
use crate::domain::agent_session::ProviderAgentSessionHistoryGateway;
use crate::domain::provider_lifecycle::ProviderKind;

#[tokio::test]
async fn test_provider_agent_session_history_gateway_claudeのmetadataだけをboundedに読む() {
    let directory = tempfile::tempdir().unwrap();
    let claude_root = directory.path().join("claude");
    let codex_root = directory.path().join("codex");
    let project = claude_root.join("projects").join("-repo-worktree");
    fs::create_dir_all(&project).unwrap();
    let history = project.join("claude-1.jsonl");
    fs::write(
        &history,
        concat!(
            "{\"type\":\"queue-operation\",\"sessionId\":\"claude-1\"}\n",
            "{\"type\":\"user\",\"sessionId\":\"claude-1\",\"cwd\":\"/repo/worktree\",\"message\":{\"content\":\"must-not-be-returned\"}}\n",
        ),
    )
    .unwrap();
    let gateway = LocalProviderAgentSessionHistoryGateway::new(claude_root, codex_root);

    let entries = gateway
        .list_metadata(ProviderKind::Claude, "/repo/worktree", 10)
        .await
        .unwrap();

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].provider_session_id, "claude-1");
    assert_eq!(entries[0].worktree_path, "/repo/worktree");
}

#[tokio::test]
async fn test_provider_agent_session_history_gateway_codexのmetadata_dbをlimit付きで読む() {
    let directory = tempfile::tempdir().unwrap();
    let claude_root = directory.path().join("claude");
    let codex_root = directory.path().join("codex");
    fs::create_dir_all(&codex_root).unwrap();
    let connection = rusqlite::Connection::open(codex_root.join("state_5.sqlite")).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE threads (id TEXT PRIMARY KEY, cwd TEXT, updated_at INTEGER);\
             INSERT INTO threads VALUES ('codex-old', '/repo/worktree', 10);\
             INSERT INTO threads VALUES ('codex-new', '/repo/worktree', 20);\
             INSERT INTO threads VALUES ('codex-other', '/other', 30);",
        )
        .unwrap();
    drop(connection);
    let gateway = LocalProviderAgentSessionHistoryGateway::new(claude_root, codex_root);

    let entries = gateway
        .list_metadata(ProviderKind::Codex, "/repo/worktree", 1)
        .await
        .unwrap();

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].provider_session_id, "codex-new");
    assert_eq!(entries[0].updated_at_ms, 20_000);
}
