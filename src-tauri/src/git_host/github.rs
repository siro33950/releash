use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::types::{GitHostProvider, PrInfo};

const GH_TIMEOUT: Duration = Duration::from_secs(10);

pub struct GitHubProvider;

impl GitHostProvider for GitHubProvider {
    fn detect_open_prs(&self, repo_path: &str) -> HashMap<String, PrInfo> {
        let output = run_gh_with_timeout(
            &[
                "pr", "list", "--state", "open", "--json", "headRefName,number,url", "--limit",
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
                "pr", "list", "--state", "merged", "--json", "headRefName", "--limit", "100",
            ],
            repo_path,
        );
        match output {
            Some(stdout) => parse_gh_merged_pr_output(&stdout),
            None => Vec::new(),
        }
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
}
