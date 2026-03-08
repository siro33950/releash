use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use super::download;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspServerConfig {
    pub command: String,
    pub args: Vec<String>,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspServerEntry {
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

/// Default language server mappings.
/// Maps language ID → (command, args, file extensions).
struct DefaultServer {
    command: &'static str,
    args: &'static [&'static str],
    extensions: &'static [&'static str],
}

static DEFAULT_SERVERS: &[(&str, DefaultServer)] = &[
    (
        "typescript",
        DefaultServer {
            command: "typescript-language-server",
            args: &["--stdio"],
            extensions: &["ts", "tsx", "js", "jsx", "mts", "cts", "mjs", "cjs"],
        },
    ),
    (
        "rust",
        DefaultServer {
            command: "rust-analyzer",
            args: &[],
            extensions: &["rs"],
        },
    ),
    (
        "go",
        DefaultServer {
            command: "gopls",
            args: &[],
            extensions: &["go"],
        },
    ),
    (
        "java",
        DefaultServer {
            command: "jdtls",
            args: &[],
            extensions: &["java"],
        },
    ),
];

/// Cache of detected servers (command → is_available).
static WHICH_CACHE: OnceLock<std::sync::Mutex<HashMap<String, bool>>> = OnceLock::new();

fn which_cache() -> &'static std::sync::Mutex<HashMap<String, bool>> {
    WHICH_CACHE.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

/// Check if a command is available on PATH.
pub fn is_command_available(command: &str) -> bool {
    let mut cache = which_cache().lock().unwrap();
    if let Some(&available) = cache.get(command) {
        return available;
    }

    #[cfg(unix)]
    let check = "which";
    #[cfg(windows)]
    let check = "where";

    let available = std::process::Command::new(check)
        .arg(command)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    cache.insert(command.to_string(), available);
    available
}

/// Detect a language server for the given language.
///
/// Detection priority:
/// 1. User config (releash.toml `[lsp]`)
/// 2. Cached binary in `lsp_cache_dir`
/// 3. Workspace `node_modules/.bin/` (TypeScript only)
/// 4. System PATH
pub fn detect_server(
    language: &str,
    user_config: &HashMap<String, LspServerEntry>,
    lsp_cache_dir: Option<&Path>,
    worktree_path: Option<&str>,
) -> Option<LspServerConfig> {
    // 1. Check user config first
    if let Some(entry) = user_config.get(language) {
        if !entry.enabled {
            return Some(LspServerConfig {
                command: String::new(),
                args: vec![],
                enabled: false,
            });
        }
        if !entry.command.is_empty() {
            return Some(LspServerConfig {
                command: entry.command.clone(),
                args: entry.args.clone(),
                enabled: true,
            });
        }
    }

    // 2. Check cached binary
    if let Some(cache_dir) = lsp_cache_dir {
        if let Some(config) = download::get_cached_server(language, cache_dir) {
            return Some(config);
        }
    }

    // 3. Check workspace node_modules (TypeScript only)
    if language == "typescript" {
        if let Some(wt) = worktree_path {
            let bin = std::path::Path::new(wt)
                .join("node_modules")
                .join(".bin")
                .join("typescript-language-server");
            if bin.exists() {
                return Some(LspServerConfig {
                    command: bin.to_string_lossy().to_string(),
                    args: vec!["--stdio".to_string()],
                    enabled: true,
                });
            }
        }
    }

    // 4. Fall back to system PATH
    for &(lang, ref default) in DEFAULT_SERVERS {
        if lang == language && is_command_available(default.command) {
            return Some(LspServerConfig {
                command: default.command.to_string(),
                args: default.args.iter().map(|s| s.to_string()).collect(),
                enabled: true,
            });
        }
    }

    None
}

/// Get the language ID for a file extension.
pub fn language_for_extension(ext: &str) -> Option<&'static str> {
    for &(lang, ref default) in DEFAULT_SERVERS {
        if default.extensions.contains(&ext) {
            return Some(lang);
        }
    }
    None
}

/// Get the LSP protocol languageId for a file extension.
/// Unlike `language_for_extension` (which returns the server key),
/// this returns the correct languageId per LSP specification.
pub fn lsp_language_id(ext: &str) -> Option<&'static str> {
    match ext {
        "tsx" => Some("typescriptreact"),
        "jsx" => Some("javascriptreact"),
        "ts" | "mts" | "cts" => Some("typescript"),
        "js" | "mjs" | "cjs" => Some("javascript"),
        _ => language_for_extension(ext),
    }
}

/// List all default supported languages.
pub fn supported_languages() -> Vec<&'static str> {
    DEFAULT_SERVERS.iter().map(|&(lang, _)| lang).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_for_extension_typescript() {
        assert_eq!(language_for_extension("ts"), Some("typescript"));
        assert_eq!(language_for_extension("tsx"), Some("typescript"));
        assert_eq!(language_for_extension("js"), Some("typescript"));
        assert_eq!(language_for_extension("jsx"), Some("typescript"));
        assert_eq!(language_for_extension("mjs"), Some("typescript"));
        assert_eq!(language_for_extension("cjs"), Some("typescript"));
    }

    #[test]
    fn language_for_extension_rust() {
        assert_eq!(language_for_extension("rs"), Some("rust"));
    }

    #[test]
    fn language_for_extension_go() {
        assert_eq!(language_for_extension("go"), Some("go"));
    }

    #[test]
    fn language_for_extension_java() {
        assert_eq!(language_for_extension("java"), Some("java"));
    }

    #[test]
    fn lsp_language_id_distinguishes_jsx() {
        assert_eq!(lsp_language_id("tsx"), Some("typescriptreact"));
        assert_eq!(lsp_language_id("jsx"), Some("javascriptreact"));
        assert_eq!(lsp_language_id("ts"), Some("typescript"));
        assert_eq!(lsp_language_id("js"), Some("javascript"));
        assert_eq!(lsp_language_id("mts"), Some("typescript"));
        assert_eq!(lsp_language_id("cts"), Some("typescript"));
        assert_eq!(lsp_language_id("mjs"), Some("javascript"));
        assert_eq!(lsp_language_id("cjs"), Some("javascript"));
        assert_eq!(lsp_language_id("rs"), Some("rust"));
        assert_eq!(lsp_language_id("go"), Some("go"));
        assert_eq!(lsp_language_id("java"), Some("java"));
        assert_eq!(lsp_language_id("xyz"), None);
    }

    #[test]
    fn language_for_extension_unknown() {
        assert_eq!(language_for_extension("xyz"), None);
    }

    #[test]
    fn supported_languages_includes_defaults() {
        let langs = supported_languages();
        assert!(langs.contains(&"typescript"));
        assert!(langs.contains(&"rust"));
        assert!(langs.contains(&"go"));
        assert!(langs.contains(&"java"));
    }

    #[test]
    fn detect_server_respects_disabled_config() {
        let mut config = HashMap::new();
        config.insert(
            "typescript".to_string(),
            LspServerEntry {
                command: String::new(),
                args: vec![],
                enabled: false,
            },
        );
        let result = detect_server("typescript", &config, None, None);
        assert!(result.is_some());
        let server = result.unwrap();
        assert!(!server.enabled);
        assert!(server.command.is_empty());
    }

    #[test]
    fn detect_server_disabled_returns_config_with_enabled_false() {
        let mut config = HashMap::new();
        config.insert(
            "java".to_string(),
            LspServerEntry {
                command: "custom-jdtls".to_string(),
                args: vec!["--stdio".to_string()],
                enabled: false,
            },
        );
        let result = detect_server("java", &config, None, None);
        assert!(result.is_some());
        let server = result.unwrap();
        assert!(!server.enabled);
        assert!(server.command.is_empty());
        assert!(server.args.is_empty());
    }

    #[test]
    fn detect_server_uses_custom_command() {
        let mut config = HashMap::new();
        config.insert(
            "typescript".to_string(),
            LspServerEntry {
                command: "my-custom-lsp".to_string(),
                args: vec!["--stdio".to_string()],
                enabled: true,
            },
        );
        let result = detect_server("typescript", &config, None, None);
        assert!(result.is_some());
        let server = result.unwrap();
        assert_eq!(server.command, "my-custom-lsp");
        assert_eq!(server.args, vec!["--stdio"]);
    }

    #[test]
    fn detect_server_uses_custom_command_java() {
        let mut config = HashMap::new();
        config.insert(
            "java".to_string(),
            LspServerEntry {
                command: "/opt/jdtls/bin/jdtls".to_string(),
                args: vec!["--stdio".to_string(), "-data".to_string(), "/tmp/jdtls-ws".to_string()],
                enabled: true,
            },
        );
        let result = detect_server("java", &config, None, None);
        assert!(result.is_some());
        let server = result.unwrap();
        assert!(server.enabled);
        assert_eq!(server.command, "/opt/jdtls/bin/jdtls");
        assert_eq!(server.args, vec!["--stdio", "-data", "/tmp/jdtls-ws"]);
    }

    #[test]
    fn detect_server_unknown_language_returns_none() {
        let config = HashMap::new();
        assert!(detect_server("brainfuck", &config, None, None).is_none());
    }

    #[test]
    fn detect_server_finds_cached_binary() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("rust-analyzer");
        std::fs::write(&bin, b"fake").unwrap();

        let config = HashMap::new();
        let result = detect_server("rust", &config, Some(dir.path()), None);
        assert!(result.is_some());
        let server = result.unwrap();
        assert!(server.command.contains("rust-analyzer"));
    }

    #[test]
    fn detect_server_finds_cached_jdtls_binary() {
        let dir = tempfile::tempdir().unwrap();
        let jdtls_dir = dir.path().join("jdtls").join("bin");
        std::fs::create_dir_all(&jdtls_dir).unwrap();
        let bin = jdtls_dir.join("jdtls");
        std::fs::write(&bin, b"fake").unwrap();

        let config = HashMap::new();
        let result = detect_server("java", &config, Some(dir.path()), None);
        assert!(result.is_some());
        let server = result.unwrap();
        assert!(server.command.contains("jdtls"));
    }

    #[test]
    fn detect_server_finds_workspace_node_modules() {
        let dir = tempfile::tempdir().unwrap();
        let bin_dir = dir.path().join("node_modules").join(".bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let bin = bin_dir.join("typescript-language-server");
        std::fs::write(&bin, b"fake").unwrap();

        let config = HashMap::new();
        let result = detect_server(
            "typescript",
            &config,
            None,
            Some(dir.path().to_str().unwrap()),
        );
        assert!(result.is_some());
        let server = result.unwrap();
        assert!(server.command.contains("typescript-language-server"));
        assert_eq!(server.args, vec!["--stdio"]);
    }
}
