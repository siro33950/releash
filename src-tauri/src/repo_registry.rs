use std::sync::Arc;

use tauri::{Emitter, Manager};

use crate::config::AppConfig;

pub type SharedRepoPaths = Arc<parking_lot::RwLock<Vec<String>>>;

fn normalize_repo_path(path: &str) -> String {
    let replaced = path.replace('\\', "/");
    let had_unc_prefix = replaced.starts_with("//");
    let mut result = String::with_capacity(replaced.len());
    let mut prev_slash = false;
    for c in replaced.chars() {
        if c == '/' {
            if !prev_slash {
                result.push(c);
            }
            prev_slash = true;
        } else {
            result.push(c);
            prev_slash = false;
        }
    }
    let mut normalized = result.trim_end_matches('/').to_string();
    if had_unc_prefix && normalized.starts_with('/') && !normalized.starts_with("//") {
        normalized.insert(0, '/');
    }
    normalized
}

/// Add a repo path to SharedRepoPaths and persist to config.toml.
/// Returns `true` if the path was newly added, `false` if it already existed.
pub fn add_repo(
    shared: &SharedRepoPaths,
    app_config: &AppConfig,
    path: &str,
) -> Result<bool, String> {
    let normalized = normalize_repo_path(path);
    if normalized.is_empty() {
        return Ok(false);
    }

    let mut paths = shared.write();
    if paths.iter().any(|p| p == &normalized) {
        return Ok(false);
    }

    let mut new_paths = paths.clone();
    new_paths.push(normalized);

    app_config.with_config_mut(|config| {
        config.app.last_repo_paths = new_paths.clone();
        Ok(())
    })?;

    *paths = new_paths;
    Ok(true)
}

pub fn remove_repo(
    shared: &SharedRepoPaths,
    app_config: &AppConfig,
    path: &str,
) -> Result<bool, String> {
    let normalized = normalize_repo_path(path);

    let mut paths = shared.write();
    let new_paths: Vec<String> = paths
        .iter()
        .filter(|p| *p != &normalized)
        .cloned()
        .collect();
    if new_paths.len() == paths.len() {
        return Ok(false);
    }

    app_config.with_config_mut(|config| {
        config.app.last_repo_paths = new_paths.clone();
        Ok(())
    })?;

    *paths = new_paths;
    Ok(true)
}

pub fn get_repos(shared: &SharedRepoPaths) -> Vec<String> {
    shared.read().clone()
}

#[tauri::command]
pub fn get_repo_paths(shared: tauri::State<'_, SharedRepoPaths>) -> Vec<String> {
    get_repos(shared.inner())
}

#[tauri::command]
pub async fn add_repo_path(
    path: String,
    shared: tauri::State<'_, SharedRepoPaths>,
    app_config: tauri::State<'_, Arc<AppConfig>>,
    app: tauri::AppHandle,
) -> Result<bool, String> {
    let shared = Arc::clone(shared.inner());
    let app_config = Arc::clone(app_config.inner());
    let added = tokio::task::spawn_blocking(move || add_repo(&shared, &app_config, &path))
        .await
        .map_err(|e| format!("task join error: {e}"))??;

    if added {
        let current = get_repos(&app.state::<SharedRepoPaths>());
        let _ = app.emit("repo-paths-changed", &current);
    }

    Ok(added)
}

#[tauri::command]
pub async fn remove_repo_path(
    path: String,
    shared: tauri::State<'_, SharedRepoPaths>,
    app_config: tauri::State<'_, Arc<AppConfig>>,
    app: tauri::AppHandle,
) -> Result<bool, String> {
    let shared = Arc::clone(shared.inner());
    let app_config = Arc::clone(app_config.inner());
    let removed = tokio::task::spawn_blocking(move || remove_repo(&shared, &app_config, &path))
        .await
        .map_err(|e| format!("task join error: {e}"))??;

    if removed {
        let current = get_repos(&app.state::<SharedRepoPaths>());
        let _ = app.emit("repo-paths-changed", &current);
    }

    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AppConfig, ReleashConfig};
    use tempfile::TempDir;

    fn make_shared() -> SharedRepoPaths {
        Arc::new(parking_lot::RwLock::new(Vec::new()))
    }

    fn make_config(dir: &TempDir) -> AppConfig {
        let path = dir.path().join("releash.toml");
        let config = ReleashConfig::default();
        AppConfig::new(config, path)
    }

    #[test]
    fn normalize_trims_trailing_slash() {
        assert_eq!(normalize_repo_path("/repo/path/"), "/repo/path");
    }

    #[test]
    fn normalize_backslash_to_forward() {
        assert_eq!(
            normalize_repo_path("C:\\Users\\test\\repo"),
            "C:/Users/test/repo"
        );
    }

    #[test]
    fn normalize_collapses_consecutive_slashes() {
        assert_eq!(normalize_repo_path("/repo//path///sub"), "/repo/path/sub");
    }

    #[test]
    fn normalize_combined() {
        assert_eq!(
            normalize_repo_path("C:\\Users\\\\test\\repo/"),
            "C:/Users/test/repo"
        );
    }

    #[test]
    fn normalize_preserves_unc_prefix() {
        assert_eq!(
            normalize_repo_path("\\\\server\\share\\repo"),
            "//server/share/repo"
        );
    }

    #[test]
    fn normalize_preserves_unc_with_duplicate_slashes() {
        assert_eq!(
            normalize_repo_path("\\\\server\\share\\\\repo"),
            "//server/share/repo"
        );
    }

    #[test]
    fn add_repo_new_path() {
        let dir = TempDir::new().unwrap();
        let shared = make_shared();
        let config = make_config(&dir);

        let added = add_repo(&shared, &config, "/repo/a").unwrap();
        assert!(added);
        assert_eq!(get_repos(&shared), vec!["/repo/a"]);
    }

    #[test]
    fn add_repo_duplicate_returns_false() {
        let dir = TempDir::new().unwrap();
        let shared = make_shared();
        let config = make_config(&dir);

        add_repo(&shared, &config, "/repo/a").unwrap();
        let added = add_repo(&shared, &config, "/repo/a").unwrap();
        assert!(!added);
        assert_eq!(get_repos(&shared).len(), 1);
    }

    #[test]
    fn add_repo_normalizes_before_dedup() {
        let dir = TempDir::new().unwrap();
        let shared = make_shared();
        let config = make_config(&dir);

        add_repo(&shared, &config, "/repo/a/").unwrap();
        let added = add_repo(&shared, &config, "/repo/a").unwrap();
        assert!(!added);
    }

    #[test]
    fn add_repo_empty_path_returns_false() {
        let dir = TempDir::new().unwrap();
        let shared = make_shared();
        let config = make_config(&dir);

        let added = add_repo(&shared, &config, "").unwrap();
        assert!(!added);
        assert!(get_repos(&shared).is_empty());
    }

    #[test]
    fn remove_repo_existing() {
        let dir = TempDir::new().unwrap();
        let shared = make_shared();
        let config = make_config(&dir);

        add_repo(&shared, &config, "/repo/a").unwrap();
        let removed = remove_repo(&shared, &config, "/repo/a").unwrap();
        assert!(removed);
        assert!(get_repos(&shared).is_empty());
    }

    #[test]
    fn remove_repo_nonexistent_returns_false() {
        let dir = TempDir::new().unwrap();
        let shared = make_shared();
        let config = make_config(&dir);

        let removed = remove_repo(&shared, &config, "/repo/a").unwrap();
        assert!(!removed);
    }

    #[test]
    fn add_persists_to_config() {
        let dir = TempDir::new().unwrap();
        let shared = make_shared();
        let config = make_config(&dir);

        add_repo(&shared, &config, "/repo/a").unwrap();
        add_repo(&shared, &config, "/repo/b").unwrap();

        let cfg = config.get_config().unwrap();
        assert_eq!(cfg.app.last_repo_paths, vec!["/repo/a", "/repo/b"]);
    }

    #[test]
    fn remove_persists_to_config() {
        let dir = TempDir::new().unwrap();
        let shared = make_shared();
        let config = make_config(&dir);

        add_repo(&shared, &config, "/repo/a").unwrap();
        add_repo(&shared, &config, "/repo/b").unwrap();
        remove_repo(&shared, &config, "/repo/a").unwrap();

        let cfg = config.get_config().unwrap();
        assert_eq!(cfg.app.last_repo_paths, vec!["/repo/b"]);
    }
}
