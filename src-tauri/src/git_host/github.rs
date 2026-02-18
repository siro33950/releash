use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::types::{GitHostProvider, IssueInfo, PrDetail, PrInfo};

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

    fn list_issues(&self, repo_path: &str) -> Vec<IssueInfo> {
        let output = run_gh_with_timeout(
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
                    eprintln!(
                        "[list_issues] parse returned 0 issues from non-empty output: {stdout}"
                    );
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

fn run_gh_with_timeout(args: &[&str], repo_path: &str) -> Option<String> {
    let mut child = match Command::new("gh")
        .args(args)
        .current_dir(repo_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[run_gh] spawn failed: {e}");
            return None;
        }
    };

    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        use std::io::Read;
        let mut buf = Vec::new();
        stdout.read_to_end(&mut buf).ok()?;
        Some(buf)
    });

    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    let stderr = child
                        .stderr
                        .take()
                        .and_then(|mut s| {
                            let mut buf = String::new();
                            std::io::Read::read_to_string(&mut s, &mut buf).ok()?;
                            Some(buf)
                        })
                        .unwrap_or_default();
                    eprintln!(
                        "[run_gh] exit {status} for `gh {}` in {repo_path}: {stderr}",
                        args.join(" ")
                    );
                    return None;
                }
                break;
            }
            Ok(None) => {
                if start.elapsed() > GH_TIMEOUT {
                    let _ = child.kill();
                    let _ = child.wait();
                    eprintln!(
                        "[run_gh] timeout for `gh {}` in {repo_path}",
                        args.join(" ")
                    );
                    return None;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => {
                eprintln!("[run_gh] try_wait error: {e}");
                return None;
            }
        }
    }

    let buf = reader.join().ok()??;
    String::from_utf8(buf).ok()
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

fn parse_gh_issue_list_output(json_str: &str) -> Vec<IssueInfo> {
    serde_json::from_str(json_str).unwrap_or_default()
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

    #[test]
    fn parse_issue_list_valid_json() {
        let json = serde_json::json!([
            {
                "number": 305,
                "title": "Kanban画面にIssue管理パネルを追加",
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
        assert_eq!(issues[0].title, "Kanban画面にIssue管理パネルを追加");
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
        // gh issue list --json の実際の出力形式（余分なフィールド含む）
        let json = serde_json::json!([
            {
                "assignees": [],
                "author": {"id": "MDQ6VXNlcjMwNjAxMTM2", "is_bot": false, "login": "siro33950", "name": "siro33950"},
                "body": "Issue body",
                "createdAt": "2026-02-17T18:05:54Z",
                "labels": [],
                "milestone": {"number": 11, "title": "マルチPTY管理", "description": "", "dueOn": null},
                "number": 313,
                "state": "OPEN",
                "title": "tmuxベースのPTYセッション永続化",
                "updatedAt": "2026-02-17T18:05:54Z",
                "url": "https://github.com/siro33950/releash/issues/313"
            },
            {
                "assignees": [],
                "author": {"id": "MDQ6VXNlcjMwNjAxMTM2", "is_bot": false, "login": "siro33950", "name": "siro33950"},
                "body": "",
                "createdAt": "2026-02-17T17:46:06Z",
                "labels": [{"id": "LA_kwDORH7BOc8AAAACW7Y17Q", "name": "enhancement", "description": "New feature or request", "color": "a2eeef"}],
                "milestone": null,
                "number": 312,
                "state": "OPEN",
                "title": "Notionタスク管理パネルを追加",
                "updatedAt": "2026-02-17T17:56:51Z",
                "url": "https://github.com/siro33950/releash/issues/312"
            }
        ])
        .to_string();
        let issues = parse_gh_issue_list_output(&json);
        assert_eq!(issues.len(), 2, "deserialization failed: got empty vec");
        assert_eq!(issues[0].number, 313);
        assert!(issues[0].milestone.is_some());
        assert_eq!(issues[0].milestone.as_ref().unwrap().title, "マルチPTY管理");
        assert_eq!(issues[1].number, 312);
        assert!(issues[1].milestone.is_none());
        assert_eq!(issues[1].labels.len(), 1);
    }

    /// run_gh_with_timeout と同じパターン（try_wait ポーリング + 後から stdout 読み取り）で
    /// 64KB超の出力がパイプバッファ溢れによりデッドロックすることを再現するテスト。
    ///
    /// macOS のパイプバッファは 64KB。stdout を読まずに try_wait だけ回すと、
    /// 子プロセスが write(2) でブロックし永遠に終了しない。
    #[cfg(unix)]
    #[test]
    fn piped_stdout_deadlocks_over_64kb() {
        // 70KB を stdout に書き出す子プロセス
        let mut child = Command::new("dd")
            .args(["if=/dev/zero", "bs=1024", "count=70"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();

        let timeout = Duration::from_secs(3);
        let start = Instant::now();
        let mut timed_out = false;

        loop {
            match child.try_wait().unwrap() {
                Some(_) => break,
                None => {
                    if start.elapsed() > timeout {
                        let _ = child.kill();
                        let _ = child.wait();
                        timed_out = true;
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        }

        // 64KB超ではパイプバッファが詰まり、子プロセスが終了できずタイムアウトする
        assert!(
            timed_out,
            "expected deadlock timeout, but process exited — pipe buffer may be >64KB on this OS"
        );
    }

    /// 64KB以下の出力では同じパターンでもデッドロックしないことを確認。
    #[cfg(unix)]
    #[test]
    fn piped_stdout_ok_under_64kb() {
        let mut child = Command::new("dd")
            .args(["if=/dev/zero", "bs=1024", "count=60"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();

        let timeout = Duration::from_secs(3);
        let start = Instant::now();
        let mut timed_out = false;

        loop {
            match child.try_wait().unwrap() {
                Some(_) => break,
                None => {
                    if start.elapsed() > timeout {
                        let _ = child.kill();
                        let _ = child.wait();
                        timed_out = true;
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        }

        assert!(!timed_out, "should not deadlock with <64KB output");
    }

    /// stdout を別スレッドで並行読み取りすれば 64KB超でもデッドロックしないことを検証。
    #[cfg(unix)]
    #[test]
    fn piped_stdout_concurrent_read_avoids_deadlock() {
        use std::io::Read;

        let mut child = Command::new("dd")
            .args(["if=/dev/zero", "bs=1024", "count=70"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();

        let mut stdout = child.stdout.take().unwrap();
        let reader = std::thread::spawn(move || {
            let mut buf = Vec::new();
            stdout.read_to_end(&mut buf).unwrap();
            buf
        });

        let timeout = Duration::from_secs(3);
        let start = Instant::now();
        let mut timed_out = false;

        loop {
            match child.try_wait().unwrap() {
                Some(status) => {
                    assert!(status.success());
                    break;
                }
                None => {
                    if start.elapsed() > timeout {
                        let _ = child.kill();
                        let _ = child.wait();
                        timed_out = true;
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        }

        assert!(!timed_out, "concurrent read should prevent deadlock");
        let buf = reader.join().unwrap();
        assert_eq!(buf.len(), 70 * 1024);
    }
}
