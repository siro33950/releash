use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;

use crate::domain::git_host::{
    GitHostError, GitHostProvider, IssueInfo, IssueLabel, Milestone, PrAuthor, PrInfo, PrState,
    PrStatus,
};

use super::discovery::is_github_repository;

pub const GH_TIMEOUT: Duration = Duration::from_secs(10);

pub struct GitHubGitHostGateway {
    runner: Arc<dyn GhCommandRunner>,
}

impl GitHubGitHostGateway {
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_runner(runner: Arc<dyn GhCommandRunner>) -> Self {
        Self { runner }
    }
}

impl Default for GitHubGitHostGateway {
    fn default() -> Self {
        Self {
            runner: Arc::new(SystemGhCommandRunner::default()),
        }
    }
}

#[async_trait::async_trait]
pub trait GhCommandRunner: Send + Sync {
    async fn output(&self, args: &[&str], repo_path: &str) -> GhCommandOutput;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GhCommandOutput {
    Success(String),
    SpawnFailed(String),
    NonZero { status: String, stderr: String },
    Timeout,
    Stopped(crate::common::operation_context::OperationStopped),
    InvalidUtf8,
}

#[derive(Default)]
pub struct SystemGhCommandRunner {
    #[cfg(any(test, feature = "test-support"))]
    program: Option<std::path::PathBuf>,
}

#[async_trait::async_trait]
impl GhCommandRunner for SystemGhCommandRunner {
    async fn output(&self, args: &[&str], repo_path: &str) -> GhCommandOutput {
        let program = std::path::Path::new("gh");
        #[cfg(any(test, feature = "test-support"))]
        let program = self.program.as_deref().unwrap_or(program);
        let mut command = tokio::process::Command::new(program);
        command.args(args).current_dir(repo_path);
        match crate::common::operation_context::timeout(GH_TIMEOUT, async {
            crate::infrastructure::process::output::output(command, Vec::new()).await
        })
        .await
        {
            Ok(Ok(output)) if output.status.success() => String::from_utf8(output.stdout)
                .map(GhCommandOutput::Success)
                .unwrap_or(GhCommandOutput::InvalidUtf8),
            Ok(Ok(output)) => GhCommandOutput::NonZero {
                status: output.status.to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            },
            Ok(Err(crate::infrastructure::process::output::ProcessError::Stopped(
                crate::common::operation_context::OperationStopped::Expired,
            )))
            | Err(crate::common::operation_context::OperationStopped::Expired) => {
                GhCommandOutput::Timeout
            }
            Ok(Err(crate::infrastructure::process::output::ProcessError::Stopped(error)))
            | Err(error) => GhCommandOutput::Stopped(error),
            Ok(Err(crate::infrastructure::process::output::ProcessError::Io(error))) => {
                GhCommandOutput::SpawnFailed(error.to_string())
            }
        }
    }
}

#[async_trait::async_trait]

impl GitHostProvider for GitHubGitHostGateway {
    async fn fetch_pr_status(&self, repo_path: &str) -> Result<PrStatus, GitHostError> {
        if !is_github_repository(repo_path).map_err(GitHostError::from)? {
            return Ok(PrStatus::default());
        }

        self.fetch_github_prs(repo_path).await
    }

    async fn list_issues(&self, repo_path: &str) -> Result<Vec<IssueInfo>, GitHostError> {
        if !is_github_repository(repo_path).map_err(GitHostError::from)? {
            return Ok(Vec::new());
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
        )
        .await;
        parse_gh_issue_list_output(&output?)
    }
}

impl GitHubGitHostGateway {
    async fn fetch_github_prs(&self, repo_path: &str) -> Result<PrStatus, GitHostError> {
        let open_prs = detect_prs(self.runner.as_ref(), repo_path, "open", PrState::Open).await?;
        let merged = detect_prs(self.runner.as_ref(), repo_path, "merged", PrState::Merged).await?;
        let mut completed_prs =
            detect_prs(self.runner.as_ref(), repo_path, "closed", PrState::Closed).await?;
        completed_prs.extend(merged);
        Ok(PrStatus {
            open_prs,
            completed_prs,
        })
    }
}

async fn detect_prs(
    runner: &dyn GhCommandRunner,
    repo_path: &str,
    state: &str,
    pr_state: PrState,
) -> Result<HashMap<String, PrInfo>, GitHostError> {
    let output = run_gh_with_timeout(
        runner,
        &[
            "pr",
            "list",
            "--state",
            state,
            "--json",
            "headRefName,number,url,isDraft",
            "--limit",
            "100",
        ],
        repo_path,
    )
    .await?;
    parse_gh_pr_list_output(&output, pr_state)
}

async fn run_gh_with_timeout(
    runner: &dyn GhCommandRunner,
    args: &[&str],
    repo_path: &str,
) -> Result<String, GitHostError> {
    let command = args.join(" ");
    let result = match crate::common::operation_context::wait(
        &crate::common::operation_context::current(),
        runner.output(args, repo_path),
    )
    .await
    .map_err(GitHostError::from)?
    {
        GhCommandOutput::Success(stdout) => return Ok(stdout),
        GhCommandOutput::Stopped(error) => return Err(GitHostError::from(error)),
        GhCommandOutput::Timeout => {
            return Err(GitHostError::Technical(
                crate::common::operation_context::OperationStopped::Expired.into(),
            ))
        }
        GhCommandOutput::SpawnFailed(error) => format!("gh spawn failed: {error}"),
        GhCommandOutput::NonZero { status, stderr } => {
            format!("gh exit {status} for `gh {command}` in {repo_path}: {stderr}")
        }
        GhCommandOutput::InvalidUtf8 => format!("gh output is not UTF-8 for `gh {command}`"),
    };
    Err(GitHostError::External(result))
}

fn parse_gh_pr_items(json_str: &str) -> Result<Vec<serde_json::Value>, GitHostError> {
    serde_json::from_str(json_str)
        .map_err(|error| GitHostError::External(format!("gh pr list output is invalid: {error}")))
}

fn parse_gh_pr_list_output(
    json_str: &str,
    state: PrState,
) -> Result<HashMap<String, PrInfo>, GitHostError> {
    let mut map = HashMap::new();
    for item in parse_gh_pr_items(json_str)? {
        let head_ref = item.get("headRefName").and_then(|v| v.as_str());
        let number = item.get("number").and_then(|v| v.as_u64());
        let url = item.get("url").and_then(|v| v.as_str());
        if let (Some(branch), Some(num), Some(u)) = (head_ref, number, url) {
            map.insert(
                branch.to_string(),
                PrInfo {
                    number: num,
                    url: u.to_string(),
                    state,
                    draft: item
                        .get("isDraft")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false),
                },
            );
        }
    }
    Ok(map)
}

fn parse_gh_issue_list_output(json_str: &str) -> Result<Vec<IssueInfo>, GitHostError> {
    serde_json::from_str::<Vec<GhIssueInfo>>(json_str)
        .map(|issues| issues.into_iter().map(Into::into).collect())
        .map_err(|error| {
            GitHostError::External(format!(
                "gh issue list output is invalid (stdout_bytes={}): {error}",
                json_str.len()
            ))
        })
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
#[path = "github_test.rs"]
mod github_tests;

#[cfg(feature = "test-support")]
impl SystemGhCommandRunner {
    pub fn test_new(program: Option<std::path::PathBuf>) -> Self {
        Self { program }
    }
}
