use std::fs;
use std::sync::Arc;

use super::{LocalAgentSessionHistoryGateway, LocalAgentSessionHistoryQueryService};
use crate::domain::agent_session::{
    AgentSessionHistoryGateway, AgentSessionHistoryGatewayError, AgentSessionHistoryMetadata,
    AgentSessionOwnershipQuery, ProviderSessionTitleEntry, ProviderSessionTitleGateway,
    ProviderSessionTitleRequest,
};
use crate::domain::provider_lifecycle::ProviderKind;
use crate::usecase::agent_session::{AgentSessionHistoryQueryService, AgentSessionHistoryRequest};

struct FixedMetadataHistoryGateway {
    inner: Arc<LocalAgentSessionHistoryGateway>,
    metadata: Vec<AgentSessionHistoryMetadata>,
}

#[async_trait::async_trait]
impl AgentSessionHistoryGateway for FixedMetadataHistoryGateway {
    async fn list_metadata(
        &self,
        provider: ProviderKind,
        worktree_path: &str,
        limit: usize,
    ) -> Result<Vec<AgentSessionHistoryMetadata>, AgentSessionHistoryGatewayError> {
        Ok(self
            .metadata
            .iter()
            .filter(|entry| entry.provider == provider && entry.worktree_path == worktree_path)
            .take(limit)
            .cloned()
            .collect())
    }

    async fn list_session_titles(
        &self,
        provider: ProviderKind,
        worktree_path: &str,
        provider_session_ids: &[String],
    ) -> Result<Vec<ProviderSessionTitleEntry>, AgentSessionHistoryGatewayError> {
        self.inner
            .list_session_titles(provider, worktree_path, provider_session_ids)
            .await
    }
}

struct UnownedProviderSessions;

#[async_trait::async_trait]
impl AgentSessionOwnershipQuery for UnownedProviderSessions {
    async fn is_owned(
        &self,
        _provider: ProviderKind,
        _provider_session_id: &str,
    ) -> Result<bool, AgentSessionHistoryGatewayError> {
        Ok(false)
    }
}

#[tokio::test]
async fn test_agent_session_history_gateway_claudeのmetadataだけをboundedに読む() {
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
    let gateway = LocalAgentSessionHistoryGateway::new(claude_root, codex_root);

    let entries = gateway
        .list_metadata(ProviderKind::Claude, "/repo/worktree", 10)
        .await
        .unwrap();

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].provider_session_id, "claude-1");
    assert_eq!(entries[0].worktree_path, "/repo/worktree");
}

#[tokio::test]
async fn test_agent_session_history_gateway_codexのmetadata_dbをlimit付きで読む() {
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
    let gateway = LocalAgentSessionHistoryGateway::new(claude_root, codex_root);

    let entries = gateway
        .list_metadata(ProviderKind::Codex, "/repo/worktree", 1)
        .await
        .unwrap();

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].provider_session_id, "codex-new");
    assert_eq!(entries[0].updated_at_ms, 20_000);
}

#[tokio::test]
async fn test_provider_session_title_gateway_claudeの末尾から最新ai_titleだけを読む() {
    let directory = tempfile::tempdir().unwrap();
    let claude_root = directory.path().join("claude");
    let codex_root = directory.path().join("codex");
    let project = claude_root.join("projects").join("-repo-worktree");
    fs::create_dir_all(&project).unwrap();
    fs::write(
        project.join("claude-title.jsonl"),
        concat!(
            "{\"type\":\"ai-title\",\"aiTitle\":\"Old title\",\"sessionId\":\"claude-title\"}\n",
            "{\"type\":\"custom-title\",\"customTitle\":\"Must be ignored\"}\n",
            "{\"type\":\"user\",\"message\":\"Must not be interpreted\"}\n",
            "{\"type\":\"ai-title\",\"aiTitle\":\"Current title\",\"sessionId\":\"claude-title\"}\n",
        ),
    )
    .unwrap();
    let gateway = LocalAgentSessionHistoryGateway::new(claude_root, codex_root);

    let title = gateway
        .read_title(ProviderSessionTitleRequest {
            provider: ProviderKind::Claude,
            provider_session_id: "claude-title".to_string(),
            worktree_path: "/repo/worktree".to_string(),
            transcript_ref: None,
        })
        .await
        .unwrap();

    assert_eq!(title.as_deref(), Some("Current title"));
}

#[tokio::test]
async fn test_provider_session_title_gateway_claudeは先頭の不完全行を捨て末尾64kibだけを読む() {
    let directory = tempfile::tempdir().unwrap();
    let claude_root = directory.path().join("claude");
    let codex_root = directory.path().join("codex");
    fs::create_dir_all(&claude_root).unwrap();
    let transcript = claude_root.join("bounded.jsonl");
    let mut contents = format!(
        "{{\"type\":\"ai-title\",\"aiTitle\":\"Outside window\"}}\n{{\"type\":\"user\",\"message\":\"{}\"}}\n",
        "x".repeat(70 * 1024)
    );
    contents.push_str("{\"type\":\"ai-title\",\"aiTitle\":\"Inside window\"}\n");
    fs::write(&transcript, contents).unwrap();
    let gateway = LocalAgentSessionHistoryGateway::new(claude_root, codex_root);

    let title = gateway
        .read_title(ProviderSessionTitleRequest {
            provider: ProviderKind::Claude,
            provider_session_id: "bounded".to_string(),
            worktree_path: "/repo/worktree".to_string(),
            transcript_ref: Some(transcript.to_string_lossy().into_owned()),
        })
        .await
        .unwrap();

    assert_eq!(title.as_deref(), Some("Inside window"));
}

#[tokio::test]
async fn test_provider_session_title_gateway_claudeは完全な先頭境界行を捨てない() {
    let directory = tempfile::tempdir().unwrap();
    let claude_root = directory.path().join("claude");
    let codex_root = directory.path().join("codex");
    fs::create_dir_all(&claude_root).unwrap();
    let title_line = b"{\"type\":\"ai-title\",\"aiTitle\":\"Boundary title\"}\n";
    let user_prefix = b"{\"type\":\"user\",\"message\":\"";
    let user_suffix = b"\"}\n";
    let padding_len = 64 * 1024 - title_line.len() - user_prefix.len() - user_suffix.len();
    let mut window = Vec::with_capacity(64 * 1024);
    window.extend_from_slice(title_line);
    window.extend_from_slice(user_prefix);
    window.extend(std::iter::repeat_n(b'x', padding_len));
    window.extend_from_slice(user_suffix);
    assert_eq!(window.len(), 64 * 1024);
    let exact_transcript = claude_root.join("exact-window.jsonl");
    fs::write(&exact_transcript, &window).unwrap();
    let boundary_transcript = claude_root.join("boundary-window.jsonl");
    let mut boundary_contents = b"{\"type\":\"user\",\"message\":\"before\"}\n".to_vec();
    boundary_contents.extend_from_slice(&window);
    fs::write(&boundary_transcript, boundary_contents).unwrap();
    let gateway = LocalAgentSessionHistoryGateway::new(claude_root, codex_root);

    for transcript in [exact_transcript, boundary_transcript] {
        let title = gateway
            .read_title(ProviderSessionTitleRequest {
                provider: ProviderKind::Claude,
                provider_session_id: "boundary".to_string(),
                worktree_path: "/repo/worktree".to_string(),
                transcript_ref: Some(transcript.to_string_lossy().into_owned()),
            })
            .await
            .unwrap();

        assert_eq!(title.as_deref(), Some("Boundary title"));
    }
}

#[tokio::test]
async fn test_provider_session_title_gateway_claudeはai_titleが末尾64kib外なら未取得を返す() {
    let directory = tempfile::tempdir().unwrap();
    let claude_root = directory.path().join("claude");
    let codex_root = directory.path().join("codex");
    fs::create_dir_all(&claude_root).unwrap();
    let transcript = claude_root.join("outside.jsonl");
    let contents = format!(
        "{{\"type\":\"ai-title\",\"aiTitle\":\"Outside window\"}}\n{{\"type\":\"user\",\"message\":\"{}\"}}\n",
        "x".repeat(70 * 1024)
    );
    fs::write(&transcript, contents).unwrap();
    let gateway = LocalAgentSessionHistoryGateway::new(claude_root, codex_root);

    let title = gateway
        .read_title(ProviderSessionTitleRequest {
            provider: ProviderKind::Claude,
            provider_session_id: "outside".to_string(),
            worktree_path: "/repo/worktree".to_string(),
            transcript_ref: Some(transcript.to_string_lossy().into_owned()),
        })
        .await
        .unwrap();

    assert_eq!(title, None);
}

#[tokio::test]
async fn test_provider_session_title_gateway_codexはthreads_nameをread_onlyで読みtitleを使わない() {
    let directory = tempfile::tempdir().unwrap();
    let claude_root = directory.path().join("claude");
    let codex_root = directory.path().join("codex");
    fs::create_dir_all(&codex_root).unwrap();
    let connection = rusqlite::Connection::open(codex_root.join("state_5.sqlite")).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                cwd TEXT,
                updated_at INTEGER,
                name TEXT,
                title TEXT,
                first_user_message TEXT
             );
             INSERT INTO threads VALUES (
                'codex-named', '/repo/worktree', 10, 'Thread name', 'First user message',
                'First user message'
             );
             INSERT INTO threads VALUES (
                'codex-empty', '/repo/worktree', 20, '', 'Must not be used', NULL
             );
             INSERT INTO threads VALUES (
                'codex-null', '/repo/worktree', 30, NULL, 'Must not be used', NULL
             );",
        )
        .unwrap();
    drop(connection);
    let gateway = LocalAgentSessionHistoryGateway::new(claude_root, codex_root);

    let named = gateway
        .read_title(ProviderSessionTitleRequest {
            provider: ProviderKind::Codex,
            provider_session_id: "codex-named".to_string(),
            worktree_path: "/repo/worktree".to_string(),
            transcript_ref: Some("/must/not/read/rollout.jsonl".to_string()),
        })
        .await
        .unwrap();
    let empty = gateway
        .read_title(ProviderSessionTitleRequest {
            provider: ProviderKind::Codex,
            provider_session_id: "codex-empty".to_string(),
            worktree_path: "/repo/worktree".to_string(),
            transcript_ref: None,
        })
        .await
        .unwrap();
    let null = gateway
        .read_title(ProviderSessionTitleRequest {
            provider: ProviderKind::Codex,
            provider_session_id: "codex-null".to_string(),
            worktree_path: "/repo/worktree".to_string(),
            transcript_ref: None,
        })
        .await
        .unwrap();

    assert_eq!(named.as_deref(), Some("Thread name"));
    assert_eq!(empty, None);
    assert_eq!(null, None);
}

#[tokio::test]
async fn test_agent_session_history_gateway_指定した可視idだけのタイトルを返す() {
    let directory = tempfile::tempdir().unwrap();
    let claude_root = directory.path().join("claude");
    let codex_root = directory.path().join("codex");
    let project = claude_root.join("projects").join("-repo-worktree");
    fs::create_dir_all(&project).unwrap();
    fs::create_dir_all(&codex_root).unwrap();
    fs::write(
        project.join("claude-visible.jsonl"),
        concat!(
            "{\"type\":\"user\",\"isMeta\":true,\"message\":{\"content\":\"Must be ignored\"}}\n",
            "{\"type\":\"user\",\"message\":{\"content\":\"Claude first prompt\"}}\n",
            "{\"type\":\"user\",\"message\":{\"content\":\"Must not replace first prompt\"}}\n",
            "{\"type\":\"ai-title\",\"aiTitle\":\"Claude visible title\"}\n",
        ),
    )
    .unwrap();
    fs::write(
        project.join("claude-not-visible.jsonl"),
        "{\"type\":\"ai-title\",\"aiTitle\":\"Must not be returned\"}\n",
    )
    .unwrap();
    let connection = rusqlite::Connection::open(codex_root.join("state_5.sqlite")).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                cwd TEXT,
                updated_at INTEGER,
                name TEXT,
                first_user_message TEXT
             );
             INSERT INTO threads VALUES (
                'codex-visible', '/repo/worktree', 10, 'Codex visible title',
                'Codex first prompt'
             );
             INSERT INTO threads VALUES (
                'codex-not-visible', '/repo/worktree', 20, 'Must not be returned',
                'Must not be returned'
             );",
        )
        .unwrap();
    drop(connection);
    let gateway = LocalAgentSessionHistoryGateway::new(claude_root, codex_root);

    let claude = gateway
        .list_session_titles(
            ProviderKind::Claude,
            "/repo/worktree",
            &["claude-visible".to_string()],
        )
        .await
        .unwrap();
    let codex = gateway
        .list_session_titles(
            ProviderKind::Codex,
            "/repo/worktree",
            &["codex-visible".to_string()],
        )
        .await
        .unwrap();

    assert_eq!(claude.len(), 1);
    assert_eq!(claude[0].provider_session_id, "claude-visible");
    assert_eq!(
        claude[0].session_title.as_deref(),
        Some("Claude visible title")
    );
    assert_eq!(
        claude[0].first_user_prompt.as_deref(),
        Some("Claude first prompt")
    );
    assert_eq!(codex.len(), 1);
    assert_eq!(codex[0].provider_session_id, "codex-visible");
    assert_eq!(
        codex[0].session_title.as_deref(),
        Some("Codex visible title")
    );
    assert_eq!(
        codex[0].first_user_prompt.as_deref(),
        Some("Codex first prompt")
    );
}

#[tokio::test]
async fn test_agent_session_history_gateway_claudeの先頭64kib外のプロンプトを読まない() {
    let directory = tempfile::tempdir().unwrap();
    let claude_root = directory.path().join("claude");
    let codex_root = directory.path().join("codex");
    let project = claude_root.join("projects").join("-repo-worktree");
    fs::create_dir_all(&project).unwrap();
    let mut transcript = format!(
        "{{\"type\":\"queue-operation\",\"payload\":\"{}\"}}\n",
        "x".repeat(70 * 1024)
    );
    transcript.push_str(concat!(
        "{\"type\":\"user\",\"message\":{\"content\":\"Outside head window\"}}\n",
        "{\"type\":\"ai-title\",\"aiTitle\":\"Title in tail\"}\n",
    ));
    fs::write(project.join("claude-bounded.jsonl"), transcript).unwrap();
    let gateway = LocalAgentSessionHistoryGateway::new(claude_root, codex_root);

    let entries = gateway
        .list_session_titles(
            ProviderKind::Claude,
            "/repo/worktree",
            &["claude-bounded".to_string()],
        )
        .await
        .unwrap();

    assert_eq!(entries[0].session_title.as_deref(), Some("Title in tail"));
    assert_eq!(entries[0].first_user_prompt, None);
}

#[tokio::test]
async fn test_agent_session_history_gateway_claudeの一部が読めなくても他のタイトルを返す() {
    let directory = tempfile::tempdir().unwrap();
    let claude_root = directory.path().join("claude");
    let codex_root = directory.path().join("codex");
    let project = claude_root.join("projects").join("-repo-worktree");
    fs::create_dir_all(&project).unwrap();
    fs::write(
        project.join("claude-readable.jsonl"),
        "{\"type\":\"ai-title\",\"aiTitle\":\"Readable title\"}\n",
    )
    .unwrap();
    fs::write(project.join("claude-corrupt.jsonl"), "not-json\n").unwrap();
    let gateway = LocalAgentSessionHistoryGateway::new(claude_root, codex_root);

    let entries = gateway
        .list_session_titles(
            ProviderKind::Claude,
            "/repo/worktree",
            &[
                "claude-missing".to_string(),
                "claude-corrupt".to_string(),
                "claude-readable".to_string(),
            ],
        )
        .await
        .unwrap();

    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].provider_session_id, "claude-missing");
    assert_eq!(entries[0].session_title, None);
    assert_eq!(entries[1].provider_session_id, "claude-corrupt");
    assert_eq!(entries[1].session_title, None);
    assert_eq!(entries[2].provider_session_id, "claude-readable");
    assert_eq!(entries[2].session_title.as_deref(), Some("Readable title"));
}

#[tokio::test]
async fn test_agent_session_history_query_claudeの一部が読めなくても同じpageへ返す() {
    let directory = tempfile::tempdir().unwrap();
    let claude_root = directory.path().join("claude");
    let codex_root = directory.path().join("codex");
    let project = claude_root.join("projects").join("-repo-worktree");
    fs::create_dir_all(&project).unwrap();
    fs::write(project.join("claude-corrupt.jsonl"), "not-json\n").unwrap();
    fs::write(
        project.join("claude-readable.jsonl"),
        "{\"type\":\"ai-title\",\"aiTitle\":\"Readable title\"}\n",
    )
    .unwrap();
    let local_gateway = Arc::new(LocalAgentSessionHistoryGateway::new(
        claude_root,
        codex_root,
    ));
    let query = LocalAgentSessionHistoryQueryService::new(
        Arc::new(FixedMetadataHistoryGateway {
            inner: local_gateway,
            metadata: vec![
                metadata(ProviderKind::Claude, "claude-missing", 30),
                metadata(ProviderKind::Claude, "claude-corrupt", 20),
                metadata(ProviderKind::Claude, "claude-readable", 10),
            ],
        }),
        Arc::new(UnownedProviderSessions),
    );

    let page = query
        .list(AgentSessionHistoryRequest {
            worktree_path: "/repo/worktree".to_string(),
            limit: 3,
            after: None,
        })
        .await
        .unwrap();

    assert_eq!(
        page.items
            .iter()
            .map(|item| (item.provider_session_id.as_str(), item.label.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("claude-missing", "Claude claude-m…"),
            ("claude-corrupt", "Claude claude-c…"),
            ("claude-readable", "Readable title"),
        ]
    );
}

#[tokio::test]
async fn test_agent_session_history_gateway_codexのdbが無くても全idを未取得で返す() {
    let directory = tempfile::tempdir().unwrap();
    let gateway = LocalAgentSessionHistoryGateway::new(
        directory.path().join("claude"),
        directory.path().join("codex"),
    );

    let entries = gateway
        .list_session_titles(
            ProviderKind::Codex,
            "/repo/worktree",
            &["codex-1".to_string(), "codex-2".to_string()],
        )
        .await
        .unwrap();

    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].provider_session_id, "codex-1");
    assert_eq!(entries[0].session_title, None);
    assert_eq!(entries[1].provider_session_id, "codex-2");
    assert_eq!(entries[1].session_title, None);
}

#[tokio::test]
async fn test_agent_session_history_query_codexのdbが無くてもfallbackラベルを返す() {
    let directory = tempfile::tempdir().unwrap();
    let local_gateway = Arc::new(LocalAgentSessionHistoryGateway::new(
        directory.path().join("claude"),
        directory.path().join("codex"),
    ));
    let query = LocalAgentSessionHistoryQueryService::new(
        Arc::new(FixedMetadataHistoryGateway {
            inner: local_gateway,
            metadata: vec![
                metadata(ProviderKind::Codex, "codex-123456", 20),
                metadata(ProviderKind::Codex, "codex-abcdef", 10),
            ],
        }),
        Arc::new(UnownedProviderSessions),
    );

    let page = query
        .list(AgentSessionHistoryRequest {
            worktree_path: "/repo/worktree".to_string(),
            limit: 2,
            after: None,
        })
        .await
        .unwrap();

    assert_eq!(
        page.items
            .iter()
            .map(|item| (item.provider_session_id.as_str(), item.label.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("codex-123456", "Codex codex-12…"),
            ("codex-abcdef", "Codex codex-ab…"),
        ]
    );
}

#[tokio::test]
async fn test_agent_session_history_gateway_不正なタイトル要求だけを拒否する() {
    let directory = tempfile::tempdir().unwrap();
    let gateway = LocalAgentSessionHistoryGateway::new(
        directory.path().join("claude"),
        directory.path().join("codex"),
    );

    let empty_worktree = gateway
        .list_session_titles(ProviderKind::Claude, " ", &["claude-1".to_string()])
        .await;
    let empty_id = gateway
        .list_session_titles(ProviderKind::Codex, "/repo/worktree", &[" ".to_string()])
        .await;

    assert_eq!(
        empty_worktree,
        Err(crate::domain::agent_session::AgentSessionHistoryGatewayError::InvalidRequest)
    );
    assert_eq!(
        empty_id,
        Err(crate::domain::agent_session::AgentSessionHistoryGatewayError::InvalidRequest)
    );
}

fn metadata(
    provider: ProviderKind,
    provider_session_id: &str,
    updated_at_ms: i64,
) -> AgentSessionHistoryMetadata {
    AgentSessionHistoryMetadata {
        provider,
        provider_session_id: provider_session_id.to_string(),
        worktree_path: "/repo/worktree".to_string(),
        updated_at_ms,
    }
}
