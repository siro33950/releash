use std::collections::HashMap;
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::domain::git_host::{
    GitHostProvider, IssueInfo, IssueLabel, Milestone, PrAuthor, PrInfo, PrStatus, ProviderStatus,
};

use super::discovery::{get_origin_url, is_github, is_github_repository};

const GH_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) struct GitHubGitHostGateway {
    runner: Arc<dyn GhCommandRunner>,
}

impl GitHubGitHostGateway {
    #[cfg(test)]
    fn with_runner(runner: Arc<dyn GhCommandRunner>) -> Self {
        Self { runner }
    }
}

impl Default for GitHubGitHostGateway {
    fn default() -> Self {
        Self {
            runner: Arc::new(SystemGhCommandRunner),
        }
    }
}

trait GhCommandRunner: Send + Sync {
    fn status(&self, args: &[&str]) -> GhCommandStatus;
    fn output(&self, args: &[&str], repo_path: &str) -> GhCommandOutput;
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum GhCommandStatus {
    Success,
    Failed,
}

impl GhCommandStatus {
    fn is_success(&self) -> bool {
        matches!(self, Self::Success)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum GhCommandOutput {
    Success(String),
    SpawnFailed(String),
    NonZero { status: String, stderr: String },
    Timeout,
    TryWaitFailed(String),
    ReadFailed,
    InvalidUtf8,
}

struct SystemGhCommandRunner;

impl GhCommandRunner for SystemGhCommandRunner {
    fn status(&self, args: &[&str]) -> GhCommandStatus {
        let success = Command::new("gh")
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

        if success {
            GhCommandStatus::Success
        } else {
            GhCommandStatus::Failed
        }
    }

    fn output(&self, args: &[&str], repo_path: &str) -> GhCommandOutput {
        let mut child = match Command::new("gh")
            .args(args)
            .current_dir(repo_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => return GhCommandOutput::SpawnFailed(e.to_string()),
        };

        let Some(stdout) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return GhCommandOutput::ReadFailed;
        };
        let Some(stderr) = child.stderr.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return GhCommandOutput::ReadFailed;
        };
        let stdout_reader = spawn_pipe_reader(stdout);
        let stderr_reader = spawn_pipe_reader(stderr);

        let start = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let stdout_buf = join_pipe_reader(stdout_reader);
                    let stderr_buf = join_pipe_reader(stderr_reader).unwrap_or_default();
                    if !status.success() {
                        let stderr = String::from_utf8(stderr_buf).unwrap_or_default();
                        return GhCommandOutput::NonZero {
                            status: status.to_string(),
                            stderr,
                        };
                    }

                    let Some(stdout_buf) = stdout_buf else {
                        return GhCommandOutput::ReadFailed;
                    };
                    return String::from_utf8(stdout_buf)
                        .map(GhCommandOutput::Success)
                        .unwrap_or(GhCommandOutput::InvalidUtf8);
                }
                Ok(None) => {
                    if start.elapsed() > GH_TIMEOUT {
                        let _ = child.kill();
                        let _ = child.wait();
                        let _ = join_pipe_reader(stdout_reader);
                        let _ = join_pipe_reader(stderr_reader);
                        return GhCommandOutput::Timeout;
                    }
                    thread::sleep(Duration::from_millis(100));
                }
                Err(e) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = join_pipe_reader(stdout_reader);
                    let _ = join_pipe_reader(stderr_reader);
                    return GhCommandOutput::TryWaitFailed(e.to_string());
                }
            }
        }
    }
}

fn spawn_pipe_reader<R>(mut pipe: R) -> JoinHandle<Option<Vec<u8>>>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut buf = Vec::new();
        pipe.read_to_end(&mut buf).ok()?;
        Some(buf)
    })
}

fn join_pipe_reader(handle: JoinHandle<Option<Vec<u8>>>) -> Option<Vec<u8>> {
    handle.join().ok().flatten()
}

impl GitHostProvider for GitHubGitHostGateway {
    fn provider_status(&self, repo_path: &str) -> ProviderStatus {
        let remote_url = match get_origin_url(repo_path) {
            Some(url) => url,
            None => return ProviderStatus::NoRemote,
        };

        if is_github(&remote_url) {
            check_github_status(self.runner.as_ref())
        } else {
            ProviderStatus::UnsupportedPlatform
        }
    }

    fn fetch_pr_status(&self, repo_path: &str) -> PrStatus {
        if !is_github_repository(repo_path) {
            return PrStatus::default();
        }

        PrStatus {
            open_prs: detect_open_prs(self.runner.as_ref(), repo_path),
            merged_branches: detect_merged_prs(self.runner.as_ref(), repo_path),
        }
    }

    fn list_issues(&self, repo_path: &str) -> Vec<IssueInfo> {
        if !is_github_repository(repo_path) {
            return Vec::new();
        }

        let output = run_gh_with_timeout(
            self.runner.as_ref(),
            &[
                "issue",
                "list",
                "--state",
                "open",
                "--json",
                "number,title,state,url,author,createdAt,updatedAt,labels,assignees,body,milestone",
                "--limit",
                "100",
            ],
            repo_path,
        );
        match output {
            Some(stdout) => {
                let issues = parse_gh_issue_list_output(&stdout);
                if issues.is_empty() && stdout.trim() != "[]" && !stdout.trim().is_empty() {
                    eprintln!("{}", list_issues_parse_empty_log_message(&stdout));
                }
                issues
            }
            None => {
                eprintln!("[list_issues] gh command returned no output for {repo_path}");
                Vec::new()
            }
        }
    }
}

fn detect_open_prs(runner: &dyn GhCommandRunner, repo_path: &str) -> HashMap<String, PrInfo> {
    let output = run_gh_with_timeout(
        runner,
        &[
            "pr",
            "list",
            "--state",
            "open",
            "--json",
            "headRefName,number,url",
            "--limit",
            "100",
        ],
        repo_path,
    );
    match output {
        Some(stdout) => parse_gh_pr_list_output(&stdout),
        None => HashMap::new(),
    }
}

fn detect_merged_prs(runner: &dyn GhCommandRunner, repo_path: &str) -> Vec<String> {
    let output = run_gh_with_timeout(
        runner,
        &[
            "pr",
            "list",
            "--state",
            "merged",
            "--json",
            "headRefName",
            "--limit",
            "100",
        ],
        repo_path,
    );
    match output {
        Some(stdout) => parse_gh_merged_pr_output(&stdout),
        None => Vec::new(),
    }
}

fn check_github_status(runner: &dyn GhCommandRunner) -> ProviderStatus {
    if !runner.status(&["--version"]).is_success() {
        return ProviderStatus::CliNotFound {
            cli: "gh".to_string(),
        };
    }

    if !runner.status(&["auth", "status"]).is_success() {
        return ProviderStatus::NotAuthenticated;
    }

    ProviderStatus::Available
}

fn run_gh_with_timeout(
    runner: &dyn GhCommandRunner,
    args: &[&str],
    repo_path: &str,
) -> Option<String> {
    match runner.output(args, repo_path) {
        GhCommandOutput::Success(stdout) => Some(stdout),
        GhCommandOutput::SpawnFailed(error) => {
            eprintln!("[run_gh] spawn failed: {error}");
            None
        }
        GhCommandOutput::NonZero { status, stderr } => {
            eprintln!(
                "[run_gh] exit {status} for `gh {}` in {repo_path}: {stderr}",
                args.join(" ")
            );
            None
        }
        GhCommandOutput::Timeout => {
            eprintln!(
                "[run_gh] timeout for `gh {}` in {repo_path}",
                args.join(" ")
            );
            None
        }
        GhCommandOutput::TryWaitFailed(error) => {
            eprintln!("[run_gh] try_wait error: {error}");
            None
        }
        GhCommandOutput::ReadFailed | GhCommandOutput::InvalidUtf8 => None,
    }
}

fn list_issues_parse_empty_log_message(stdout: &str) -> String {
    format!(
        "[list_issues] parse returned 0 issues from non-empty output (stdout_bytes={})",
        stdout.len()
    )
}

fn parse_gh_pr_list_output(json_str: &str) -> HashMap<String, PrInfo> {
    let mut map = HashMap::new();
    let parsed: Result<Vec<serde_json::Value>, _> = serde_json::from_str(json_str);
    if let Ok(items) = parsed {
        for item in items {
            let head_ref = item.get("headRefName").and_then(|v| v.as_str());
            let number = item.get("number").and_then(|v| v.as_u64());
            let url = item.get("url").and_then(|v| v.as_str());
            if let (Some(branch), Some(num), Some(u)) = (head_ref, number, url) {
                map.insert(
                    branch.to_string(),
                    PrInfo {
                        number: num,
                        url: u.to_string(),
                    },
                );
            }
        }
    }
    map
}

fn parse_gh_merged_pr_output(json_str: &str) -> Vec<String> {
    let parsed: Result<Vec<serde_json::Value>, _> = serde_json::from_str(json_str);
    match parsed {
        Ok(items) => items
            .iter()
            .filter_map(|item| item.get("headRefName").and_then(|v| v.as_str()))
            .map(|s| s.to_string())
            .collect(),
        Err(_) => Vec::new(),
    }
}

fn parse_gh_issue_list_output(json_str: &str) -> Vec<IssueInfo> {
    serde_json::from_str::<Vec<GhIssueInfo>>(json_str)
        .map(|issues| issues.into_iter().map(Into::into).collect())
        .unwrap_or_default()
}

#[derive(Debug, Deserialize)]
struct GhPrAuthor {
    login: String,
}

impl From<GhPrAuthor> for PrAuthor {
    fn from(author: GhPrAuthor) -> Self {
        Self {
            login: author.login,
        }
    }
}

#[derive(Debug, Deserialize)]
struct GhMilestone {
    title: String,
}

impl From<GhMilestone> for Milestone {
    fn from(milestone: GhMilestone) -> Self {
        Self {
            title: milestone.title,
        }
    }
}

#[derive(Debug, Deserialize)]
struct GhIssueLabel {
    name: String,
    color: String,
}

impl From<GhIssueLabel> for IssueLabel {
    fn from(label: GhIssueLabel) -> Self {
        Self {
            name: label.name,
            color: label.color,
        }
    }
}

#[derive(Debug, Deserialize)]
struct GhIssueInfo {
    number: u64,
    title: String,
    state: String,
    url: String,
    author: GhPrAuthor,
    #[serde(alias = "createdAt")]
    created_at: String,
    #[serde(alias = "updatedAt")]
    updated_at: String,
    #[serde(default)]
    labels: Vec<GhIssueLabel>,
    #[serde(default)]
    assignees: Vec<GhPrAuthor>,
    #[serde(default)]
    body: String,
    #[serde(default)]
    milestone: Option<GhMilestone>,
}

impl From<GhIssueInfo> for IssueInfo {
    fn from(issue: GhIssueInfo) -> Self {
        Self {
            number: issue.number,
            title: issue.title,
            state: issue.state,
            url: issue.url,
            author: issue.author.into(),
            created_at: issue.created_at,
            updated_at: issue.updated_at,
            labels: issue.labels.into_iter().map(Into::into).collect(),
            assignees: issue.assignees.into_iter().map(Into::into).collect(),
            body: issue.body,
            milestone: issue.milestone.map(Into::into),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    #[derive(Default)]
    struct FakeGhRunner {
        statuses: Mutex<HashMap<Vec<String>, GhCommandStatus>>,
        outputs: Mutex<HashMap<Vec<String>, GhCommandOutput>>,
        status_calls: Mutex<Vec<Vec<String>>>,
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

        fn with_status(mut self, args: &[&str], status: GhCommandStatus) -> Self {
            self.statuses
                .get_mut()
                .unwrap()
                .insert(args_key(args), status);
            self
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

    impl GhCommandRunner for FakeGhRunner {
        fn status(&self, args: &[&str]) -> GhCommandStatus {
            let key = args_key(args);
            self.status_calls.lock().unwrap().push(key.clone());
            self.statuses
                .lock()
                .unwrap()
                .get(&key)
                .cloned()
                .unwrap_or(GhCommandStatus::Failed)
        }

        fn output(&self, args: &[&str], repo_path: &str) -> GhCommandOutput {
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

    #[test]
    fn provider_status_no_remote() {
        let dir = tempfile::TempDir::new().unwrap();
        git2::Repository::init(dir.path()).unwrap();

        let status = GitHubGitHostGateway::default().provider_status(dir.path().to_str().unwrap());

        assert!(matches!(status, ProviderStatus::NoRemote));
    }

    #[test]
    fn provider_status_unsupported_platform() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        repo.remote("origin", "https://gitlab.com/user/repo.git")
            .unwrap();

        let status = GitHubGitHostGateway::default().provider_status(dir.path().to_str().unwrap());

        assert!(matches!(status, ProviderStatus::UnsupportedPlatform));
    }

    #[test]
    fn provider_status_returns_cli_not_found_when_gh_version_fails() {
        let dir = github_repo();
        let runner =
            Arc::new(FakeGhRunner::new().with_status(&["--version"], GhCommandStatus::Failed));

        let status =
            GitHubGitHostGateway::with_runner(runner).provider_status(dir.path().to_str().unwrap());

        assert!(matches!(
            status,
            ProviderStatus::CliNotFound { cli } if cli == "gh"
        ));
    }

    #[test]
    fn provider_status_returns_not_authenticated_when_auth_status_fails() {
        let dir = github_repo();
        let runner = Arc::new(
            FakeGhRunner::new()
                .with_status(&["--version"], GhCommandStatus::Success)
                .with_status(&["auth", "status"], GhCommandStatus::Failed),
        );

        let status =
            GitHubGitHostGateway::with_runner(runner).provider_status(dir.path().to_str().unwrap());

        assert!(matches!(status, ProviderStatus::NotAuthenticated));
    }

    #[test]
    fn provider_status_returns_available_when_cli_and_auth_succeed() {
        let dir = github_repo();
        let runner = Arc::new(
            FakeGhRunner::new()
                .with_status(&["--version"], GhCommandStatus::Success)
                .with_status(&["auth", "status"], GhCommandStatus::Success),
        );

        let status =
            GitHubGitHostGateway::with_runner(runner).provider_status(dir.path().to_str().unwrap());

        assert!(matches!(status, ProviderStatus::Available));
    }

    #[test]
    fn fetch_pr_status_returns_empty_for_non_github_repo() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        repo.remote("origin", "https://gitlab.com/user/repo.git")
            .unwrap();

        let status = GitHubGitHostGateway::default().fetch_pr_status(dir.path().to_str().unwrap());

        assert_eq!(status, PrStatus::default());
    }

    #[test]
    fn list_issues_returns_empty_for_non_github_repo() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        repo.remote("origin", "https://gitlab.com/user/repo.git")
            .unwrap();

        let issues = GitHubGitHostGateway::default().list_issues(dir.path().to_str().unwrap());

        assert!(issues.is_empty());
    }

    #[test]
    fn fetch_pr_status_combines_open_and_merged_runner_outputs() {
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
            .fetch_pr_status(dir.path().to_str().unwrap());

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

    #[test]
    fn list_issues_returns_runner_output_as_issue_info() {
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
            .list_issues(dir.path().to_str().unwrap());

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

    #[test]
    fn gh_output_failures_collapse_to_empty_results() {
        let failures = [
            GhCommandOutput::SpawnFailed("gh is missing".to_string()),
            GhCommandOutput::NonZero {
                status: "exit status: 1".to_string(),
                stderr: "x".repeat(70 * 1024),
            },
            GhCommandOutput::Timeout,
        ];

        for failure in failures {
            let dir = github_repo();
            let runner = Arc::new(
                FakeGhRunner::new()
                    .with_output(&open_pr_list_args(), failure.clone())
                    .with_output(&merged_pr_list_args(), failure.clone())
                    .with_output(&issue_list_args(), failure),
            );
            let gateway = GitHubGitHostGateway::with_runner(runner);

            assert_eq!(
                gateway.fetch_pr_status(dir.path().to_str().unwrap()),
                PrStatus::default()
            );
            assert!(gateway.list_issues(dir.path().to_str().unwrap()).is_empty());
        }
    }

    #[test]
    fn parse_open_prs_valid_json() {
        let json = r#"[
            {"headRefName":"feat/login","number":42,"url":"https://github.com/owner/repo/pull/42"},
            {"headRefName":"fix/typo","number":7,"url":"https://github.com/owner/repo/pull/7"}
        ]"#;

        let map = parse_gh_pr_list_output(json);

        assert_eq!(map.len(), 2);
        let pr = map.get("feat/login").unwrap();
        assert_eq!(pr.number, 42);
        assert_eq!(pr.url, "https://github.com/owner/repo/pull/42");
    }

    #[test]
    fn parse_open_prs_empty_array() {
        let map = parse_gh_pr_list_output("[]");

        assert!(map.is_empty());
    }

    #[test]
    fn parse_open_prs_invalid_json() {
        let map = parse_gh_pr_list_output("not json");

        assert!(map.is_empty());
    }

    #[test]
    fn parse_open_prs_missing_fields() {
        let json = r#"[{"headRefName":"feat/x"}]"#;

        let map = parse_gh_pr_list_output(json);

        assert!(map.is_empty());
    }

    #[test]
    fn parse_merged_prs_valid() {
        let json = r#"[{"headRefName":"feat/a"},{"headRefName":"feat/b"}]"#;

        let branches = parse_gh_merged_pr_output(json);

        assert_eq!(branches, vec!["feat/a", "feat/b"]);
    }

    #[test]
    fn parse_merged_prs_empty() {
        let branches = parse_gh_merged_pr_output("[]");

        assert!(branches.is_empty());
    }

    #[test]
    fn parse_merged_prs_invalid() {
        let branches = parse_gh_merged_pr_output("invalid");

        assert!(branches.is_empty());
    }

    #[test]
    fn parse_issue_list_valid_json() {
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

        let issues = parse_gh_issue_list_output(&json);

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
        let issues = parse_gh_issue_list_output("[]");

        assert!(issues.is_empty());
    }

    #[test]
    fn parse_issue_list_invalid_json() {
        let issues = parse_gh_issue_list_output("not json");

        assert!(issues.is_empty());
    }

    #[test]
    fn parse_issue_list_missing_optional_fields() {
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

        let issues = parse_gh_issue_list_output(&json);

        assert_eq!(issues.len(), 1);
        assert!(issues[0].labels.is_empty());
        assert!(issues[0].assignees.is_empty());
        assert!(issues[0].body.is_empty());
    }

    #[test]
    fn parse_issue_list_real_gh_output() {
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

        let issues = parse_gh_issue_list_output(&json);

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
        let stdout = serde_json::json!({
            "title": "Sensitive title",
            "url": "https://github.com/owner/repo/issues/1",
            "body": "Sensitive body"
        })
        .to_string();

        let message = list_issues_parse_empty_log_message(&stdout);

        assert!(message.contains(&format!("stdout_bytes={}", stdout.len())));
        assert!(!message.contains("Sensitive title"));
        assert!(!message.contains("https://github.com/owner/repo/issues/1"));
        assert!(!message.contains("Sensitive body"));
        assert!(!message.contains(&stdout));
    }
}
