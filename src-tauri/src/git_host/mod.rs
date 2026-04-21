pub mod github;
pub mod types;

use std::collections::HashMap;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use types::{GitHostProvider, IssueInfo, PrStatus, ProviderStatus};

const PR_CACHE_TTL: Duration = Duration::from_secs(30);
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

/// Parse a subset of RFC 3339 timestamps into milliseconds since epoch.
/// Supports formats: "YYYY-MM-DDTHH:MM:SSZ" and "YYYY-MM-DDTHH:MM:SS+HH:MM" / "-HH:MM".
#[cfg(test)]
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
#[cfg(test)]
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let m_adj = if m > 2 { m - 3 } else { m + 9 } as u64;
    let doy = (153 * m_adj + 2) / 5 + (d as u64).saturating_sub(1);
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe as i64 - 719468
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
