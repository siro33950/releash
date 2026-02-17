pub mod github;
pub mod types;

use std::collections::HashMap;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use types::{GitHostProvider, IssueInfo, PrDetail, PrStatus, ProviderStatus};

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
    fn check_provider_status_unsupported_platform() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        repo.remote("origin", "https://gitlab.com/user/repo.git")
            .unwrap();
        let status = check_provider_status(dir.path().to_str().unwrap());
        assert!(matches!(status, ProviderStatus::UnsupportedPlatform));
    }
}
