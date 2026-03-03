use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use super::detect::LspServerConfig;

#[derive(Debug, thiserror::Error)]
pub enum LspDownloadError {
    #[error("未対応の言語: {0}")]
    UnsupportedLanguage(String),
    #[error("前提条件を満たしていません: {0}")]
    MissingPrerequisite(String),
    #[error("ダウンロード失敗: {0}")]
    Download(String),
    #[error("インストール失敗: {0}")]
    Install(String),
    #[error("IO エラー: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, Serialize)]
pub struct LspDownloadProgress {
    pub language: String,
    pub status: String,
    pub progress: f64,
}

fn emit_progress(app: &AppHandle, language: &str, status: &str, progress: f64) {
    let _ = app.emit(
        "lsp-download-progress",
        LspDownloadProgress {
            language: language.to_string(),
            status: status.to_string(),
            progress,
        },
    );
}

/// Check if a cached LSP server binary exists and return its config.
pub fn get_cached_server(language: &str, lsp_cache_dir: &Path) -> Option<LspServerConfig> {
    match language {
        "rust" => {
            let bin = lsp_cache_dir.join("rust-analyzer");
            if bin.exists() {
                Some(LspServerConfig {
                    command: bin.to_string_lossy().to_string(),
                    args: vec![],
                    enabled: true,
                })
            } else {
                None
            }
        }
        "typescript" => {
            let bin = lsp_cache_dir
                .join("typescript")
                .join("node_modules")
                .join(".bin")
                .join("typescript-language-server");
            if bin.exists() {
                Some(LspServerConfig {
                    command: "node".to_string(),
                    args: vec![bin.to_string_lossy().to_string(), "--stdio".to_string()],
                    enabled: true,
                })
            } else {
                None
            }
        }
        "go" => {
            let bin = lsp_cache_dir.join("gopls");
            if bin.exists() {
                Some(LspServerConfig {
                    command: bin.to_string_lossy().to_string(),
                    args: vec![],
                    enabled: true,
                })
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Install an LSP server for the given language into the cache directory.
pub async fn install_lsp_server(
    app: &AppHandle,
    language: &str,
    lsp_cache_dir: &Path,
) -> Result<LspServerConfig, LspDownloadError> {
    std::fs::create_dir_all(lsp_cache_dir)?;

    match language {
        "rust" => install_rust_analyzer(app, lsp_cache_dir).await,
        "typescript" => install_typescript_lsp(app, lsp_cache_dir).await,
        "go" => install_gopls(app, lsp_cache_dir).await,
        _ => Err(LspDownloadError::UnsupportedLanguage(language.to_string())),
    }
}

/// Download rust-analyzer from GitHub Releases.
async fn install_rust_analyzer(
    app: &AppHandle,
    cache_dir: &Path,
) -> Result<LspServerConfig, LspDownloadError> {
    emit_progress(app, "rust", "downloading", 0.0);

    let target = rust_analyzer_target();
    let version = fetch_latest_rust_analyzer_version().await?;
    let url = format!(
        "https://github.com/rust-lang/rust-analyzer/releases/download/{version}/rust-analyzer-{target}.gz"
    );

    log::info!("Downloading rust-analyzer from {url}");

    let response = reqwest::get(&url)
        .await
        .map_err(|e| LspDownloadError::Download(format!("{url}: {e}")))?;

    if !response.status().is_success() {
        return Err(LspDownloadError::Download(format!(
            "HTTP {}: {url}",
            response.status()
        )));
    }

    emit_progress(app, "rust", "downloading", 0.5);

    let bytes = response
        .bytes()
        .await
        .map_err(|e| LspDownloadError::Download(e.to_string()))?;

    emit_progress(app, "rust", "installing", 0.8);

    // Decompress gzip
    use flate2::read::GzDecoder;
    use std::io::Read;

    let mut decoder = GzDecoder::new(&bytes[..]);
    let mut decompressed = Vec::new();
    decoder
        .read_to_end(&mut decompressed)
        .map_err(|e| LspDownloadError::Install(format!("gzip展開失敗: {e}")))?;

    let bin_path = cache_dir.join("rust-analyzer");
    std::fs::write(&bin_path, &decompressed)?;

    // chmod +x
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bin_path, std::fs::Permissions::from_mode(0o755))?;
    }

    // Save version file
    std::fs::write(cache_dir.join("rust-analyzer.version"), &version)?;

    log::info!(
        "rust-analyzer {version} installed to {}",
        bin_path.display()
    );
    emit_progress(app, "rust", "done", 1.0);

    Ok(LspServerConfig {
        command: bin_path.to_string_lossy().to_string(),
        args: vec![],
        enabled: true,
    })
}

/// Install typescript-language-server via npm.
async fn install_typescript_lsp(
    app: &AppHandle,
    cache_dir: &Path,
) -> Result<LspServerConfig, LspDownloadError> {
    if !is_command_available("node") || !is_command_available("npm") {
        return Err(LspDownloadError::MissingPrerequisite(
            "node と npm が PATH に必要です".to_string(),
        ));
    }

    emit_progress(app, "typescript", "installing", 0.0);

    let prefix = cache_dir.join("typescript");
    std::fs::create_dir_all(&prefix)?;

    let output = tokio::process::Command::new("npm")
        .args([
            "install",
            "--prefix",
            &prefix.to_string_lossy(),
            "typescript-language-server",
            "typescript",
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await
        .map_err(|e| LspDownloadError::Install(format!("npm install 実行失敗: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(LspDownloadError::Install(format!(
            "npm install 失敗: {stderr}"
        )));
    }

    emit_progress(app, "typescript", "done", 1.0);

    let bin = prefix
        .join("node_modules")
        .join(".bin")
        .join("typescript-language-server");

    log::info!("typescript-language-server installed to {}", bin.display());

    Ok(LspServerConfig {
        command: "node".to_string(),
        args: vec![bin.to_string_lossy().to_string(), "--stdio".to_string()],
        enabled: true,
    })
}

/// Install gopls via `go install`.
async fn install_gopls(
    app: &AppHandle,
    cache_dir: &Path,
) -> Result<LspServerConfig, LspDownloadError> {
    if !is_command_available("go") {
        return Err(LspDownloadError::MissingPrerequisite(
            "go が PATH に必要です".to_string(),
        ));
    }

    emit_progress(app, "go", "installing", 0.0);

    let output = tokio::process::Command::new("go")
        .args(["install", "golang.org/x/tools/gopls@latest"])
        .env("GOBIN", cache_dir.as_os_str())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await
        .map_err(|e| LspDownloadError::Install(format!("go install 実行失敗: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(LspDownloadError::Install(format!(
            "go install 失敗: {stderr}"
        )));
    }

    emit_progress(app, "go", "done", 1.0);

    let bin = cache_dir.join("gopls");
    log::info!("gopls installed to {}", bin.display());

    Ok(LspServerConfig {
        command: bin.to_string_lossy().to_string(),
        args: vec![],
        enabled: true,
    })
}

/// Fetch the latest release tag of rust-analyzer from GitHub.
async fn fetch_latest_rust_analyzer_version() -> Result<String, LspDownloadError> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| LspDownloadError::Download(e.to_string()))?;

    let resp = client
        .head("https://github.com/rust-lang/rust-analyzer/releases/latest")
        .send()
        .await
        .map_err(|e| LspDownloadError::Download(format!("バージョン取得失敗: {e}")))?;

    // GitHub redirects to /releases/tag/<version>
    if let Some(location) = resp.headers().get("location") {
        let loc = location.to_str().unwrap_or("");
        if let Some(tag) = loc.rsplit('/').next() {
            return Ok(tag.to_string());
        }
    }

    Err(LspDownloadError::Download(
        "rust-analyzer の最新バージョンを取得できません".to_string(),
    ))
}

/// Get the rust-analyzer target triple for the current platform.
fn rust_analyzer_target() -> &'static str {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "aarch64-apple-darwin"
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "x86_64-apple-darwin"
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "x86_64-unknown-linux-gnu"
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        "aarch64-unknown-linux-gnu"
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        "x86_64-pc-windows-msvc"
    }
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    {
        "aarch64-pc-windows-msvc"
    }
}

fn is_command_available(command: &str) -> bool {
    std::process::Command::new("which")
        .arg(command)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Return the LSP cache directory for the given app.
pub fn lsp_cache_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app_data_dir 取得失敗: {e}"))?;
    Ok(data_dir.join("lsp"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_cached_server_returns_none_when_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(get_cached_server("rust", dir.path()).is_none());
        assert!(get_cached_server("typescript", dir.path()).is_none());
        assert!(get_cached_server("go", dir.path()).is_none());
        assert!(get_cached_server("unknown", dir.path()).is_none());
    }

    #[test]
    fn get_cached_server_rust_returns_config_when_binary_exists() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("rust-analyzer");
        std::fs::write(&bin, b"fake").unwrap();

        let config = get_cached_server("rust", dir.path()).unwrap();
        assert_eq!(config.command, bin.to_string_lossy());
        assert!(config.args.is_empty());
        assert!(config.enabled);
    }

    #[test]
    fn get_cached_server_typescript_returns_config_when_binary_exists() {
        let dir = tempfile::tempdir().unwrap();
        let bin_dir = dir
            .path()
            .join("typescript")
            .join("node_modules")
            .join(".bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let bin = bin_dir.join("typescript-language-server");
        std::fs::write(&bin, b"fake").unwrap();

        let config = get_cached_server("typescript", dir.path()).unwrap();
        assert_eq!(config.command, "node");
        assert_eq!(config.args.len(), 2);
        assert!(config.args[0].contains("typescript-language-server"));
        assert_eq!(config.args[1], "--stdio");
    }

    #[test]
    fn get_cached_server_go_returns_config_when_binary_exists() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("gopls");
        std::fs::write(&bin, b"fake").unwrap();

        let config = get_cached_server("go", dir.path()).unwrap();
        assert_eq!(config.command, bin.to_string_lossy());
        assert!(config.args.is_empty());
    }

    #[test]
    fn rust_analyzer_target_returns_valid_triple() {
        let target = rust_analyzer_target();
        assert!(!target.is_empty());
    }
}
