use super::*;

#[test]
fn parse_open_prs_valid_json() {
    let json = r#"[
            {"headRefName":"feat/login","number":42,"url":"https://github.com/owner/repo/pull/42"},
            {"headRefName":"fix/typo","number":7,"url":"https://github.com/owner/repo/pull/7"}
        ]"#;

    let map = parse_gh_pr_list_output(json, PrState::Open).unwrap();

    assert_eq!(map.len(), 2);
    let pr = &map
        .iter()
        .find(|(branch, _)| branch == "feat/login")
        .unwrap()
        .1;
    assert_eq!(pr.number, 42);
    assert_eq!(pr.url, "https://github.com/owner/repo/pull/42");
}

#[test]
fn parse_open_prs_empty_array() {
    let map = parse_gh_pr_list_output("[]", PrState::Open).unwrap();

    assert!(map.is_empty());
}

#[test]
fn parse_open_prs_invalid_json() {
    assert!(parse_gh_pr_list_output("not json", PrState::Open).is_err());
}

#[test]
fn parse_open_prs_missing_fields() {
    let json = r#"[{"headRefName":"feat/x"}]"#;

    let map = parse_gh_pr_list_output(json, PrState::Open).unwrap();

    assert!(map.is_empty());
}

#[test]
fn test_pr状態_draftと完了状態を保持する() {
    // Given
    let json = r#"[{"headRefName":"feat","number":1,"url":"url","isDraft":true}]"#;
    // When / Then
    for state in [PrState::Open, PrState::Merged, PrState::Closed] {
        let parsed = parse_gh_pr_list_output(json, state).unwrap();
        assert_eq!(parsed[0].1.state, state);
        assert!(parsed[0].1.draft);
    }
}

#[test]
fn parse_issue_list_valid_json() {
    // Given
    let json = serde_json::json!([
        {
            "number": 305,
            "title": "Add issue panel",
            "state": "OPEN",
            "url": "https://github.com/owner/repo/issues/305",
            "author": {"login": "user1"},
            "createdAt": "2024-01-01T00:00:00Z",
            "updatedAt": "2024-01-02T00:00:00Z",
            "labels": [{"name": "enhancement", "color": "a2eeef"}],
            "assignees": [{"login": "user1"}],
            "body": "Issue body"
        },
        {
            "number": 100,
            "title": "Bug fix",
            "state": "OPEN",
            "url": "https://github.com/owner/repo/issues/100",
            "author": {"login": "user2"},
            "createdAt": "2024-01-01T00:00:00Z",
            "updatedAt": "2024-01-01T00:00:00Z",
            "labels": [],
            "assignees": [],
            "body": ""
        }
    ])
    .to_string();

    // When
    let issues = parse_gh_issue_list_output(&json).unwrap();
    // Then
    assert_eq!(issues.len(), 2);
    assert_eq!(issues[0].number, 305);
    assert_eq!(issues[0].title, "Add issue panel");
    assert_eq!(issues[0].labels.len(), 1);
    assert_eq!(issues[0].labels[0].name, "enhancement");
    assert_eq!(issues[0].assignees.len(), 1);
    assert_eq!(issues[1].number, 100);
    assert!(issues[1].labels.is_empty());
}

#[test]
fn parse_issue_list_empty_array() {
    // Given
    let json = "[]";
    // When
    let issues = parse_gh_issue_list_output(json).unwrap();
    // Then
    assert!(issues.is_empty());
}

#[test]
fn parse_issue_list_invalid_json() {
    // Given
    let json = "not json";
    // When
    let result = parse_gh_issue_list_output(json);
    // Then
    assert!(result.is_err());
}

#[test]
fn parse_issue_list_missing_optional_fields() {
    // Given
    let json = serde_json::json!([
        {
            "number": 1,
            "title": "Test",
            "state": "OPEN",
            "url": "https://github.com/owner/repo/issues/1",
            "author": {"login": "user"},
            "createdAt": "2024-01-01T00:00:00Z",
            "updatedAt": "2024-01-01T00:00:00Z"
        }
    ])
    .to_string();

    // When
    let issues = parse_gh_issue_list_output(&json).unwrap();
    // Then
    assert_eq!(issues.len(), 1);
    assert!(issues[0].labels.is_empty());
    assert!(issues[0].assignees.is_empty());
    assert!(issues[0].body.is_empty());
}

#[test]
fn parse_issue_list_real_gh_output() {
    // Given
    let json = serde_json::json!([
            {
                "assignees": [],
                "author": {"id": "MDQ6VXNlcjMwNjAxMTM2", "is_bot": false, "login": "user1", "name": "User One"},
                "body": "Issue body",
                "createdAt": "2026-02-17T18:05:54Z",
                "labels": [],
                "milestone": {"number": 11, "title": "Milestone", "description": "", "dueOn": null},
                "number": 313,
                "state": "OPEN",
                "title": "Persist pty sessions",
                "updatedAt": "2026-02-17T18:05:54Z",
                "url": "https://github.com/owner/repo/issues/313"
            },
            {
                "assignees": [],
                "author": {"id": "MDQ6VXNlcjMwNjAxMTM2", "is_bot": false, "login": "user1", "name": "User One"},
                "body": "",
                "createdAt": "2026-02-17T17:46:06Z",
                "labels": [{"id": "LA_kwDORH7BOc8AAAACW7Y17Q", "name": "enhancement", "description": "New feature or request", "color": "a2eeef"}],
                "milestone": null,
                "number": 312,
                "state": "OPEN",
                "title": "Add Notion task panel",
                "updatedAt": "2026-02-17T17:56:51Z",
                "url": "https://github.com/owner/repo/issues/312"
            }
        ])
        .to_string();

    // When
    let issues = parse_gh_issue_list_output(&json).unwrap();
    // Then
    assert_eq!(issues.len(), 2, "deserialization failed: got empty vec");
    assert_eq!(issues[0].number, 313);
    assert!(issues[0].milestone.is_some());
    assert_eq!(issues[0].milestone.as_ref().unwrap().title, "Milestone");
    assert_eq!(issues[1].number, 312);
    assert!(issues[1].milestone.is_none());
    assert_eq!(issues[1].labels.len(), 1);
}

#[test]
fn list_issue_parse_empty_log_message_omits_raw_payload() {
    // Given
    let stdout = serde_json::json!({
        "title": "Sensitive title",
        "url": "https://github.com/owner/repo/issues/1",
        "body": "Sensitive body"
    })
    .to_string();

    // When
    let message = parse_gh_issue_list_output(&stdout).unwrap_err().to_string();
    // Then
    assert!(message.contains(&format!("stdout_bytes={}", stdout.len())));
    assert!(!message.contains("Sensitive title"));
    assert!(!message.contains("https://github.com/owner/repo/issues/1"));
    assert!(!message.contains("Sensitive body"));
    assert!(!message.contains(&stdout));
}

mod cancellation_contract {
    use crate::domain::git_host::GitHostError;
    use crate::usecase::git_host::git_host_usecase::GitHostUsecase;
    use crate::usecase::git_host::test_helpers::{FakeIssueCache, FakePrCache};
    use std::sync::Arc;
    #[tokio::test]
    async fn test_github検出_停止時に既定pr状態を保存しない() {
        // Given
        use crate::common::operation_context::{OperationContext, OperationStopped};
        let pr_cache = Arc::new(FakePrCache::default());
        let issue_cache = Arc::new(FakeIssueCache::default());
        let uc = GitHostUsecase::new(
            Arc::new(crate::adaptor::gateway::git_host::github::GitHubGitHostGateway::default()),
            pr_cache.clone(),
            issue_cache.clone(),
        );
        let token = tokio_util::sync::CancellationToken::new();
        token.cancel();
        let context = OperationContext::new(None, Arc::new(token));
        // When
        let (pr, issues) = crate::common::operation_context::scope(context, async {
            (
                uc.refresh_pr_status("/missing").await,
                uc.fetch_issues("/missing").await,
            )
        })
        .await;
        // Then
        assert!(
            matches!(pr, Err(GitHostError::Technical(error)) if error == OperationStopped::Cancelled.into())
        );
        assert!(
            matches!(issues, Err(GitHostError::Technical(error)) if error == OperationStopped::Cancelled.into())
        );
        assert!(pr_cache.stored_values().is_empty());
        assert!(issue_cache.stored_values().is_empty());
    }

    #[tokio::test]
    async fn test_github検出_期限切れ時に既定pr状態を保存しない() {
        // Given
        use crate::common::operation_context::{Deadline, OperationContext, OperationStopped};
        let pr_cache = Arc::new(FakePrCache::default());
        let issue_cache = Arc::new(FakeIssueCache::default());
        let uc = GitHostUsecase::new(
            Arc::new(crate::adaptor::gateway::git_host::github::GitHubGitHostGateway::default()),
            pr_cache.clone(),
            issue_cache.clone(),
        );
        let token = tokio_util::sync::CancellationToken::new();
        token.cancel();
        let context = OperationContext::new(
            Some(Deadline::new(std::time::Instant::now())),
            Arc::new(token),
        );
        // When
        let (pr, issues) = crate::common::operation_context::scope(context, async {
            (
                uc.refresh_pr_status("/missing").await,
                uc.fetch_issues("/missing").await,
            )
        })
        .await;
        // Then
        assert!(
            matches!(pr, Err(GitHostError::Technical(error)) if error == OperationStopped::Expired.into())
        );
        assert!(
            matches!(issues, Err(GitHostError::Technical(error)) if error == OperationStopped::Expired.into())
        );
        assert!(pr_cache.stored_values().is_empty());
        assert!(issue_cache.stored_values().is_empty());
    }
}

#[tokio::test]
async fn test_pr取得_状態ごとに百件を取得し全件を順序どおり渡す() {
    struct Runner(parking_lot::Mutex<Vec<Vec<String>>>);
    #[async_trait::async_trait]
    impl GhCommandRunner for Runner {
        async fn output(&self, args: &[&str], repo: &str) -> GhCommandOutput {
            assert_eq!(repo, "/repo");
            self.0
                .lock()
                .push(args.iter().map(|arg| arg.to_string()).collect());
            GhCommandOutput::Success(if args[3] == "open" {
                "[]".into()
            } else {
                r#"[{"headRefName":"same","number":1,"url":"url","isDraft":false}]"#.into()
            })
        }
    }
    let runner = Arc::new(Runner(Default::default()));
    let gateway = GitHubGitHostGateway::with_runner(runner.clone());
    let status = gateway.fetch_github_prs("/repo").await.unwrap();
    assert_eq!(
        *runner.0.lock(),
        ["open", "merged", "closed"].map(|state| vec![
            "pr",
            "list",
            "--state",
            state,
            "--json",
            "headRefName,number,url,isDraft",
            "--limit",
            "100"
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>())
    );
    assert_eq!(
        status
            .completed_prs
            .iter()
            .map(|(_, pr)| pr.state)
            .collect::<Vec<_>>(),
        [PrState::Merged, PrState::Closed]
    );
}

#[test]
fn test_pr解析_同じbranchの同状態prを取得順のまま残す() {
    // Given / When
    let parsed = parse_gh_pr_list_output(r#"[{"headRefName":"same","number":1,"url":"first"},{"headRefName":"same","number":2,"url":"last"}]"#, PrState::Open).unwrap();
    // Then
    assert_eq!(
        parsed.iter().map(|(_, pr)| pr.number).collect::<Vec<_>>(),
        [1, 2]
    );
}

#[tokio::test]
async fn test_pr操作_headを明示し入力を別引数で渡す() {
    // Given
    struct Runner(parking_lot::Mutex<Vec<Vec<String>>>);
    #[async_trait::async_trait]
    impl GhCommandRunner for Runner {
        async fn output(&self, args: &[&str], _: &str) -> GhCommandOutput {
            self.0
                .lock()
                .push(args.iter().map(|arg| (*arg).into()).collect());
            GhCommandOutput::Success("".into())
        }
    }
    let runner = Arc::new(Runner(Default::default()));
    let gateway = GitHubGitHostGateway::with_runner(runner.clone());
    let new = crate::domain::git_host::NewPullRequest::new(
        "title".into(),
        "body; command".into(),
        "main".into(),
        true,
    )
    .unwrap();
    // When
    gateway
        .create_pull_request("/repo", "topic", &new)
        .await
        .unwrap();
    gateway
        .post_pull_request_comment(
            "/repo",
            42,
            &crate::domain::git_host::PrCommentText::new("comment".into()).unwrap(),
        )
        .await
        .unwrap();
    gateway
        .fetch_check_log("/repo", crate::domain::git_host::ActionsJobId(7))
        .await
        .unwrap();
    // Then
    let calls = runner.0.lock();
    assert_eq!(
        calls[0],
        [
            "pr",
            "create",
            "--head",
            "topic",
            "--base",
            "main",
            "--title",
            "title",
            "--body",
            "body; command",
            "--draft"
        ]
    );
    assert_eq!(calls[1], ["pr", "comment", "42", "--body", "comment"]);
    assert_eq!(calls[2], ["run", "view", "--job", "7", "--log"]);
}

#[tokio::test]
async fn test_pr取得_本文とcheckとcommentを読み取る() {
    // Given
    struct Runner;
    #[async_trait::async_trait]
    impl GhCommandRunner for Runner {
        async fn output(&self, args: &[&str], _: &str) -> GhCommandOutput {
            let value = if args[0..2] == ["pr", "view"] {
                r#"{"number":42,"url":"url","state":"OPEN","isDraft":true,"title":"title","body":"body"}"#
            } else if args[0..2] == ["pr", "checks"] {
                r#"[{"name":"test","bucket":"fail","startedAt":"2026-01-01T00:00:00Z","completedAt":"2026-01-01T00:04:00Z","link":"https://github.com/a/b/actions/runs/1/job/7"},{"name":"running","bucket":"pending","startedAt":"2026-01-01T00:00:00Z","completedAt":"0001-01-01T00:00:00Z","link":""},{"name":"external","bucket":"fail","startedAt":"0001-01-01T00:00:00Z","completedAt":"0001-01-01T00:00:00Z","link":"https://example.com/check"}]"#
            } else if args[1].ends_with("/reviews") {
                r#"[[{"user":{"login":"reviewer","type":"User"},"body":"review","submitted_at":"2026-01-01T00:02:00Z"},{"user":{"login":"empty","type":"User"},"body":"","submitted_at":null}]]"#
            } else {
                r#"[[{"user":{"login":"bot","type":"Bot"},"body":"comment","created_at":"2026-01-01T00:01:00Z"}]]"#
            };
            GhCommandOutput::Success(value.into())
        }
    }
    // When
    let pr = GitHubGitHostGateway::with_runner(Arc::new(Runner))
        .fetch_pull_request("/repo", 42)
        .await
        .unwrap();
    // Then
    assert_eq!(pr.title, "title");
    assert!(pr.info.draft);
    assert_eq!(pr.body, "body");
    assert_eq!(pr.checks.listed()[1].started_at, None);
    assert_eq!(pr.checks.listed()[1].completed_at, None);
    assert_eq!(pr.checks.listed()[1].failure_seconds(), None);
    assert!(pr.checks.listed()[2].running_since().is_some());
    assert_eq!(pr.checks.listed()[2].completed_at, None);
    assert_eq!(pr.checks.listed()[0].failure_seconds(), Some(240));
    assert_eq!(
        pr.checks.listed()[0].log_job(),
        Some(crate::domain::git_host::ActionsJobId(7))
    );
    assert_eq!(pr.conversation.entries().len(), 2);
    assert!(pr.conversation.entries()[0].author.is_bot);
}

#[tokio::test]
async fn test_pr取得_不正な応答を失敗として返す() {
    struct Runner(&'static str, &'static str, &'static str);
    #[async_trait::async_trait]
    impl GhCommandRunner for Runner {
        async fn output(&self, args: &[&str], _: &str) -> GhCommandOutput {
            GhCommandOutput::Success(
                if args[0..2] == ["pr", "view"] {
                    self.0
                } else if args[0..2] == ["pr", "checks"] {
                    self.1
                } else {
                    self.2
                }
                .into(),
            )
        }
    }
    let valid =
        r#"{"number":42,"url":"url","state":"OPEN","isDraft":false,"title":"title","body":""}"#;
    // Given
    for runner in [
        Runner("not json", "[]", "[]"),
        Runner(
            r#"{"number":42,"url":"url","state":"UNKNOWN","isDraft":false,"title":"title","body":""}"#,
            "[]",
            "[]",
        ),
        Runner(
            valid,
            r#"[{"name":"test","bucket":"unknown","link":""}]"#,
            "[]",
        ),
        Runner(
            valid,
            r#"[{"name":"test","bucket":"fail","link":"","startedAt":"invalid"}]"#,
            "[]",
        ),
        Runner(
            valid,
            r#"[{"name":"test","bucket":"fail","link":"","startedAt":"1969-12-31T23:59:59Z"}]"#,
            "[]",
        ),
        Runner(
            valid,
            "[]",
            r#"[[{"user":{"login":"user","type":"User"},"body":"comment"}]]"#,
        ),
    ] {
        // When
        let result = GitHubGitHostGateway::with_runner(Arc::new(runner))
            .fetch_pull_request("/repo", 42)
            .await;
        // Then
        assert!(matches!(result, Err(GitHostError::External(_))));
    }
}

#[tokio::test]
async fn test_pr操作_外部コマンドの失敗理由を返す() {
    // Given
    struct Runner;
    #[async_trait::async_trait]
    impl GhCommandRunner for Runner {
        async fn output(&self, _: &[&str], _: &str) -> GhCommandOutput {
            GhCommandOutput::NonZero {
                status: "1".into(),
                stderr: "permission denied".into(),
            }
        }
    }
    let gateway = GitHubGitHostGateway::with_runner(Arc::new(Runner));
    let new = crate::domain::git_host::NewPullRequest::new(
        "title".into(),
        "".into(),
        "main".into(),
        false,
    )
    .unwrap();
    let comment = crate::domain::git_host::PrCommentText::new("comment".into()).unwrap();
    // When / Then
    assert!(gateway
        .fetch_pull_request("/repo", 42)
        .await
        .unwrap_err()
        .to_string()
        .contains("permission denied"));
    assert!(gateway
        .create_pull_request("/repo", "topic", &new)
        .await
        .unwrap_err()
        .to_string()
        .contains("permission denied"));
    assert!(gateway
        .post_pull_request_comment("/repo", 42, &comment)
        .await
        .unwrap_err()
        .to_string()
        .contains("permission denied"));
    assert!(gateway
        .fetch_check_log("/repo", crate::domain::git_host::ActionsJobId(7))
        .await
        .unwrap_err()
        .to_string()
        .contains("permission denied"));
}
