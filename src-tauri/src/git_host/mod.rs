pub mod github;
pub mod types;

use std::collections::HashMap;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use types::{
    GitHostProvider, IssueInfo, PostedComment, PrDetail, PrFile, PrReviewComment, PrStatus,
    ProviderStatus,
};

const PR_CACHE_TTL: Duration = Duration::from_secs(30);
const PR_DETAIL_CACHE_TTL: Duration = Duration::from_secs(60);
const ISSUE_CACHE_TTL: Duration = Duration::from_secs(30);

struct CacheEntry {
    value: PrStatus,
    fetched_at: Instant,
}

pub struct PrCache {
    entries: Mutex<HashMap<String, CacheEntry>>,
}

impl PrCache {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }
}

struct PrDetailCacheEntry {
    value: PrDetail,
    fetched_at: Instant,
}

pub struct PrDetailCache {
    entries: Mutex<HashMap<String, PrDetailCacheEntry>>,
}

impl PrDetailCache {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }
}

struct IssueCacheEntry {
    value: Vec<IssueInfo>,
    fetched_at: Instant,
}

pub struct IssueCache {
    entries: Mutex<HashMap<String, IssueCacheEntry>>,
}

impl IssueCache {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }
}

use crate::protocol::thread::{Thread, ThreadEntry};

/// Parse a subset of RFC 3339 timestamps into milliseconds since epoch.
/// Supports formats: "YYYY-MM-DDTHH:MM:SSZ" and "YYYY-MM-DDTHH:MM:SS+HH:MM" / "-HH:MM".
fn parse_rfc3339_to_millis(s: &str) -> f64 {
    // Minimum: "2024-01-01T00:00:00Z" (20 chars)
    if s.len() < 20 {
        return 0.0;
    }
    let parse_u = |slice: &str| -> i64 { slice.parse::<i64>().unwrap_or(0) };

    let year = parse_u(&s[0..4]);
    let month = parse_u(&s[5..7]);
    let day = parse_u(&s[8..10]);
    let hour = parse_u(&s[11..13]);
    let min = parse_u(&s[14..16]);
    let sec = parse_u(&s[17..19]);

    // Days from epoch (1970-01-01) to the given date
    let days = days_from_civil(year, month, day);
    let mut epoch_secs = days * 86400 + hour * 3600 + min * 60 + sec;

    // Handle timezone offset
    let tz_part = &s[19..];
    if !tz_part.is_empty() && tz_part != "Z" {
        let sign: i64 = if tz_part.starts_with('-') { 1 } else { -1 };
        if tz_part.len() >= 6 {
            let tz_h = parse_u(&tz_part[1..3]);
            let tz_m = parse_u(&tz_part[4..6]);
            epoch_secs += sign * (tz_h * 3600 + tz_m * 60);
        }
    }

    epoch_secs as f64 * 1000.0
}

/// Compute the number of days from 1970-01-01 to y/m/d (civil calendar).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let m_adj = if m > 2 { m - 3 } else { m + 9 } as u64;
    let doy = (153 * m_adj + 2) / 5 + (d as u64).saturating_sub(1);
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe as i64 - 719468
}

/// Convert PR review comments into Thread objects.
/// Mirrors the frontend `prReviewCommentsToThreads` logic.
pub(crate) fn pr_review_comments_to_threads(comments: Vec<PrReviewComment>) -> Vec<Thread> {
    let mut root_comments: Vec<&PrReviewComment> = Vec::new();
    let mut replies: HashMap<u64, Vec<&PrReviewComment>> = HashMap::new();

    for comment in &comments {
        if let Some(parent_id) = comment.in_reply_to_id {
            replies.entry(parent_id).or_default().push(comment);
        } else {
            root_comments.push(comment);
        }
    }

    root_comments
        .into_iter()
        .map(|root| {
            let root_entry = comment_to_entry(root);
            let mut child_entries: Vec<ThreadEntry> = replies
                .get(&root.id)
                .map(|reps| {
                    let mut sorted = reps.clone();
                    sorted.sort_by(|a, b| {
                        let a_ms = parse_rfc3339_to_millis(&a.created_at);
                        let b_ms = parse_rfc3339_to_millis(&b.created_at);
                        a_ms.partial_cmp(&b_ms).unwrap_or(std::cmp::Ordering::Equal)
                    });
                    sorted.into_iter().map(comment_to_entry).collect()
                })
                .unwrap_or_default();

            let mut entries = vec![root_entry];
            entries.append(&mut child_entries);

            let line_number = root.line.or(root.original_line).unwrap_or(1);

            Thread {
                id: format!("pr-comment-{}", root.id),
                file_path: root.path.clone(),
                line_number,
                end_line: None,
                entries,
                resolved: false,
                severity: None,
                anchor: None,
                created_at: parse_rfc3339_to_millis(&root.created_at),
            }
        })
        .collect()
}

fn comment_to_entry(comment: &PrReviewComment) -> ThreadEntry {
    ThreadEntry {
        id: format!("pr-entry-{}", comment.id),
        content: comment.body.clone(),
        is_ai: false,
        action: None,
        author_name: Some(comment.author.login.clone()),
        author_avatar_url: comment.author.avatar_url.clone(),
        pr_comment_id: Some(comment.id),
        created_at: parse_rfc3339_to_millis(&comment.created_at),
    }
}

pub fn create_provider(repo_path: &str) -> Option<Box<dyn GitHostProvider>> {
    let remote_url = get_origin_url(repo_path)?;
    if remote_url.contains("github.com") {
        Some(Box::new(github::GitHubProvider))
    } else {
        None
    }
}

#[tauri::command]
pub async fn check_pr_provider_status(repo_path: String) -> Result<ProviderStatus, String> {
    tokio::task::spawn_blocking(move || check_provider_status(&repo_path))
        .await
        .map_err(|e| format!("task join error: {e}"))
}

pub(crate) fn fetch_pr_status_inner(repo_path: &str) -> PrStatus {
    let provider = create_provider(repo_path);
    match provider {
        Some(p) => PrStatus {
            open_prs: p.detect_open_prs(repo_path),
            merged_branches: p.detect_merged_prs(repo_path),
        },
        None => PrStatus::default(),
    }
}

pub(crate) fn fetch_pr_status_with_cache(cache: &PrCache, repo_path: &str) -> PrStatus {
    if let Ok(map) = cache.entries.lock() {
        if let Some(entry) = map.get(repo_path) {
            if entry.fetched_at.elapsed() < PR_CACHE_TTL {
                return entry.value.clone();
            }
        }
    }
    let status = fetch_pr_status_inner(repo_path);
    if let Ok(mut map) = cache.entries.lock() {
        map.retain(|_, entry| entry.fetched_at.elapsed() < PR_CACHE_TTL);
        map.insert(
            repo_path.to_string(),
            CacheEntry {
                value: status.clone(),
                fetched_at: Instant::now(),
            },
        );
    }
    status
}

#[tauri::command]
pub async fn fetch_pr_status(repo_path: String) -> Result<PrStatus, String> {
    tokio::task::spawn_blocking(move || fetch_pr_status_inner(&repo_path))
        .await
        .map_err(|e| format!("task join error: {e}"))
}

fn fetch_pr_detail_inner(repo_path: &str, pr_number: u64) -> Option<PrDetail> {
    let provider = create_provider(repo_path)?;
    provider.get_pr_detail(repo_path, pr_number)
}

fn fetch_pr_detail_with_cache(
    cache: &PrDetailCache,
    repo_path: &str,
    pr_number: u64,
) -> Option<PrDetail> {
    let key = format!("{repo_path}::{pr_number}");
    if let Ok(map) = cache.entries.lock() {
        if let Some(entry) = map.get(&key) {
            if entry.fetched_at.elapsed() < PR_DETAIL_CACHE_TTL {
                return Some(entry.value.clone());
            }
        }
    }
    let detail = fetch_pr_detail_inner(repo_path, pr_number)?;
    if let Ok(mut map) = cache.entries.lock() {
        map.retain(|_, entry| entry.fetched_at.elapsed() < PR_DETAIL_CACHE_TTL);
        map.insert(
            key,
            PrDetailCacheEntry {
                value: detail.clone(),
                fetched_at: Instant::now(),
            },
        );
    }
    Some(detail)
}

#[tauri::command]
pub async fn get_pr_detail(
    cache: tauri::State<'_, Arc<PrDetailCache>>,
    repo_path: String,
    pr_number: u64,
) -> Result<Option<PrDetail>, String> {
    let cache = Arc::clone(&cache);
    tokio::task::spawn_blocking(move || fetch_pr_detail_with_cache(&cache, &repo_path, pr_number))
        .await
        .map_err(|e| format!("task join error: {e}"))
}

#[tauri::command]
pub async fn get_cached_pr_status(
    cache: tauri::State<'_, Arc<PrCache>>,
    repo_path: String,
) -> Result<PrStatus, String> {
    let cache = Arc::clone(&cache);
    let status =
        tokio::task::spawn_blocking(move || fetch_pr_status_with_cache(&cache, &repo_path))
            .await
            .map_err(|e| format!("task join error: {e}"))?;
    Ok(status)
}

fn fetch_issues_inner(repo_path: &str) -> Vec<IssueInfo> {
    let provider = create_provider(repo_path);
    match provider {
        Some(p) => p.list_issues(repo_path),
        None => Vec::new(),
    }
}

fn fetch_issues_with_cache(cache: &IssueCache, repo_path: &str) -> Vec<IssueInfo> {
    if let Ok(map) = cache.entries.lock() {
        if let Some(entry) = map.get(repo_path) {
            if entry.fetched_at.elapsed() < ISSUE_CACHE_TTL {
                return entry.value.clone();
            }
        }
    }
    let issues = fetch_issues_inner(repo_path);
    if let Ok(mut map) = cache.entries.lock() {
        map.retain(|_, entry| entry.fetched_at.elapsed() < ISSUE_CACHE_TTL);
        map.insert(
            repo_path.to_string(),
            IssueCacheEntry {
                value: issues.clone(),
                fetched_at: Instant::now(),
            },
        );
    }
    issues
}

#[tauri::command]
pub async fn fetch_issues(repo_path: String) -> Result<Vec<IssueInfo>, String> {
    tokio::task::spawn_blocking(move || fetch_issues_inner(&repo_path))
        .await
        .map_err(|e| format!("task join error: {e}"))
}

fn fetch_pr_files_inner(repo_path: &str, pr_number: u64) -> Vec<PrFile> {
    let provider = create_provider(repo_path);
    match provider {
        Some(p) => p.get_pr_files(repo_path, pr_number),
        None => Vec::new(),
    }
}

#[tauri::command]
pub async fn get_pr_files(repo_path: String, pr_number: u64) -> Result<Vec<PrFile>, String> {
    tokio::task::spawn_blocking(move || fetch_pr_files_inner(&repo_path, pr_number))
        .await
        .map_err(|e| format!("task join error: {e}"))
}

pub(crate) fn fetch_pr_review_comments_inner(
    repo_path: &str,
    pr_number: u64,
) -> Vec<PrReviewComment> {
    let provider = create_provider(repo_path);
    match provider {
        Some(p) => p.get_pr_review_comments(repo_path, pr_number),
        None => Vec::new(),
    }
}

#[tauri::command]
pub async fn get_pr_review_comments(
    repo_path: String,
    pr_number: u64,
) -> Result<Vec<PrReviewComment>, String> {
    tokio::task::spawn_blocking(move || fetch_pr_review_comments_inner(&repo_path, pr_number))
        .await
        .map_err(|e| format!("task join error: {e}"))
}

fn reply_to_pr_review_comment_inner(
    repo_path: &str,
    pr_number: u64,
    comment_id: u64,
    body: &str,
) -> Option<PostedComment> {
    let provider = create_provider(repo_path)?;
    provider.reply_to_pr_review_comment(repo_path, pr_number, comment_id, body)
}

#[tauri::command]
pub async fn reply_to_pr_review_comment(
    repo_path: String,
    pr_number: u64,
    comment_id: u64,
    body: String,
) -> Result<PostedComment, String> {
    tokio::task::spawn_blocking(move || {
        reply_to_pr_review_comment_inner(&repo_path, pr_number, comment_id, &body)
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
    .ok_or_else(|| "Failed to post reply".to_string())
}

fn post_pr_comment_inner(repo_path: &str, pr_number: u64, body: &str) -> Option<PostedComment> {
    let provider = create_provider(repo_path)?;
    provider.post_pr_comment(repo_path, pr_number, body)
}

#[tauri::command]
pub async fn post_pr_comment(
    repo_path: String,
    pr_number: u64,
    body: String,
) -> Result<PostedComment, String> {
    tokio::task::spawn_blocking(move || post_pr_comment_inner(&repo_path, pr_number, &body))
        .await
        .map_err(|e| format!("task join error: {e}"))?
        .ok_or_else(|| "Failed to post comment".to_string())
}

#[tauri::command]
pub async fn get_cached_issues(
    cache: tauri::State<'_, Arc<IssueCache>>,
    repo_path: String,
) -> Result<Vec<IssueInfo>, String> {
    let cache = Arc::clone(&cache);
    tokio::task::spawn_blocking(move || fetch_issues_with_cache(&cache, &repo_path))
        .await
        .map_err(|e| format!("task join error: {e}"))
}

pub fn check_provider_status(repo_path: &str) -> ProviderStatus {
    let remote_url = match get_origin_url(repo_path) {
        Some(url) => url,
        None => return ProviderStatus::NoRemote,
    };

    if remote_url.contains("github.com") {
        check_github_status()
    } else {
        ProviderStatus::UnsupportedPlatform
    }
}

fn check_github_status() -> ProviderStatus {
    let cli_exists = Command::new("gh")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if !cli_exists {
        return ProviderStatus::CliNotFound {
            cli: "gh".to_string(),
        };
    }

    let auth_ok = Command::new("gh")
        .args(["auth", "status"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if !auth_ok {
        return ProviderStatus::NotAuthenticated;
    }

    ProviderStatus::Available
}

fn get_origin_url(repo_path: &str) -> Option<String> {
    let repo = git2::Repository::open(repo_path).ok()?;
    let remote = repo.find_remote("origin").ok()?;
    remote.url().map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::PrReviewCommentAuthor;

    #[test]
    fn get_origin_url_no_remote() {
        let dir = tempfile::TempDir::new().unwrap();
        git2::Repository::init(dir.path()).unwrap();
        assert!(get_origin_url(dir.path().to_str().unwrap()).is_none());
    }

    #[test]
    fn get_origin_url_with_github_remote() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        repo.remote("origin", "https://github.com/user/repo.git")
            .unwrap();
        let url = get_origin_url(dir.path().to_str().unwrap()).unwrap();
        assert!(url.contains("github.com"));
    }

    #[test]
    fn create_provider_returns_some_for_github() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        repo.remote("origin", "git@github.com:user/repo.git")
            .unwrap();
        let provider = create_provider(dir.path().to_str().unwrap());
        assert!(provider.is_some());
    }

    #[test]
    fn create_provider_returns_none_for_unknown() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        repo.remote("origin", "https://gitlab.com/user/repo.git")
            .unwrap();
        let provider = create_provider(dir.path().to_str().unwrap());
        assert!(provider.is_none());
    }

    #[test]
    fn create_provider_returns_none_for_no_remote() {
        let dir = tempfile::TempDir::new().unwrap();
        git2::Repository::init(dir.path()).unwrap();
        let provider = create_provider(dir.path().to_str().unwrap());
        assert!(provider.is_none());
    }

    #[test]
    fn check_provider_status_no_remote() {
        let dir = tempfile::TempDir::new().unwrap();
        git2::Repository::init(dir.path()).unwrap();
        let status = check_provider_status(dir.path().to_str().unwrap());
        assert!(matches!(status, ProviderStatus::NoRemote));
    }

    #[test]
    fn parse_rfc3339_to_millis_utc() {
        let ms = parse_rfc3339_to_millis("2024-01-01T00:00:00Z");
        assert_eq!(ms, 1704067200000.0);
    }

    #[test]
    fn parse_rfc3339_to_millis_with_offset() {
        // 2024-01-01T09:00:00+09:00 == 2024-01-01T00:00:00Z
        let ms = parse_rfc3339_to_millis("2024-01-01T09:00:00+09:00");
        assert_eq!(ms, 1704067200000.0);
    }

    #[test]
    fn parse_rfc3339_to_millis_invalid() {
        assert_eq!(parse_rfc3339_to_millis(""), 0.0);
        assert_eq!(parse_rfc3339_to_millis("not-a-date"), 0.0);
    }

    fn make_pr_review_comment(overrides: Option<PrReviewComment>) -> PrReviewComment {
        let default = PrReviewComment {
            id: 1,
            path: "src/main.rs".to_string(),
            line: Some(10),
            original_line: None,
            body: "test comment".to_string(),
            author: PrReviewCommentAuthor {
                login: "reviewer".to_string(),
                avatar_url: None,
            },
            in_reply_to_id: None,
            created_at: "2024-01-01T00:00:00Z".to_string(),
        };
        overrides.unwrap_or(default)
    }

    #[test]
    fn pr_review_comments_to_threads_single_root() {
        let threads = pr_review_comments_to_threads(vec![make_pr_review_comment(None)]);
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].id, "pr-comment-1");
        assert_eq!(threads[0].file_path, "src/main.rs");
        assert_eq!(threads[0].line_number, 10);
        assert_eq!(threads[0].entries.len(), 1);
        assert_eq!(threads[0].entries[0].content, "test comment");
        assert_eq!(threads[0].entries[0].pr_comment_id, Some(1));
        assert_eq!(
            threads[0].entries[0].author_name.as_deref(),
            Some("reviewer")
        );
    }

    #[test]
    fn pr_review_comments_to_threads_with_replies() {
        let comments = vec![
            PrReviewComment {
                id: 1,
                body: "root".to_string(),
                ..make_pr_review_comment(None)
            },
            PrReviewComment {
                id: 2,
                body: "reply".to_string(),
                in_reply_to_id: Some(1),
                created_at: "2024-01-01T01:00:00Z".to_string(),
                ..make_pr_review_comment(None)
            },
        ];
        let threads = pr_review_comments_to_threads(comments);
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].entries.len(), 2);
        assert_eq!(threads[0].entries[0].content, "root");
        assert_eq!(threads[0].entries[1].content, "reply");
    }

    #[test]
    fn pr_review_comments_to_threads_sort_replies() {
        let comments = vec![
            PrReviewComment {
                id: 1,
                body: "root".to_string(),
                ..make_pr_review_comment(None)
            },
            PrReviewComment {
                id: 3,
                body: "later".to_string(),
                in_reply_to_id: Some(1),
                created_at: "2024-01-01T03:00:00Z".to_string(),
                ..make_pr_review_comment(None)
            },
            PrReviewComment {
                id: 2,
                body: "earlier".to_string(),
                in_reply_to_id: Some(1),
                created_at: "2024-01-01T01:00:00Z".to_string(),
                ..make_pr_review_comment(None)
            },
        ];
        let threads = pr_review_comments_to_threads(comments);
        assert_eq!(threads[0].entries[1].content, "earlier");
        assert_eq!(threads[0].entries[2].content, "later");
    }

    #[test]
    fn pr_review_comments_to_threads_original_line_fallback() {
        let threads = pr_review_comments_to_threads(vec![PrReviewComment {
            line: None,
            original_line: Some(25),
            ..make_pr_review_comment(None)
        }]);
        assert_eq!(threads[0].line_number, 25);
    }

    #[test]
    fn pr_review_comments_to_threads_default_line_number() {
        let threads = pr_review_comments_to_threads(vec![PrReviewComment {
            line: None,
            original_line: None,
            ..make_pr_review_comment(None)
        }]);
        assert_eq!(threads[0].line_number, 1);
    }

    #[test]
    fn pr_review_comments_to_threads_empty() {
        let threads = pr_review_comments_to_threads(vec![]);
        assert!(threads.is_empty());
    }

    #[test]
    fn pr_review_comments_to_threads_separate_root_comments() {
        let comments = vec![
            PrReviewComment {
                id: 1,
                path: "a.rs".to_string(),
                line: Some(10),
                body: "first".to_string(),
                ..make_pr_review_comment(None)
            },
            PrReviewComment {
                id: 2,
                path: "b.rs".to_string(),
                line: Some(20),
                body: "second".to_string(),
                ..make_pr_review_comment(None)
            },
        ];
        let threads = pr_review_comments_to_threads(comments);
        assert_eq!(threads.len(), 2);
        assert_eq!(threads[0].file_path, "a.rs");
        assert_eq!(threads[1].file_path, "b.rs");
    }

    #[test]
    fn pr_review_comments_to_threads_preserves_avatar_url() {
        let threads = pr_review_comments_to_threads(vec![PrReviewComment {
            author: PrReviewCommentAuthor {
                login: "user1".to_string(),
                avatar_url: Some("https://avatars.example.com/1".to_string()),
            },
            ..make_pr_review_comment(None)
        }]);
        assert_eq!(
            threads[0].entries[0].author_avatar_url.as_deref(),
            Some("https://avatars.example.com/1")
        );
    }

    #[test]
    fn pr_review_comments_to_threads_sets_pr_comment_id() {
        let comments = vec![
            PrReviewComment {
                id: 100,
                body: "root".to_string(),
                ..make_pr_review_comment(None)
            },
            PrReviewComment {
                id: 200,
                body: "reply".to_string(),
                in_reply_to_id: Some(100),
                ..make_pr_review_comment(None)
            },
        ];
        let threads = pr_review_comments_to_threads(comments);
        assert_eq!(threads[0].entries[0].pr_comment_id, Some(100));
        assert_eq!(threads[0].entries[1].pr_comment_id, Some(200));
    }

    #[test]
    fn check_provider_status_unsupported_platform() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        repo.remote("origin", "https://gitlab.com/user/repo.git")
            .unwrap();
        let status = check_provider_status(dir.path().to_str().unwrap());
        assert!(matches!(status, ProviderStatus::UnsupportedPlatform));
    }
}
