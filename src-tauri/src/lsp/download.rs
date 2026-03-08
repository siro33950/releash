use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use super::detect::{is_command_available, LspServerConfig};

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

fn cached_binary_config(bin_path: PathBuf) -> Option<LspServerConfig> {
    if bin_path.exists() {
        Some(LspServerConfig {
            command: bin_path.to_string_lossy().to_string(),
            args: vec![],
            enabled: true,
        })
    } else {
        None
    }
}

/// Check if a cached LSP server binary exists and return its config.
pub fn get_cached_server(language: &str, lsp_cache_dir: &Path) -> Option<LspServerConfig> {
    match language {
        "rust" => cached_binary_config(lsp_cache_dir.join("rust-analyzer")),
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
        "go" => cached_binary_config(lsp_cache_dir.join("gopls")),
        "java" => {
            let launcher = if cfg!(windows) { "jdtls.bat" } else { "jdtls" };
            cached_binary_config(lsp_cache_dir.join("jdtls").join("bin").join(launcher))
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
        "java" => install_jdtls(app, lsp_cache_dir).await,
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
        if let Some(tag) = loc.rsplit('/').next().filter(|t| !t.is_empty()) {
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

/// Check that Java 17+ is available on PATH.
fn check_java_version() -> Result<(), LspDownloadError> {
    let output = std::process::Command::new("java")
        .arg("-version")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|_| {
            LspDownloadError::MissingPrerequisite("java が PATH に必要です (JDK 17+)".to_string())
        })?;

    // java -version outputs to stderr
    let version_str = String::from_utf8_lossy(&output.stderr);
    if version_str.is_empty() {
        let version_str = String::from_utf8_lossy(&output.stdout);
        return parse_java_version(&version_str);
    }
    parse_java_version(&version_str)
}

fn parse_java_version(version_str: &str) -> Result<(), LspDownloadError> {
    // Patterns: "openjdk version \"17.0.1\"", "java version \"1.8.0_291\""
    let major = version_str.lines().find_map(|line| {
        let start = line.find('"')?;
        let end = line[start + 1..].find('"')? + start + 1;
        let ver = &line[start + 1..end];
        let first_segment = ver.split('.').next()?;
        let num: u32 = first_segment.parse().ok()?;
        // "1.8.x" style → major is the second segment
        if num == 1 {
            ver.split('.').nth(1)?.parse().ok()
        } else {
            Some(num)
        }
    });

    match major {
        Some(v) if v >= 17 => Ok(()),
        Some(v) => Err(LspDownloadError::MissingPrerequisite(format!(
            "JDK 17+ が必要です (検出: Java {v})"
        ))),
        None => Err(LspDownloadError::MissingPrerequisite(
            "Java バージョンを検出できません。JDK 17+ をインストールしてください".to_string(),
        )),
    }
}

/// Download and install Eclipse JDT LS from GitHub Releases.
async fn install_jdtls(
    app: &AppHandle,
    cache_dir: &Path,
) -> Result<LspServerConfig, LspDownloadError> {
    check_java_version()?;

    emit_progress(app, "java", "fetching version", 0.0);

    let (download_url, version) = fetch_latest_jdtls_release().await?;

    log::info!("Downloading Eclipse JDT LS {version} from {download_url}");
    emit_progress(app, "java", "downloading", 0.1);

    let client = reqwest::Client::new();
    let response = client
        .get(&download_url)
        .header("User-Agent", "releash")
        .send()
        .await
        .map_err(|e| LspDownloadError::Download(format!("{download_url}: {e}")))?;

    if !response.status().is_success() {
        return Err(LspDownloadError::Download(format!(
            "HTTP {}: {download_url}",
            response.status()
        )));
    }

    emit_progress(app, "java", "downloading", 0.5);

    let bytes = response
        .bytes()
        .await
        .map_err(|e| LspDownloadError::Download(e.to_string()))?;

    emit_progress(app, "java", "extracting", 0.7);

    let jdtls_dir = cache_dir.join("jdtls");
    let staging_dir = cache_dir.join(format!(".jdtls.tmp.{}", uuid::Uuid::new_v4()));
    if staging_dir.exists() {
        std::fs::remove_dir_all(&staging_dir)?;
    }
    std::fs::create_dir_all(&staging_dir)?;

    extract_tar_gz(&bytes, &staging_dir)?;

    let launcher = if cfg!(windows) { "jdtls.bat" } else { "jdtls" };
    let staged_bin = staging_dir.join("bin").join(launcher);

    // chmod +x on the launcher script
    #[cfg(unix)]
    {
        if staged_bin.exists() {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&staged_bin, std::fs::Permissions::from_mode(0o755))?;
        }
    }

    if !staged_bin.exists() {
        let _ = std::fs::remove_dir_all(&staging_dir);
        return Err(LspDownloadError::Install(format!(
            "展開されたアーカイブに bin/{launcher} が見つかりません"
        )));
    }

    // Atomically replace the live directory
    if jdtls_dir.exists() {
        std::fs::remove_dir_all(&jdtls_dir)?;
    }
    std::fs::rename(&staging_dir, &jdtls_dir)?;

    let bin_path = jdtls_dir.join("bin").join(launcher);

    // Save version file (after confirming bin/jdtls exists)
    std::fs::write(cache_dir.join("jdtls.version"), &version)?;

    log::info!(
        "Eclipse JDT LS {version} installed to {}",
        bin_path.display()
    );
    emit_progress(app, "java", "done", 1.0);

    Ok(LspServerConfig {
        command: bin_path.to_string_lossy().to_string(),
        args: vec![],
        enabled: true,
    })
}

/// Fetch the latest Eclipse JDT LS release info.
///
/// JDT LS はGitHub Releasesではなく Eclipse ダウンロードサーバーで配布されている。
/// 1. GitHub Tags API で最新バージョンを取得
/// 2. Eclipse サーバーの latest.txt からtar.gzファイル名を解決
///
/// Returns (download_url, version).
async fn fetch_latest_jdtls_release() -> Result<(String, String), LspDownloadError> {
    let client = reqwest::Client::new();

    // 1. GitHub Tags API で最新バージョンを取得
    let tags_resp = client
        .get("https://api.github.com/repos/eclipse-jdtls/eclipse.jdt.ls/tags?per_page=1")
        .header("User-Agent", "releash")
        .send()
        .await
        .map_err(|e| LspDownloadError::Download(format!("JDT LS タグ取得失敗: {e}")))?;

    if !tags_resp.status().is_success() {
        return Err(LspDownloadError::Download(format!(
            "GitHub API HTTP {}: JDT LS タグ取得",
            tags_resp.status()
        )));
    }

    let tags: serde_json::Value = tags_resp
        .json()
        .await
        .map_err(|e| LspDownloadError::Download(format!("JSON パース失敗: {e}")))?;

    let tag = tags
        .as_array()
        .and_then(|arr| arr.first())
        .and_then(|t| t["name"].as_str())
        .ok_or_else(|| LspDownloadError::Download("JDT LS タグが見つかりません".to_string()))?;

    let version = tag.strip_prefix('v').unwrap_or(tag);

    // 2. Eclipse ダウンロードサーバーの latest.txt からファイル名を取得
    let latest_txt_url =
        format!("https://download.eclipse.org/jdtls/milestones/{version}/latest.txt");
    let latest_resp = client
        .get(&latest_txt_url)
        .send()
        .await
        .map_err(|e| LspDownloadError::Download(format!("latest.txt 取得失敗: {e}")))?;

    if !latest_resp.status().is_success() {
        return Err(LspDownloadError::Download(format!(
            "HTTP {}: {latest_txt_url}",
            latest_resp.status()
        )));
    }

    let filename = latest_resp
        .text()
        .await
        .map_err(|e| LspDownloadError::Download(format!("latest.txt 読み取り失敗: {e}")))?
        .trim()
        .to_string();

    let download_url =
        format!("https://download.eclipse.org/jdtls/milestones/{version}/{filename}");

    Ok((download_url, version.to_string()))
}

/// Extract a tar.gz archive into the given directory.
fn extract_tar_gz(data: &[u8], dest: &Path) -> Result<(), LspDownloadError> {
    use flate2::read::GzDecoder;
    let decoder = GzDecoder::new(data);
    let mut archive = tar::Archive::new(decoder);
    archive
        .unpack(dest)
        .map_err(|e| LspDownloadError::Install(format!("tar.gz 展開失敗: {e}")))?;
    Ok(())
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
        assert!(get_cached_server("java", dir.path()).is_none());
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
    fn get_cached_server_java_returns_config_when_binary_exists() {
        let dir = tempfile::tempdir().unwrap();
        let bin_dir = dir.path().join("jdtls").join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let launcher = if cfg!(windows) { "jdtls.bat" } else { "jdtls" };
        let bin = bin_dir.join(launcher);
        std::fs::write(&bin, b"fake").unwrap();

        let config = get_cached_server("java", dir.path()).unwrap();
        assert!(config.command.contains("jdtls"));
        assert!(config.args.is_empty());
        assert!(config.enabled);
    }

    #[test]
    fn extract_tar_gz_unpacks_archive() {
        let dir = tempfile::tempdir().unwrap();

        // Create a small tar.gz in memory
        let buf = Vec::new();
        let encoder = flate2::write::GzEncoder::new(buf, flate2::Compression::default());
        let mut tar_builder = tar::Builder::new(encoder);

        let content = b"hello world";
        let mut header = tar::Header::new_gnu();
        header.set_path("test.txt").unwrap();
        header.set_size(content.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        tar_builder.append(&header, &content[..]).unwrap();

        let encoder = tar_builder.into_inner().unwrap();
        let compressed = encoder.finish().unwrap();

        extract_tar_gz(&compressed, dir.path()).unwrap();

        let extracted = std::fs::read_to_string(dir.path().join("test.txt")).unwrap();
        assert_eq!(extracted, "hello world");
    }

    #[test]
    fn rust_analyzer_target_returns_valid_triple() {
        let target = rust_analyzer_target();
        assert!(!target.is_empty());
    }
}
