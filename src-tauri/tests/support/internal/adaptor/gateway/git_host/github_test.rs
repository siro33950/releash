use super::*;
use crate::common::operation_context::scope;
use crate::common::operation_context::{Deadline, OperationContext, OperationStopped};
use std::time::Instant;

#[cfg(unix)]
#[tokio::test]
pub async fn test_gh実runner_呼出期限と資源期限の早い方で停止する() {
    // Given
    let runner = SystemGhCommandRunner::test_new(Some("/bin/sh".into()));
    for deadline in [Some(Duration::from_millis(50)), Some(GH_TIMEOUT * 3), None] {
        let start = Instant::now();
        let context = OperationContext::default();
        let context = deadline.map_or(context.clone(), |duration| {
            context.with_deadline(Deadline::new(start + duration))
        });
        // When
        let output = scope(context, async {
            runner.output(&["-c", "exec sleep 30"], "/").await
        })
        .await;
        // Then
        assert_eq!(output, GhCommandOutput::Timeout);
        let expected = deadline.unwrap_or(GH_TIMEOUT).min(GH_TIMEOUT);
        assert!(start.elapsed() >= expected);
        assert!(start.elapsed() < expected + Duration::from_secs(3));
    }
}

#[cfg(unix)]
#[tokio::test]
pub async fn test_gh実runner_実行中の取消を保持する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let started = directory.path().join("started");
    let token = tokio_util::sync::CancellationToken::new();
    let context = OperationContext::new(None, Arc::new(token.clone()));
    let runner = SystemGhCommandRunner::test_new(Some("/bin/sh".into()));
    let task = tokio::spawn(async move {
        scope(context, async {
            runner
                .output(
                    &["-c", "touch started; exec sleep 30"],
                    directory.path().to_str().unwrap(),
                )
                .await
        })
        .await
    });
    let start = Instant::now();
    while !started.exists() {
        assert!(start.elapsed() < Duration::from_secs(3));
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    // When
    token.cancel();
    // Then
    assert_eq!(
        task.await.unwrap(),
        GhCommandOutput::Stopped(OperationStopped::Cancelled)
    );
}

#[cfg(unix)]
#[tokio::test]
pub async fn test_gh実runner_出力と起動失敗の変換を保つ() {
    // Given
    let runner = SystemGhCommandRunner::test_new(Some("/bin/sh".into()));
    // When / Then
    assert_eq!(
        runner.output(&["-c", "printf ok"], "/").await,
        GhCommandOutput::Success("ok".into())
    );
    assert!(
        matches!(runner.output(&["-c", "printf error >&2; exit 7"], "/").await, GhCommandOutput::NonZero { stderr, .. } if stderr == "error")
    );
    assert_eq!(
        runner.output(&["-c", "printf '\\377'"], "/").await,
        GhCommandOutput::InvalidUtf8
    );
    let missing = tempfile::tempdir().unwrap();
    let runner = SystemGhCommandRunner::test_new(Some(missing.path().join("missing-gh")));
    assert!(matches!(
        runner.output(&[], "/").await,
        GhCommandOutput::SpawnFailed(_)
    ));
}
pub(crate) mod tests {
    use releash_lib::test_support::integration::domain::git_host::GitHostProvider;
    use std::sync::{Arc, Mutex};

    use super::super::*;

    #[derive(Default)]
    struct FakeGhRunner {
        outputs: Mutex<HashMap<Vec<String>, GhCommandOutput>>,
        output_calls: Mutex<Vec<FakeOutputCall>>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct FakeOutputCall {
        args: Vec<String>,
        repo_path: String,
    }

    impl FakeGhRunner {
        fn new() -> Self {
            Self::default()
        }

        fn with_output(mut self, args: &[&str], output: GhCommandOutput) -> Self {
            self.outputs
                .get_mut()
                .unwrap()
                .insert(args_key(args), output);
            self
        }

        fn output_calls(&self) -> Vec<FakeOutputCall> {
            self.output_calls.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl GhCommandRunner for FakeGhRunner {
        async fn output(&self, args: &[&str], repo_path: &str) -> GhCommandOutput {
            let key = args_key(args);
            self.output_calls.lock().unwrap().push(FakeOutputCall {
                args: key.clone(),
                repo_path: repo_path.to_string(),
            });
            self.outputs
                .lock()
                .unwrap()
                .get(&key)
                .cloned()
                .unwrap_or_else(|| GhCommandOutput::SpawnFailed("unexpected gh call".to_string()))
        }
    }

    fn args_key(args: &[&str]) -> Vec<String> {
        args.iter().map(|arg| (*arg).to_string()).collect()
    }

    fn github_repo() -> tempfile::TempDir {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        repo.remote("origin", "https://github.com/user/repo.git")
            .unwrap();
        dir
    }

    fn open_pr_list_args() -> Vec<&'static str> {
        vec![
            "pr",
            "list",
            "--state",
            "open",
            "--json",
            "headRefName,number,url",
            "--limit",
            "100",
        ]
    }

    fn merged_pr_list_args() -> Vec<&'static str> {
        vec![
            "pr",
            "list",
            "--state",
            "merged",
            "--json",
            "headRefName",
            "--limit",
            "100",
        ]
    }

    fn issue_list_args() -> Vec<&'static str> {
        vec![
            "issue",
            "list",
            "--state",
            "open",
            "--json",
            "number,title,state,url,author,createdAt,updatedAt,labels,assignees,body,milestone",
            "--limit",
            "100",
        ]
    }

    #[tokio::test]
    pub async fn fetch_pr_status_returns_empty_for_non_github_repo() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        repo.remote("origin", "https://gitlab.com/user/repo.git")
            .unwrap();

        let status = GitHubGitHostGateway::default()
            .fetch_pr_status(dir.path().to_str().unwrap())
            .await;

        assert_eq!(status, Ok(PrStatus::default()));
    }

    #[tokio::test]
    pub async fn list_issues_returns_empty_for_non_github_repo() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        repo.remote("origin", "https://gitlab.com/user/repo.git")
            .unwrap();

        let issues = GitHubGitHostGateway::default()
            .list_issues(dir.path().to_str().unwrap())
            .await
            .unwrap();

        assert!(issues.is_empty());
    }

    #[tokio::test]
    pub async fn fetch_pr_status_combines_open_and_merged_runner_outputs() {
        let dir = github_repo();
        let runner = Arc::new(
            FakeGhRunner::new()
                .with_output(
                    &open_pr_list_args(),
                    GhCommandOutput::Success(
                        r#"[
                            {"headRefName":"feat/login","number":42,"url":"https://github.com/owner/repo/pull/42"}
                        ]"#
                        .to_string(),
                    ),
                )
                .with_output(
                    &merged_pr_list_args(),
                    GhCommandOutput::Success(r#"[{"headRefName":"feat/done"}]"#.to_string()),
                ),
        );

        let status = GitHubGitHostGateway::with_runner(runner.clone())
            .fetch_pr_status(dir.path().to_str().unwrap())
            .await
            .unwrap();

        assert_eq!(status.open_prs.len(), 1);
        assert_eq!(status.open_prs["feat/login"].number, 42);
        assert_eq!(
            status.open_prs["feat/login"].url,
            "https://github.com/owner/repo/pull/42"
        );
        assert_eq!(status.merged_branches, vec!["feat/done"]);
        assert_eq!(
            runner.output_calls(),
            vec![
                FakeOutputCall {
                    args: args_key(&open_pr_list_args()),
                    repo_path: dir.path().to_string_lossy().to_string(),
                },
                FakeOutputCall {
                    args: args_key(&merged_pr_list_args()),
                    repo_path: dir.path().to_string_lossy().to_string(),
                },
            ]
        );
    }

    #[tokio::test]
    pub async fn list_issues_returns_runner_output_as_issue_info() {
        let dir = github_repo();
        let runner = Arc::new(
            FakeGhRunner::new().with_output(
                &issue_list_args(),
                GhCommandOutput::Success(
                    serde_json::json!([
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
                            "body": "Issue body",
                            "milestone": null
                        }
                    ])
                    .to_string(),
                ),
            ),
        );

        let issues = GitHubGitHostGateway::with_runner(runner.clone())
            .list_issues(dir.path().to_str().unwrap())
            .await
            .unwrap();

        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].number, 305);
        assert_eq!(issues[0].title, "Add issue panel");
        assert_eq!(issues[0].url, "https://github.com/owner/repo/issues/305");
        assert_eq!(
            runner.output_calls(),
            vec![FakeOutputCall {
                args: args_key(&issue_list_args()),
                repo_path: dir.path().to_string_lossy().to_string(),
            }]
        );
    }

    #[tokio::test]
    pub async fn test_gh出力_起動失敗をprとissueの空に変換しない() {
        // Given
        let failure = GhCommandOutput::SpawnFailed("gh is missing".to_string());
        let dir = github_repo();
        let runner = Arc::new(
            FakeGhRunner::new()
                .with_output(&open_pr_list_args(), failure.clone())
                .with_output(&merged_pr_list_args(), failure.clone())
                .with_output(&issue_list_args(), failure),
        );
        let gateway = GitHubGitHostGateway::with_runner(runner);
        // When
        let pr = gateway.fetch_pr_status(dir.path().to_str().unwrap()).await;
        let issues = gateway.list_issues(dir.path().to_str().unwrap()).await;
        // Then
        assert!(pr.is_err());
        assert!(issues.is_err());
    }

    #[tokio::test]
    pub async fn test_gh出力_異常終了をprとissueの空に変換しない() {
        // Given
        let failure = GhCommandOutput::NonZero {
            status: "exit status: 1".to_string(),
            stderr: "x".repeat(70 * 1024),
        };
        let dir = github_repo();
        let runner = Arc::new(
            FakeGhRunner::new()
                .with_output(&open_pr_list_args(), failure.clone())
                .with_output(&merged_pr_list_args(), failure.clone())
                .with_output(&issue_list_args(), failure),
        );
        let gateway = GitHubGitHostGateway::with_runner(runner);
        // When
        let pr = gateway.fetch_pr_status(dir.path().to_str().unwrap()).await;
        let issues = gateway.list_issues(dir.path().to_str().unwrap()).await;
        // Then
        assert!(pr.is_err());
        assert!(issues.is_err());
    }

    #[tokio::test]
    pub async fn test_gh出力_タイムアウトをprとissueの空に変換しない() {
        // Given
        let failure = GhCommandOutput::Timeout;
        let dir = github_repo();
        let runner = Arc::new(
            FakeGhRunner::new()
                .with_output(&open_pr_list_args(), failure.clone())
                .with_output(&merged_pr_list_args(), failure.clone())
                .with_output(&issue_list_args(), failure),
        );
        let gateway = GitHubGitHostGateway::with_runner(runner);
        // When
        let pr = gateway.fetch_pr_status(dir.path().to_str().unwrap()).await;
        let issues = gateway.list_issues(dir.path().to_str().unwrap()).await;
        // Then
        assert!(pr.is_err());
        assert!(matches!(
            issues,
            Err(GitHostError::Technical(
                crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                    ..
                }
            ))
        ));
    }

    #[tokio::test]
    pub async fn test_gh出力_json破損をprとissueの空に変換しない() {
        // Given
        let failure = GhCommandOutput::Success("not json".to_string());
        let dir = github_repo();
        let runner = Arc::new(
            FakeGhRunner::new()
                .with_output(&open_pr_list_args(), failure.clone())
                .with_output(&merged_pr_list_args(), failure.clone())
                .with_output(&issue_list_args(), failure),
        );
        let gateway = GitHubGitHostGateway::with_runner(runner);
        // When
        let pr = gateway.fetch_pr_status(dir.path().to_str().unwrap()).await;
        let issues = gateway.list_issues(dir.path().to_str().unwrap()).await;
        // Then
        assert!(pr.is_err());
        assert!(issues.is_err());
    }
}
