use super::*;

#[test]
fn parse_open_prs_valid_json() {
    let json = r#"[
            {"headRefName":"feat/login","number":42,"url":"https://github.com/owner/repo/pull/42"},
            {"headRefName":"fix/typo","number":7,"url":"https://github.com/owner/repo/pull/7"}
        ]"#;

    let map = parse_gh_pr_list_output(json).unwrap();

    assert_eq!(map.len(), 2);
    let pr = map.get("feat/login").unwrap();
    assert_eq!(pr.number, 42);
    assert_eq!(pr.url, "https://github.com/owner/repo/pull/42");
}

#[test]
fn parse_open_prs_empty_array() {
    let map = parse_gh_pr_list_output("[]").unwrap();

    assert!(map.is_empty());
}

#[test]
fn parse_open_prs_invalid_json() {
    assert!(parse_gh_pr_list_output("not json").is_err());
}

#[test]
fn parse_open_prs_missing_fields() {
    let json = r#"[{"headRefName":"feat/x"}]"#;

    let map = parse_gh_pr_list_output(json).unwrap();

    assert!(map.is_empty());
}

#[test]
fn parse_merged_prs_valid() {
    let json = r#"[{"headRefName":"feat/a"},{"headRefName":"feat/b"}]"#;

    let branches = parse_gh_merged_pr_output(json).unwrap();

    assert_eq!(branches, vec!["feat/a", "feat/b"]);
}

#[test]
fn parse_merged_prs_empty() {
    let branches = parse_gh_merged_pr_output("[]").unwrap();

    assert!(branches.is_empty());
}

#[test]
fn parse_merged_prs_invalid() {
    assert!(parse_gh_merged_pr_output("invalid").is_err());
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
