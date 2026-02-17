use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::types::{GitHostProvider, PrDetail, PrInfo};

const GH_TIMEOUT: Duration = Duration::from_secs(10);

pub struct GitHubProvider;

impl GitHostProvider for GitHubProvider {
    fn detect_open_prs(&self, repo_path: &str) -> HashMap<String, PrInfo> {
        let output = run_gh_with_timeout(
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

    fn detect_merged_prs(&self, repo_path: &str) -> Vec<String> {
        let output = run_gh_with_timeout(
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

    fn get_pr_detail(&self, repo_path: &str, pr_number: u64) -> Option<PrDetail> {
        let number_str = pr_number.to_string();
        let output = run_gh_with_timeout(
            &[
                "pr",
                "view",
                &number_str,
                "--json",
                "number,title,body,state,url,author,createdAt,headRefName,baseRefName,additions,deletions,changedFiles,comments,reviews",
            ],
            repo_path,
        )?;
        parse_gh_pr_detail(&output)
    }
}

fn run_gh_with_timeout(args: &[&str], repo_path: &str) -> Option<String> {
    let mut child = Command::new("gh")
        .args(args)
        .current_dir(repo_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return None;
                }
                break;
            }
            Ok(None) => {
                if start.elapsed() > GH_TIMEOUT {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(_) => return None,
        }
    }

    let output = child.wait_with_output().ok()?;
    String::from_utf8(output.stdout).ok()
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

fn parse_gh_pr_detail(json_str: &str) -> Option<PrDetail> {
    serde_json::from_str(json_str).ok()
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn parse_pr_detail_valid_json() {
        let json = serde_json::json!({
            "number": 42,
            "title": "Add feature",
            "body": "## Description\nSome changes",
            "state": "OPEN",
            "url": "https://github.com/owner/repo/pull/42",
            "author": {"login": "user1"},
            "createdAt": "2024-01-01T00:00:00Z",
            "headRefName": "feat/login",
            "baseRefName": "main",
            "additions": 10,
            "deletions": 3,
            "changedFiles": 2,
            "comments": [
                {"author": {"login": "reviewer1"}, "body": "LGTM", "createdAt": "2024-01-02T00:00:00Z"}
            ],
            "reviews": [
                {"author": {"login": "reviewer1"}, "body": "Approved!", "state": "APPROVED", "submittedAt": "2024-01-02T00:00:00Z"}
            ]
        })
        .to_string();
        let detail = parse_gh_pr_detail(&json).unwrap();
        assert_eq!(detail.number, 42);
        assert_eq!(detail.title, "Add feature");
        assert_eq!(detail.state, "OPEN");
        assert_eq!(detail.head_ref_name, "feat/login");
        assert_eq!(detail.base_ref_name, "main");
        assert_eq!(detail.additions, 10);
        assert_eq!(detail.deletions, 3);
        assert_eq!(detail.changed_files, 2);
        assert_eq!(detail.comments.len(), 1);
        assert_eq!(detail.comments[0].author.login, "reviewer1");
        assert_eq!(detail.reviews.len(), 1);
        assert_eq!(detail.reviews[0].state, "APPROVED");
    }

    #[test]
    fn parse_pr_detail_empty_comments_and_reviews() {
        let json = serde_json::json!({
            "number": 1,
            "title": "Fix bug",
            "body": "",
            "state": "MERGED",
            "url": "https://github.com/owner/repo/pull/1",
            "author": {"login": "dev"},
            "createdAt": "2024-01-01T00:00:00Z",
            "headRefName": "fix/bug",
            "baseRefName": "main",
            "additions": 0,
            "deletions": 0,
            "changedFiles": 0,
            "comments": [],
            "reviews": []
        })
        .to_string();
        let detail = parse_gh_pr_detail(&json).unwrap();
        assert_eq!(detail.number, 1);
        assert!(detail.comments.is_empty());
        assert!(detail.reviews.is_empty());
    }

    #[test]
    fn parse_pr_detail_invalid_json() {
        assert!(parse_gh_pr_detail("not json").is_none());
    }

    #[test]
    fn parse_pr_detail_missing_fields() {
        let json = serde_json::json!({"number": 1, "title": "Partial"}).to_string();
        assert!(parse_gh_pr_detail(&json).is_none());
    }
}
