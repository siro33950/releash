pub(crate) mod tests {

    use releash_lib::test_support::integration::workflow::SessionExecutionTreeRootFacts;

    use crate::cli_common_test_support::write_review_config;
    use crate::cli_common_test_support::write_review_session;
    use crate::cli_common_test_support::write_review_session_with_lifecycle;
    use releash_lib::test_support::integration::platform::build_review_comment_usecase;
    use releash_lib::test_support::integration::platform::cli_error_exit_code;
    use releash_lib::test_support::integration::platform::cli_error_stderr;
    use releash_lib::test_support::integration::platform::state_file;
    use releash_lib::test_support::integration::platform::test_uuid;
    use releash_lib::test_support::integration::platform::CliError;
    use releash_lib::test_support::integration::platform::ReviewActor;
    use releash_lib::test_support::integration::platform::ReviewThreadDto;
    use releash_lib::test_support::integration::platform::ReviewThreadFilter;
    use releash_lib::test_support::integration::platform::ReviewThreadState;
    use releash_lib::test_support::integration::review::cmd_review;
    use releash_lib::test_support::integration::review::review_actor;
    use releash_lib::test_support::integration::review::review_actor_and_worktree;
    use releash_lib::test_support::integration::review::review_list_actor_and_worktree;
    use releash_lib::test_support::integration::review::review_worktree_from_session;
    use releash_lib::test_support::integration::review::ReviewSubcommand;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    fn seed_review_thread(data_dir: &Path) -> String {
        let thread_id = test_uuid(42);
        let event_id = test_uuid(44);
        let comment_id = test_uuid(43);
        let path = state_file(data_dir, "/repo");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            path,
            format!(
                r#"[
  {{
    "eventType": "thread_created",
    "eventId": "{event_id}",
    "threadId": "{thread_id}",
    "commentId": "{comment_id}",
    "actor": {{
      "kind": "agent",
      "backendId": "codex",
      "model": "gpt-5",
      "sessionId": null,
      "displayName": "codex/gpt-5"
    }},
    "target": {{
      "filePath": "src/main.rs",
      "lineNumber": 3,
      "endLine": 5
    }},
    "content": "Claim",
    "at": 10.0
  }}
]"#
            ),
        )
        .unwrap();
        thread_id
    }

    fn human_line_value<'a>(output: &'a str, prefix: &str) -> &'a str {
        output
            .lines()
            .find_map(|line| line.strip_prefix(prefix))
            .unwrap_or_else(|| panic!("missing human output line with prefix: {prefix}"))
    }

    fn json_string(value: &serde_json::Value, pointer: &str) -> String {
        value
            .pointer(pointer)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_else(|| panic!("missing JSON string at pointer: {pointer}"))
            .to_string()
    }

    /// 実際の JSON 出力テキストから数値リテラルの生トークンを抽出する。
    ///
    /// `serde_json::Value` を経由して再シリアライズすると、`float_roundtrip`
    /// feature 無効時に f64 の parse 精度が往復保証されず、実出力と桁数が
    /// ずれることがある（実時刻タイムスタンプで顕在化）。golden 比較では
    /// 実出力の生トークンをそのまま埋め込み、この不一致を避ける。
    fn json_raw_number(json_text: &str, key: &str) -> String {
        let needle = format!("\"{key}\": ");
        let start = json_text
            .find(&needle)
            .unwrap_or_else(|| panic!("missing JSON key: {key}"))
            + needle.len();
        let rest = &json_text[start..];
        let end = rest.find([',', '\n']).unwrap_or(rest.len());
        rest[..end].trim().to_string()
    }

    #[tokio::test]
    pub async fn review_list_get_handler_outputs_match_split_before_golden() {
        let tmp = TempDir::new().unwrap();
        write_review_config(tmp.path());
        let session_id = "550e8400-e29b-41d4-a716-446655440061".to_string();
        write_review_session(tmp.path(), &session_id, Some("codex")).await;
        let thread_id = seed_review_thread(tmp.path());
        let comment_id = test_uuid(43);

        let list_human = cmd_review(
            tmp.path(),
            ReviewSubcommand::List {
                session_id: Some(session_id.clone()),
                file: None,
                state: None,
                author: None,
                unread: None,
                thread_id: Vec::new(),
                json: false,
            },
        )
        .await
        .unwrap();
        assert_eq!(
            list_human,
            format!(
                "{:<36}  {:<9}  {:<20}  UPDATED\n{:<36}  {:<9}  {:<20}  {}\n",
                "THREAD_ID", "STATE", "AUTHOR", thread_id, "open", "codex/gpt-5", "10"
            )
        );

        let list_json = cmd_review(
            tmp.path(),
            ReviewSubcommand::List {
                session_id: Some(session_id.clone()),
                file: None,
                state: None,
                author: None,
                unread: None,
                thread_id: Vec::new(),
                json: true,
            },
        )
        .await
        .unwrap();
        assert_eq!(
            list_json,
            format!(
                r#"[
  {{
    "id": "{thread_id}",
    "worktreeName": "/repo",
    "author": {{
      "kind": "agent",
      "backendId": "codex",
      "model": "gpt-5",
      "displayName": "codex/gpt-5"
    }},
    "target": {{
      "filePath": "src/main.rs",
      "lineNumber": 3,
      "endLine": 5
    }},
    "state": "open",
    "comments": [
      {{
        "id": "{comment_id}",
        "threadId": "{thread_id}",
        "author": {{
          "kind": "agent",
          "backendId": "codex",
          "model": "gpt-5",
          "displayName": "codex/gpt-5"
        }},
        "content": "Claim",
        "createdAt": 10.0
      }}
    ],
    "resolve": null,
    "createdAt": 10.0,
    "updatedAt": 10.0,
    "version": 1,
    "canResolve": true
  }}
]
"#
            )
        );

        let get_human = cmd_review(
            tmp.path(),
            ReviewSubcommand::Get {
                thread_id: thread_id.clone(),
                session_id: session_id.clone(),
                json: false,
            },
        )
        .await
        .unwrap();
        assert_eq!(
            get_human,
            format!(
                "thread_id: {thread_id}\nstate:     Open\nauthor:    codex/gpt-5\nlocation:  src/main.rs:L3-L5\nupdated:   10\ncomments:  1\n"
            )
        );

        let get_json = cmd_review(
            tmp.path(),
            ReviewSubcommand::Get {
                thread_id: thread_id.clone(),
                session_id,
                json: true,
            },
        )
        .await
        .unwrap();
        assert_eq!(
            get_json,
            format!(
                r#"{{
  "id": "{thread_id}",
  "worktreeName": "/repo",
  "author": {{
    "kind": "agent",
    "backendId": "codex",
    "model": "gpt-5",
    "displayName": "codex/gpt-5"
  }},
  "target": {{
    "filePath": "src/main.rs",
    "lineNumber": 3,
    "endLine": 5
  }},
  "state": "open",
  "comments": [
    {{
      "id": "{comment_id}",
      "threadId": "{thread_id}",
      "author": {{
        "kind": "agent",
        "backendId": "codex",
        "model": "gpt-5",
        "displayName": "codex/gpt-5"
      }},
      "content": "Claim",
      "createdAt": 10.0
    }}
  ],
  "resolve": null,
  "createdAt": 10.0,
  "updatedAt": 10.0,
  "version": 1,
  "canResolve": true
}}
"#
            )
        );
    }

    #[tokio::test]
    pub async fn review_create_handler_outputs_match_split_before_golden() {
        let tmp = TempDir::new().unwrap();
        write_review_config(tmp.path());
        let session_id = "550e8400-e29b-41d4-a716-446655440062".to_string();
        write_review_session(tmp.path(), &session_id, Some("codex")).await;

        let human = cmd_review(
            tmp.path(),
            ReviewSubcommand::Create {
                session_id: session_id.clone(),
                content: "Claim".to_string(),
                file: Some("src/main.rs".to_string()),
                line: Some(3),
                end_line: Some(5),
                json: false,
            },
        )
        .await
        .unwrap();
        let human_thread_id = human_line_value(&human, "thread_id: ");
        let human_updated = human_line_value(&human, "updated:   ");
        assert_eq!(
            human,
            format!(
                "thread_id: {human_thread_id}\nstate:     Open\nauthor:    codex\nlocation:  src/main.rs:L3-L5\nupdated:   {human_updated}\ncomments:  1\n"
            )
        );

        let json = cmd_review(
            tmp.path(),
            ReviewSubcommand::Create {
                session_id,
                content: "Claim".to_string(),
                file: Some("src/main.rs".to_string()),
                line: Some(3),
                end_line: Some(5),
                json: true,
            },
        )
        .await
        .unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let json_thread_id = json_string(&value, "/id");
        let json_comment_id = json_string(&value, "/comments/0/id");
        let json_created_at = json_raw_number(&json, "createdAt");
        let json_updated_at = json_raw_number(&json, "updatedAt");
        let json_comment_created_at = json_raw_number(&json, "createdAt");
        assert_eq!(json_created_at, json_updated_at);
        assert_eq!(json_created_at, json_comment_created_at);
        assert_eq!(
            json,
            format!(
                r#"{{
  "id": "{json_thread_id}",
  "worktreeName": "/repo",
  "author": {{
    "kind": "agent",
    "backendId": "codex",
    "model": null,
    "displayName": "codex"
  }},
  "target": {{
    "filePath": "src/main.rs",
    "lineNumber": 3,
    "endLine": 5
  }},
  "state": "open",
  "comments": [
    {{
      "id": "{json_comment_id}",
      "threadId": "{json_thread_id}",
      "author": {{
        "kind": "agent",
        "backendId": "codex",
        "model": null,
        "displayName": "codex"
      }},
      "content": "Claim",
      "createdAt": {json_created_at}
    }}
  ],
  "resolve": null,
  "createdAt": {json_created_at},
  "updatedAt": {json_updated_at},
  "version": 1,
  "canResolve": true
}}
"#
            )
        );
    }

    #[tokio::test]
    pub async fn review_handler_error_stderr_and_exit_codes_match_split_before_golden() {
        let tmp = TempDir::new().unwrap();
        write_review_config(tmp.path());
        let session_id = "550e8400-e29b-41d4-a716-446655440063".to_string();
        write_review_session(tmp.path(), &session_id, Some("codex")).await;

        let invalid_state = cmd_review(
            tmp.path(),
            ReviewSubcommand::List {
                session_id: Some(session_id.clone()),
                file: None,
                state: Some("paused".to_string()),
                author: None,
                unread: None,
                thread_id: Vec::new(),
                json: false,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(
            cli_error_stderr(&invalid_state),
            "error: Invalid --state value: paused (expected: open | resolved)"
        );
        assert_eq!(cli_error_exit_code(&invalid_state), 2);

        let missing_thread_id = test_uuid(99);
        let missing_thread = cmd_review(
            tmp.path(),
            ReviewSubcommand::Get {
                thread_id: missing_thread_id.clone(),
                session_id: session_id.clone(),
                json: false,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(
            cli_error_stderr(&missing_thread),
            format!("Review thread not found: {missing_thread_id}")
        );
        assert_eq!(cli_error_exit_code(&missing_thread), 4);

        let invalid_target = cmd_review(
            tmp.path(),
            ReviewSubcommand::Create {
                session_id: session_id.clone(),
                content: "Bad target".to_string(),
                file: Some("../secret".to_string()),
                line: Some(1),
                end_line: None,
                json: true,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(
            cli_error_stderr(&invalid_target),
            "error: file_path must not contain root, prefix, '.', or '..' components"
        );
        assert_eq!(cli_error_exit_code(&invalid_target), 2);

        let archived_session_id = uuid::Uuid::new_v4().to_string();
        write_review_session_with_lifecycle(
            tmp.path(),
            &archived_session_id,
            Some("codex"),
            releash_lib::test_support::integration::sessions::AgentSessionLifecycle::Archived,
        )
        .await;
        let closed_session = cmd_review(
            tmp.path(),
            ReviewSubcommand::Create {
                session_id: archived_session_id.clone(),
                content: "Claim".to_string(),
                file: None,
                line: None,
                end_line: None,
                json: false,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(
            cli_error_stderr(&closed_session),
            format!(
                "error: Session is not open and cannot be used as a review actor: {archived_session_id}"
            )
        );
        assert_eq!(cli_error_exit_code(&closed_session), 2);
    }

    #[tokio::test]
    pub async fn review_actor_resolves_provider_from_canonical_agent_session() {
        let tmp = TempDir::new().unwrap();
        write_review_config(tmp.path());
        let session_id = uuid::Uuid::new_v4().to_string();
        write_review_session(tmp.path(), &session_id, Some("codex")).await;

        let actor = review_actor(tmp.path(), &session_id).await.unwrap();

        assert_eq!(actor.backend_id.as_deref(), Some("codex"));
        assert_eq!(actor.model, None);
        assert_eq!(actor.session_id.as_deref(), Some(session_id.as_str()));
    }

    #[tokio::test]
    pub async fn review_actor_uses_canonical_provider_without_model_catalog() {
        let tmp = TempDir::new().unwrap();
        write_review_config(tmp.path());

        let missing = review_actor(tmp.path(), &uuid::Uuid::new_v4().to_string()).await;
        assert!(matches!(missing, Err(CliError::NotFound(_))));

        for provider in ["codex", "claude"] {
            let session_id = uuid::Uuid::new_v4().to_string();
            write_review_session(tmp.path(), &session_id, Some(provider)).await;
            let actor = review_actor(tmp.path(), &session_id).await.unwrap();
            assert_eq!(actor.backend_id.as_deref(), Some(provider));
            assert_eq!(actor.model, None);
        }
    }

    #[tokio::test]
    pub async fn review_actor_and_worktree_rejects_empty_session_id() {
        let tmp = TempDir::new().unwrap();

        for session_id in ["", " "] {
            let err = review_actor_and_worktree(tmp.path(), session_id)
                .await
                .unwrap_err();
            assert!(matches!(err, CliError::InvalidInput(_)));
        }
    }

    #[tokio::test]
    pub async fn review_worktree_from_session_rejects_empty_session_id() {
        let tmp = TempDir::new().unwrap();

        for session_id in ["", " "] {
            let err = review_worktree_from_session(tmp.path(), session_id)
                .await
                .unwrap_err();
            assert!(matches!(err, CliError::InvalidInput(_)));
        }
    }

    #[tokio::test]
    pub async fn review_worktree_resolution_allows_archived_session() {
        let tmp = TempDir::new().unwrap();
        write_review_config(tmp.path());
        let session_id = uuid::Uuid::new_v4().to_string();
        write_review_session_with_lifecycle(
            tmp.path(),
            &session_id,
            Some("codex"),
            releash_lib::test_support::integration::sessions::AgentSessionLifecycle::Archived,
        )
        .await;

        let worktree = review_worktree_from_session(tmp.path(), &session_id)
            .await
            .unwrap();

        assert_eq!(worktree, "/repo");
    }

    #[tokio::test]
    pub async fn review_list_allows_paused_and_archived_sessions() {
        let tmp = TempDir::new().unwrap();
        write_review_config(tmp.path());
        let thread_id = seed_review_thread(tmp.path());

        for lifecycle in [
            releash_lib::test_support::integration::sessions::AgentSessionLifecycle::Paused,
            releash_lib::test_support::integration::sessions::AgentSessionLifecycle::Archived,
        ] {
            let session_id = uuid::Uuid::new_v4().to_string();
            write_review_session_with_lifecycle(tmp.path(), &session_id, Some("codex"), lifecycle)
                .await;

            let output = cmd_review(
                tmp.path(),
                ReviewSubcommand::List {
                    session_id: Some(session_id.clone()),
                    file: None,
                    state: Some("open".to_string()),
                    author: None,
                    unread: None,
                    thread_id: Vec::new(),
                    json: true,
                },
            )
            .await
            .unwrap();

            assert!(output.contains(&thread_id));

            let unread_output = cmd_review(
                tmp.path(),
                ReviewSubcommand::List {
                    session_id: Some(session_id),
                    file: None,
                    state: Some("open".to_string()),
                    author: None,
                    unread: Some("true".to_string()),
                    thread_id: Vec::new(),
                    json: true,
                },
            )
            .await
            .unwrap();

            assert!(unread_output.contains(&thread_id));
        }
    }

    #[tokio::test]
    pub async fn review_list_actor_filters_require_session_id() {
        let tmp = TempDir::new().unwrap();

        for (author, unread) in [
            (Some("self".to_string()), None),
            (None, Some("true".to_string())),
        ] {
            let error = cmd_review(
                tmp.path(),
                ReviewSubcommand::List {
                    session_id: None,
                    file: None,
                    state: None,
                    author,
                    unread,
                    thread_id: Vec::new(),
                    json: true,
                },
            )
            .await
            .unwrap_err();

            assert!(matches!(
                error,
                CliError::InvalidInput(message)
                    if message == "--session-id is required when --author or --unread is specified"
            ));
        }
    }

    #[tokio::test]
    pub async fn review_list_uses_worktree_environment_without_session() {
        let tmp = TempDir::new().unwrap();

        let (actor, worktree) =
            review_list_actor_and_worktree(tmp.path(), None, Some("/repo"), false)
                .await
                .unwrap();

        assert_eq!(actor, ReviewActor::human());
        assert_eq!(worktree, "/repo");
    }

    #[tokio::test]
    pub async fn review_cli_rejects_mutation_for_archived_session() {
        let tmp = TempDir::new().unwrap();
        write_review_config(tmp.path());
        let session_id = uuid::Uuid::new_v4().to_string();
        write_review_session_with_lifecycle(
            tmp.path(),
            &session_id,
            Some("codex"),
            releash_lib::test_support::integration::sessions::AgentSessionLifecycle::Archived,
        )
        .await;
        let closed = cmd_review(
            tmp.path(),
            ReviewSubcommand::Create {
                session_id,
                content: "Claim".to_string(),
                file: None,
                line: None,
                end_line: None,
                json: true,
            },
        )
        .await;
        match closed {
            Err(CliError::InvalidInput(msg)) => assert!(msg.contains("Session is not open")),
            other => panic!("expected archived session rejection, got {other:?}"),
        }
    }

    #[tokio::test]
    pub async fn cmd_review_create_list_get_and_json_mode_use_session_worktree_key() {
        let tmp = TempDir::new().unwrap();
        write_review_config(tmp.path());
        let session_id = uuid::Uuid::new_v4().to_string();
        write_review_session(tmp.path(), &session_id, Some("codex")).await;

        cmd_review(
            tmp.path(),
            ReviewSubcommand::Create {
                session_id: session_id.clone(),
                content: "Claim".to_string(),
                file: None,
                line: None,
                end_line: None,
                json: true,
            },
        )
        .await
        .unwrap();

        let usecase = build_review_comment_usecase();
        let threads = usecase
            .list_threads(tmp.path(), "/repo", None, ReviewActor::human())
            .unwrap();
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].worktree_name, "/repo");
        let json = serde_json::to_string(&ReviewThreadDto::from(&threads[0])).unwrap();
        assert!(!json.contains("sessionId"));

        cmd_review(
            tmp.path(),
            ReviewSubcommand::List {
                session_id: Some(session_id.clone()),
                file: None,
                state: Some("open".to_string()),
                author: None,
                unread: None,
                thread_id: Vec::new(),
                json: true,
            },
        )
        .await
        .unwrap();
        cmd_review(
            tmp.path(),
            ReviewSubcommand::Get {
                thread_id: threads[0].id.clone(),
                session_id,
                json: true,
            },
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    pub async fn cmd_review_comment_resolve_history_and_rejections_use_domain_reasons() {
        let tmp = TempDir::new().unwrap();
        write_review_config(tmp.path());
        let owner_session = uuid::Uuid::new_v4().to_string();
        let other_session = uuid::Uuid::new_v4().to_string();
        write_review_session(tmp.path(), &owner_session, Some("codex")).await;
        write_review_session(tmp.path(), &other_session, Some("claude")).await;

        cmd_review(
            tmp.path(),
            ReviewSubcommand::Create {
                session_id: owner_session.clone(),
                content: "Claim".to_string(),
                file: Some("src/main.rs".to_string()),
                line: Some(3),
                end_line: Some(5),
                json: true,
            },
        )
        .await
        .unwrap();
        let usecase = build_review_comment_usecase();
        let thread_id = usecase
            .list_threads(tmp.path(), "/repo", None, ReviewActor::human())
            .unwrap()[0]
            .id
            .clone();

        cmd_review(
            tmp.path(),
            ReviewSubcommand::Comment {
                thread_id: thread_id.clone(),
                session_id: owner_session.clone(),
                content: "Follow-up".to_string(),
                json: true,
            },
        )
        .await
        .unwrap();
        cmd_review(
            tmp.path(),
            ReviewSubcommand::Comment {
                thread_id: thread_id.clone(),
                session_id: owner_session.clone(),
                content: "Another follow-up".to_string(),
                json: true,
            },
        )
        .await
        .unwrap();
        cmd_review(
            tmp.path(),
            ReviewSubcommand::History {
                thread_id: thread_id.clone(),
                session_id: owner_session.clone(),
                json: true,
            },
        )
        .await
        .unwrap();

        // 別 backend/model session からの Resolve も participant identity に依らず成功する
        // (spec issues-1022: Resolve 権限は participant 識別に依存しない)。
        cmd_review(
            tmp.path(),
            ReviewSubcommand::Resolve {
                thread_id: thread_id.clone(),
                session_id: other_session,
                outcome: "accepted".to_string(),
                summary: "non-owner resolve".to_string(),
                json: true,
            },
        )
        .await
        .unwrap();
        // resolved 後の Resolve / Comment 追記は state により拒否される。
        let rejected_after_resolve = cmd_review(
            tmp.path(),
            ReviewSubcommand::Resolve {
                thread_id: thread_id.clone(),
                session_id: owner_session.clone(),
                outcome: "accepted".to_string(),
                summary: "second resolve".to_string(),
                json: true,
            },
        )
        .await;
        match rejected_after_resolve {
            Err(CliError::InvalidInput(msg)) => assert!(msg.contains("already resolved")),
            other => panic!("expected resolved rejection, got {other:?}"),
        }
        let rejected_late_comment = cmd_review(
            tmp.path(),
            ReviewSubcommand::Comment {
                thread_id: thread_id.clone(),
                session_id: owner_session.clone(),
                content: "late".to_string(),
                json: true,
            },
        )
        .await;
        match rejected_late_comment {
            Err(CliError::InvalidInput(msg)) => assert!(msg.contains("already resolved")),
            other => panic!("expected resolved rejection, got {other:?}"),
        }

        let missing_history = cmd_review(
            tmp.path(),
            ReviewSubcommand::History {
                thread_id: "missing-thread".to_string(),
                session_id: owner_session.clone(),
                json: true,
            },
        )
        .await;
        assert!(matches!(missing_history, Err(CliError::NotFound(_))));

        let invalid_target = cmd_review(
            tmp.path(),
            ReviewSubcommand::Create {
                session_id: owner_session,
                content: "Bad target".to_string(),
                file: Some("../secret".to_string()),
                line: Some(1),
                end_line: None,
                json: true,
            },
        )
        .await;
        assert!(matches!(invalid_target, Err(CliError::InvalidInput(_))));
    }

    #[tokio::test]
    pub async fn test_隔離thread参照_command環境とsession指定で同じworkspaceのopen一覧を読む() {
        // Given
        use releash_lib::test_support::integration::persistence::LocalEventStore;
        use releash_lib::test_support::integration::persistence::LocalEventStoreConfig;

        use releash_lib::test_support::integration::workflow::IsolatedWorktree;
        use releash_lib::test_support::integration::workflow::NodeFact;
        use releash_lib::test_support::integration::workflow::StartedFact;
        use releash_lib::test_support::integration::workflow::WorktreeMode;
        let tmp = TempDir::new().unwrap();
        write_review_config(tmp.path());
        let store = LocalEventStore::open(LocalEventStoreConfig::production(
            tmp.path().into(),
            std::sync::Arc::new(
                releash_lib::test_support::integration::platform::RetryLimiter::new(),
            ),
        ))
        .unwrap();
        let id = test_uuid(173);
        let mut facts = SessionExecutionTreeRootFacts::new(
            &id,
            "/repo",
            "/repo",
            releash_lib::test_support::integration::providers::ProviderKind::Codex,
            None,
        )
        .unwrap();
        let NodeFact::Started(StartedFact {
            worktree: None,
            root: Some(root),
            ..
        }) = &mut facts.started
        else {
            unreachable!();
        };
        root.repository_root = Some("/repo".into());
        root.definition.as_mut().unwrap().nodes[0].worktree = Some(WorktreeMode::Isolated);
        for (meta, fact) in facts.into_facts() {
            releash_lib::test_support::integration::workflow::append_single_fact(
                &store, &meta, &fact, 1,
            )
            .await
            .unwrap();
        }
        let thread_id = seed_review_thread(tmp.path());
        let path = IsolatedWorktree::for_attempt("/repo", &id, 1).path;

        // When
        let (actor, workspace) =
            review_list_actor_and_worktree(tmp.path(), None, Some(&path), false)
                .await
                .unwrap();
        let command_threads = build_review_comment_usecase()
            .list_threads(
                tmp.path(),
                &workspace,
                Some(ReviewThreadFilter {
                    state: Some(ReviewThreadState::Open),
                    ..Default::default()
                }),
                actor,
            )
            .unwrap();
        let session_output = cmd_review(
            tmp.path(),
            ReviewSubcommand::List {
                session_id: Some(id),
                file: None,
                state: Some("open".into()),
                author: None,
                unread: None,
                thread_id: Vec::new(),
                json: true,
            },
        )
        .await
        .unwrap();
        let session_threads: serde_json::Value = serde_json::from_str(&session_output).unwrap();

        // Then
        assert_eq!(workspace, "/repo");
        assert_eq!(command_threads.len(), 1);
        assert_eq!(command_threads[0].id, thread_id);
        assert_eq!(session_threads[0]["id"], thread_id);
        assert!(!state_file(tmp.path(), &path).exists());
    }
}
