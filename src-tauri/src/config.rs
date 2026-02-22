use rand::distr::Alphanumeric;
use rand::RngExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::notion::types::NotionRepoConfig;

const TOKEN_LENGTH: usize = 48;

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleashConfig {
    #[serde(default = "default_true")]
    pub telemetry_enabled: bool,
    #[serde(default)]
    pub server: ServerSection,
    #[serde(default)]
    pub telemetry: TelemetrySection,
    #[serde(default)]
    pub notion: HashMap<String, NotionRepoConfig>,
    #[serde(default)]
    pub remote: RemoteSection,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RemoteSection {
    #[serde(default)]
    pub auto_start: bool,
    #[serde(default)]
    pub auto_start_on_lan: bool,
}

fn default_crash_reporting() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetrySection {
    #[serde(default = "default_crash_reporting")]
    pub crash_reporting: bool,
}

impl Default for TelemetrySection {
    fn default() -> Self {
        Self {
            crash_reporting: true,
        }
    }
}

impl Default for ReleashConfig {
    fn default() -> Self {
        Self {
            telemetry_enabled: true,
            server: ServerSection::default(),
            telemetry: TelemetrySection::default(),
            notion: HashMap::new(),
            remote: RemoteSection::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerSection {
    #[serde(default = "default_bind")]
    pub bind: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_hook_port")]
    pub hook_port: u16,
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub tls: TlsSection,
    #[serde(default)]
    pub notify: NotifySection,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DesktopNotifyMode {
    #[default]
    Always,
    WhenInactive,
}

fn default_inactive_timeout() -> u32 {
    2
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotifySection {
    #[serde(default)]
    pub webhook_url: String,
    #[serde(default)]
    pub on_running: bool,
    #[serde(default = "default_true")]
    pub on_done: bool,
    #[serde(default = "default_true")]
    pub on_error: bool,
    #[serde(default = "default_true")]
    pub on_waiting: bool,
    #[serde(default)]
    pub desktop_mode: DesktopNotifyMode,
    #[serde(default = "default_inactive_timeout")]
    pub inactive_timeout_minutes: u32,
}

impl Default for NotifySection {
    fn default() -> Self {
        Self {
            webhook_url: String::new(),
            on_running: false,
            on_done: true,
            on_error: true,
            on_waiting: true,
            desktop_mode: DesktopNotifyMode::default(),
            inactive_timeout_minutes: default_inactive_timeout(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TlsSection {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub cert: String,
    #[serde(default)]
    pub key: String,
}

fn default_bind() -> String {
    "127.0.0.1".to_string()
}

fn default_port() -> u16 {
    9700
}

fn default_hook_port() -> u16 {
    19700
}

impl Default for ServerSection {
    fn default() -> Self {
        Self {
            bind: default_bind(),
            port: default_port(),
            hook_port: default_hook_port(),
            token: String::new(),
            tls: TlsSection::default(),
            notify: NotifySection::default(),
        }
    }
}

pub struct AppConfig {
    config: Mutex<ReleashConfig>,
    config_path: PathBuf,
}

impl AppConfig {
    pub fn new(config: ReleashConfig, config_path: PathBuf) -> Self {
        Self {
            config: Mutex::new(config),
            config_path,
        }
    }

    pub fn get_config(&self) -> Result<ReleashConfig, String> {
        let config = self
            .config
            .lock()
            .map_err(|e| format!("ロック取得失敗: {e}"))?;
        Ok(config.clone())
    }

    pub fn with_config_mut<F, R>(&self, f: F) -> Result<R, String>
    where
        F: FnOnce(&mut ReleashConfig) -> Result<R, String>,
    {
        let mut config = self
            .config
            .lock()
            .map_err(|e| format!("ロック取得失敗: {e}"))?;
        let result = f(&mut config)?;
        write_config(&self.config_path, &config)?;
        Ok(result)
    }
}

pub fn generate_token() -> String {
    rand::rng()
        .sample_iter(&Alphanumeric)
        .take(TOKEN_LENGTH)
        .map(char::from)
        .collect()
}

pub fn load_or_create_config(path: &Path) -> Result<ReleashConfig, String> {
    let mut config = if path.exists() {
        let content =
            fs::read_to_string(path).map_err(|e| format!("設定ファイル読み込み失敗: {e}"))?;
        toml::from_str::<ReleashConfig>(&content)
            .map_err(|e| format!("設定ファイルのパース失敗: {e}"))?
    } else {
        ReleashConfig::default()
    };

    if config.server.token.is_empty() {
        config.server.token = generate_token();
        write_config(path, &config)?;
    }

    Ok(config)
}

pub fn write_config(path: &Path, config: &ReleashConfig) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("ディレクトリ作成失敗: {e}"))?;
    }

    let content =
        toml::to_string_pretty(config).map_err(|e| format!("設定のシリアライズ失敗: {e}"))?;

    let tmp_path = path.with_extension("toml.tmp");
    fs::write(&tmp_path, &content).map_err(|e| format!("一時ファイル書き込み失敗: {e}"))?;
    fs::rename(&tmp_path, path).map_err(|e| format!("ファイルのリネーム失敗: {e}"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("パーミッション設定失敗: {e}"))?;
    }

    Ok(())
}

#[tauri::command]
pub async fn update_telemetry_enabled(
    state: tauri::State<'_, Arc<AppConfig>>,
    enabled: bool,
) -> Result<(), String> {
    let app_config = state.inner().clone();
    tokio::task::spawn_blocking(move || {
        let mut config = app_config
            .config
            .lock()
            .map_err(|e| format!("ロック取得失敗: {e}"))?;
        config.telemetry_enabled = enabled;
        write_config(&app_config.config_path, &config)?;
        Ok(())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub fn get_server_config(state: tauri::State<'_, Arc<AppConfig>>) -> Result<ServerSection, String> {
    let config = state
        .config
        .lock()
        .map_err(|e| format!("ロック取得失敗: {e}"))?;
    Ok(config.server.clone())
}

#[tauri::command]
pub async fn update_server_port(
    state: tauri::State<'_, Arc<AppConfig>>,
    port: u16,
) -> Result<(), String> {
    let app_config = state.inner().clone();
    tokio::task::spawn_blocking(move || {
        let mut config = app_config
            .config
            .lock()
            .map_err(|e| format!("ロック取得失敗: {e}"))?;
        config.server.port = port;
        write_config(&app_config.config_path, &config)?;
        Ok(())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn regenerate_token(state: tauri::State<'_, Arc<AppConfig>>) -> Result<String, String> {
    let app_config = state.inner().clone();
    tokio::task::spawn_blocking(move || {
        let mut config = app_config
            .config
            .lock()
            .map_err(|e| format!("ロック取得失敗: {e}"))?;
        config.server.token = generate_token();
        write_config(&app_config.config_path, &config)?;
        Ok(config.server.token.clone())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub fn generate_hooks_config(state: tauri::State<'_, Arc<AppConfig>>) -> Result<String, String> {
    let config = state.get_config()?;
    let port = config.server.hook_port;
    let token = config.server.token;

    let hooks_json = serde_json::json!({
        "hooks": {
            "UserPromptSubmit": [{
                "matcher": "",
                "hooks": [{
                    "type": "command",
                    "command": format!(
                        "printf '{{\"worktree_path\":\"%s\",\"event\":\"%s\"}}' \"$(git rev-parse --show-toplevel 2>/dev/null || pwd)\" \"prompt_submit\" | curl -s -X POST http://localhost:{port}/hooks/agent -H 'Authorization: Bearer {token}' -H 'Content-Type: application/json' -d @- || true"
                    )
                }]
            }],
            "Stop": [{
                "matcher": "",
                "hooks": [{
                    "type": "command",
                    "command": format!(
                        "printf '{{\"worktree_path\":\"%s\",\"event\":\"%s\"}}' \"$(git rev-parse --show-toplevel 2>/dev/null || pwd)\" \"stop\" | curl -s -X POST http://localhost:{port}/hooks/agent -H 'Authorization: Bearer {token}' -H 'Content-Type: application/json' -d @- || true"
                    )
                }]
            }],
            "Notification": [
                {
                    "matcher": "permission_prompt",
                    "hooks": [{
                        "type": "command",
                        "command": format!(
                            "printf '{{\"worktree_path\":\"%s\",\"event\":\"%s\"}}' \"$(git rev-parse --show-toplevel 2>/dev/null || pwd)\" \"notification\" | curl -s -X POST http://localhost:{port}/hooks/agent -H 'Authorization: Bearer {token}' -H 'Content-Type: application/json' -d @- || true"
                        )
                    }]
                },
                {
                    "matcher": "elicitation_dialog",
                    "hooks": [{
                        "type": "command",
                        "command": format!(
                            "printf '{{\"worktree_path\":\"%s\",\"event\":\"%s\"}}' \"$(git rev-parse --show-toplevel 2>/dev/null || pwd)\" \"notification\" | curl -s -X POST http://localhost:{port}/hooks/agent -H 'Authorization: Bearer {token}' -H 'Content-Type: application/json' -d @- || true"
                        )
                    }]
                }
            ],
            "PostToolUse": [{
                "matcher": "",
                "hooks": [{
                    "type": "command",
                    "command": format!(
                        "printf '{{\"worktree_path\":\"%s\",\"event\":\"%s\"}}' \"$(git rev-parse --show-toplevel 2>/dev/null || pwd)\" \"post_tool_use\" | curl -s -X POST http://localhost:{port}/hooks/agent -H 'Authorization: Bearer {token}' -H 'Content-Type: application/json' -d @- || true"
                    )
                }]
            }],
            "PostToolUseFailure": [{
                "matcher": "",
                "hooks": [{
                    "type": "command",
                    "command": format!(
                        "printf '{{\"worktree_path\":\"%s\",\"event\":\"%s\"}}' \"$(git rev-parse --show-toplevel 2>/dev/null || pwd)\" \"post_tool_use_failure\" | curl -s -X POST http://localhost:{port}/hooks/agent -H 'Authorization: Bearer {token}' -H 'Content-Type: application/json' -d @- || true"
                    )
                }]
            }],
            "SessionStart": [{
                "matcher": "",
                "hooks": [{
                    "type": "command",
                    "command": format!(
                        "printf '{{\"worktree_path\":\"%s\",\"event\":\"%s\"}}' \"$(git rev-parse --show-toplevel 2>/dev/null || pwd)\" \"session_start\" | curl -s -X POST http://localhost:{port}/hooks/agent -H 'Authorization: Bearer {token}' -H 'Content-Type: application/json' -d @- || true"
                    )
                }]
            }]
        }
    });

    serde_json::to_string_pretty(&hooks_json).map_err(|e| format!("JSON生成失敗: {e}"))
}

fn is_releash_hook_entry(entry: &serde_json::Value) -> bool {
    entry
        .get("hooks")
        .and_then(|h| h.as_array())
        .is_some_and(|hooks| {
            hooks.iter().any(|hook| {
                hook.get("command")
                    .and_then(|c| c.as_str())
                    .is_some_and(|cmd| cmd.contains("/hooks/agent"))
            })
        })
}

fn merge_hooks(
    existing: &mut serde_json::Value,
    new_config: &serde_json::Value,
) -> Result<(), String> {
    if let Some(serde_json::Value::Object(new_hooks)) = new_config.get("hooks") {
        let existing_hooks = existing
            .as_object_mut()
            .ok_or("settings.jsonがオブジェクトではありません")?
            .entry("hooks")
            .or_insert_with(|| serde_json::json!({}));
        if let serde_json::Value::Object(map) = existing_hooks {
            for (key, new_entries) in new_hooks {
                let existing_entries = map
                    .entry(key.clone())
                    .or_insert_with(|| serde_json::json!([]));

                if let (Some(existing_arr), Some(new_arr)) =
                    (existing_entries.as_array_mut(), new_entries.as_array())
                {
                    for new_entry in new_arr {
                        let new_matcher = new_entry
                            .get("matcher")
                            .and_then(|m| m.as_str())
                            .unwrap_or("");

                        let existing_idx = existing_arr.iter().position(|e| {
                            let matcher_matches =
                                e.get("matcher").and_then(|m| m.as_str()).unwrap_or("")
                                    == new_matcher;
                            matcher_matches && is_releash_hook_entry(e)
                        });

                        match existing_idx {
                            Some(idx) => existing_arr[idx] = new_entry.clone(),
                            None => existing_arr.push(new_entry.clone()),
                        }
                    }
                }
            }
        } else {
            *existing_hooks = serde_json::Value::Object(new_hooks.clone());
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn apply_hooks_config(config_json: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let home = dirs::home_dir().ok_or("ホームディレクトリの取得失敗")?;
        let settings_path = home.join(".claude").join("settings.json");

        let mut existing: serde_json::Value = if settings_path.exists() {
            let content = fs::read_to_string(&settings_path)
                .map_err(|e| format!("settings.json読み込み失敗: {e}"))?;
            serde_json::from_str(&content).map_err(|e| format!("settings.jsonパース失敗: {e}"))?
        } else {
            serde_json::json!({})
        };

        let new_config: serde_json::Value =
            serde_json::from_str(&config_json).map_err(|e| format!("設定JSONパース失敗: {e}"))?;

        merge_hooks(&mut existing, &new_config)?;

        if let Some(parent) = settings_path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("ディレクトリ作成失敗: {e}"))?;
        }
        let content = serde_json::to_string_pretty(&existing)
            .map_err(|e| format!("JSONシリアライズ失敗: {e}"))?;
        let tmp_path = settings_path.with_extension("json.tmp");
        fs::write(&tmp_path, &content).map_err(|e| format!("一時ファイル書き込み失敗: {e}"))?;
        fs::rename(&tmp_path, &settings_path)
            .map_err(|e| format!("ファイルのリネーム失敗: {e}"))?;

        Ok(())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn get_hooks_status(state: tauri::State<'_, Arc<AppConfig>>) -> Result<String, String> {
    let config = state.get_config()?;
    let hook_port = config.server.hook_port;
    let token = config.server.token.clone();

    tokio::task::spawn_blocking(move || {
        let home = dirs::home_dir().ok_or("ホームディレクトリの取得失敗")?;
        let settings_path = home.join(".claude").join("settings.json");

        if !settings_path.exists() {
            return Ok("not_configured".to_string());
        }

        let content = fs::read_to_string(&settings_path)
            .map_err(|e| format!("settings.json読み込み失敗: {e}"))?;
        let parsed: serde_json::Value =
            serde_json::from_str(&content).map_err(|e| format!("settings.jsonパース失敗: {e}"))?;

        let port_str = format!("localhost:{hook_port}");
        let hooks_str = parsed
            .get("hooks")
            .map(|h| h.to_string())
            .unwrap_or_default();

        if !hooks_str.contains(&port_str) {
            return Ok("not_configured".to_string());
        }

        if !hooks_str.contains(&token) {
            return Ok("token_mismatch".to_string());
        }

        Ok("active".to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub fn get_notify_config(state: tauri::State<'_, Arc<AppConfig>>) -> Result<NotifySection, String> {
    let config = state
        .config
        .lock()
        .map_err(|e| format!("ロック取得失敗: {e}"))?;
    Ok(config.server.notify.clone())
}

#[tauri::command]
pub async fn update_notify_config(
    state: tauri::State<'_, Arc<AppConfig>>,
    notify: NotifySection,
) -> Result<(), String> {
    let app_config = state.inner().clone();
    tokio::task::spawn_blocking(move || {
        let mut config = app_config
            .config
            .lock()
            .map_err(|e| format!("ロック取得失敗: {e}"))?;
        config.server.notify = notify;
        write_config(&app_config.config_path, &config)?;
        Ok(())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub fn get_remote_config(state: tauri::State<'_, Arc<AppConfig>>) -> Result<RemoteSection, String> {
    let config = state
        .config
        .lock()
        .map_err(|e| format!("ロック取得失敗: {e}"))?;
    Ok(config.remote.clone())
}

#[tauri::command]
pub async fn update_remote_config(
    state: tauri::State<'_, Arc<AppConfig>>,
    remote: RemoteSection,
) -> Result<(), String> {
    let app_config = state.inner().clone();
    tokio::task::spawn_blocking(move || {
        let mut config = app_config
            .config
            .lock()
            .map_err(|e| format!("ロック取得失敗: {e}"))?;
        config.remote = remote;
        write_config(&app_config.config_path, &config)?;
        Ok(())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub fn get_crash_reporting_enabled(
    state: tauri::State<'_, Arc<AppConfig>>,
) -> Result<bool, String> {
    let config = state
        .config
        .lock()
        .map_err(|e| format!("ロック取得失敗: {e}"))?;
    Ok(config.telemetry.crash_reporting)
}

#[tauri::command]
pub async fn update_webhook_url(
    state: tauri::State<'_, Arc<AppConfig>>,
    url: String,
) -> Result<(), String> {
    let app_config = state.inner().clone();
    tokio::task::spawn_blocking(move || {
        let mut config = app_config
            .config
            .lock()
            .map_err(|e| format!("ロック取得失敗: {e}"))?;
        config.server.notify.webhook_url = url;
        write_config(&app_config.config_path, &config)?;
        Ok(())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn update_crash_reporting(
    state: tauri::State<'_, Arc<AppConfig>>,
    enabled: bool,
) -> Result<(), String> {
    let app_config = state.inner().clone();
    tokio::task::spawn_blocking(move || {
        let mut config = app_config
            .config
            .lock()
            .map_err(|e| format!("ロック取得失敗: {e}"))?;
        config.telemetry.crash_reporting = enabled;
        write_config(&app_config.config_path, &config)?;
        crate::sentry_integration::set_crash_reporting_enabled(enabled);
        Ok(())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn config_path(dir: &TempDir) -> PathBuf {
        dir.path().join("releash.toml")
    }

    #[test]
    fn creates_default_config_with_token() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let config = load_or_create_config(&path).unwrap();

        assert_eq!(config.server.bind, "127.0.0.1");
        assert_eq!(config.server.port, 9700);
        assert_eq!(config.server.token.len(), TOKEN_LENGTH);
        assert!(!config.server.tls.enabled);
        assert!(config.telemetry_enabled);
        assert!(path.exists());
    }

    #[test]
    fn loads_existing_config() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let content = r#"
[server]
bind = "0.0.0.0"
port = 8080
token = "existing_token_value_here_with_enough_length_!!"
"#;
        fs::write(&path, content).unwrap();

        let config = load_or_create_config(&path).unwrap();

        assert_eq!(config.server.bind, "0.0.0.0");
        assert_eq!(config.server.port, 8080);
        assert_eq!(
            config.server.token,
            "existing_token_value_here_with_enough_length_!!"
        );
    }

    #[test]
    fn generates_token_when_empty_and_writes_back() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let content = r#"
[server]
bind = "0.0.0.0"
port = 9700
token = ""
"#;
        fs::write(&path, content).unwrap();

        let config = load_or_create_config(&path).unwrap();

        assert_eq!(config.server.token.len(), TOKEN_LENGTH);

        let reloaded = fs::read_to_string(&path).unwrap();
        let reloaded: ReleashConfig = toml::from_str(&reloaded).unwrap();
        assert_eq!(reloaded.server.token, config.server.token);
    }

    #[test]
    fn fills_defaults_for_partial_config() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let content = "[server]\nport = 3000\n";
        fs::write(&path, content).unwrap();

        let config = load_or_create_config(&path).unwrap();

        assert_eq!(config.server.bind, "127.0.0.1");
        assert_eq!(config.server.port, 3000);
        assert_eq!(config.server.token.len(), TOKEN_LENGTH);
        assert!(!config.server.tls.enabled);
    }

    #[test]
    fn roundtrip_serialize_deserialize() {
        let mut config = ReleashConfig::default();
        config.server.token = generate_token();
        config.server.bind = "192.168.1.1".to_string();
        config.server.port = 5555;
        config.server.tls.enabled = true;
        config.server.tls.cert = "/path/to/cert.pem".to_string();
        config.server.tls.key = "/path/to/key.pem".to_string();

        let serialized = toml::to_string_pretty(&config).unwrap();
        let deserialized: ReleashConfig = toml::from_str(&serialized).unwrap();

        assert_eq!(deserialized.server.bind, config.server.bind);
        assert_eq!(deserialized.server.port, config.server.port);
        assert_eq!(deserialized.server.token, config.server.token);
        assert_eq!(deserialized.server.tls.enabled, config.server.tls.enabled);
        assert_eq!(deserialized.server.tls.cert, config.server.tls.cert);
        assert_eq!(deserialized.server.tls.key, config.server.tls.key);
    }

    #[test]
    fn generated_tokens_are_unique_and_correct_length() {
        let t1 = generate_token();
        let t2 = generate_token();

        assert_ne!(t1, t2);
        assert_eq!(t1.len(), TOKEN_LENGTH);
        assert_eq!(t2.len(), TOKEN_LENGTH);
        assert!(t1.chars().all(|c| c.is_ascii_alphanumeric()));
        assert!(t2.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn telemetry_enabled_defaults_to_true() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let content = "[server]\nport = 9700\n";
        fs::write(&path, content).unwrap();

        let config = load_or_create_config(&path).unwrap();
        assert!(config.telemetry_enabled);
    }

    #[test]
    fn telemetry_disabled_persists() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let content = "telemetry_enabled = false\n\n[server]\nport = 9700\n";
        fs::write(&path, content).unwrap();

        let config = load_or_create_config(&path).unwrap();
        assert!(!config.telemetry_enabled);
    }

    #[test]
    fn atomic_write_leaves_no_tmp_file() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let config = ReleashConfig::default();
        write_config(&path, &config).unwrap();

        assert!(path.exists());
        assert!(!path.with_extension("toml.tmp").exists());
    }

    fn releash_hook_entry(matcher: &str, port: u16) -> serde_json::Value {
        serde_json::json!({
            "matcher": matcher,
            "hooks": [{
                "type": "command",
                "command": format!(
                    "curl -s -X POST http://localhost:{port}/hooks/agent -H 'Content-Type: application/json' -d '{{}}' || true"
                )
            }]
        })
    }

    fn user_hook_entry(matcher: &str, command: &str) -> serde_json::Value {
        serde_json::json!({
            "matcher": matcher,
            "hooks": [{
                "type": "command",
                "command": command
            }]
        })
    }

    #[test]
    fn is_releash_hook_entry_identifies_releash_hooks() {
        let releash = releash_hook_entry("", 19700);
        let user = user_hook_entry("", "echo hello");

        assert!(is_releash_hook_entry(&releash));
        assert!(!is_releash_hook_entry(&user));
    }

    #[test]
    fn merge_hooks_preserves_user_hooks() {
        let user_entry =
            user_hook_entry("permission_prompt", "notify-send 'Claude needs permission'");
        let mut existing = serde_json::json!({
            "hooks": {
                "Notification": [user_entry.clone()]
            }
        });

        let new_config = serde_json::json!({
            "hooks": {
                "Notification": [
                    releash_hook_entry("permission_prompt", 19700),
                    releash_hook_entry("elicitation_dialog", 19700),
                ]
            }
        });

        merge_hooks(&mut existing, &new_config).unwrap();

        let entries = existing["hooks"]["Notification"].as_array().unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0], user_entry);
        assert!(is_releash_hook_entry(&entries[1]));
        assert!(is_releash_hook_entry(&entries[2]));
    }

    #[test]
    fn merge_hooks_updates_existing_releash_hooks() {
        let user_entry = user_hook_entry("", "echo hello");
        let old_releash = releash_hook_entry("", 19700);
        let mut existing = serde_json::json!({
            "hooks": {
                "Stop": [user_entry.clone(), old_releash]
            }
        });

        let new_releash = releash_hook_entry("", 29700);
        let new_config = serde_json::json!({
            "hooks": {
                "Stop": [new_releash.clone()]
            }
        });

        merge_hooks(&mut existing, &new_config).unwrap();

        let entries = existing["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0], user_entry);
        assert_eq!(entries[1], new_releash);
    }

    #[test]
    fn merge_hooks_adds_new_event_key() {
        let mut existing = serde_json::json!({
            "hooks": {
                "Notification": [user_hook_entry("", "echo notify")]
            }
        });

        let new_config = serde_json::json!({
            "hooks": {
                "SessionStart": [releash_hook_entry("", 19700)]
            }
        });

        merge_hooks(&mut existing, &new_config).unwrap();

        assert!(existing["hooks"]["Notification"].as_array().unwrap().len() == 1);
        assert!(existing["hooks"]["SessionStart"].as_array().unwrap().len() == 1);
    }

    #[test]
    fn merge_hooks_creates_hooks_key_when_absent() {
        let mut existing = serde_json::json!({});

        let new_config = serde_json::json!({
            "hooks": {
                "Stop": [releash_hook_entry("", 19700)]
            }
        });

        merge_hooks(&mut existing, &new_config).unwrap();

        let entries = existing["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert!(is_releash_hook_entry(&entries[0]));
    }

    #[test]
    fn merge_hooks_preserves_non_hooks_settings() {
        let mut existing = serde_json::json!({
            "permissions": { "allow": ["Read"] },
            "hooks": {}
        });

        let new_config = serde_json::json!({
            "hooks": {
                "Stop": [releash_hook_entry("", 19700)]
            }
        });

        merge_hooks(&mut existing, &new_config).unwrap();

        assert_eq!(existing["permissions"]["allow"][0], "Read");
    }

    #[test]
    fn telemetry_defaults_to_enabled() {
        let config = ReleashConfig::default();
        assert!(config.telemetry.crash_reporting);
    }

    #[test]
    fn telemetry_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let mut config = ReleashConfig::default();
        config.server.token = generate_token();
        config.telemetry.crash_reporting = false;
        write_config(&path, &config).unwrap();

        let reloaded = fs::read_to_string(&path).unwrap();
        let reloaded: ReleashConfig = toml::from_str(&reloaded).unwrap();
        assert!(!reloaded.telemetry.crash_reporting);
    }

    #[test]
    fn existing_config_without_telemetry_gets_defaults() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let content = r#"
[server]
bind = "127.0.0.1"
port = 9700
token = "existing_token_value_here_with_enough_length_!!"
"#;
        fs::write(&path, content).unwrap();

        let config = load_or_create_config(&path).unwrap();
        assert!(config.telemetry.crash_reporting);
    }

    #[test]
    fn notify_section_defaults() {
        let notify = NotifySection::default();
        assert!(notify.webhook_url.is_empty());
        assert!(!notify.on_running);
        assert!(notify.on_done);
        assert!(notify.on_error);
        assert!(notify.on_waiting);
        assert_eq!(notify.desktop_mode, DesktopNotifyMode::Always);
        assert_eq!(notify.inactive_timeout_minutes, 2);
    }

    #[test]
    fn notify_section_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let mut config = ReleashConfig::default();
        config.server.token = generate_token();
        config.server.notify.webhook_url = "https://hooks.slack.com/test".to_string();
        config.server.notify.on_running = true;
        config.server.notify.on_done = false;
        config.server.notify.desktop_mode = DesktopNotifyMode::WhenInactive;
        config.server.notify.inactive_timeout_minutes = 5;
        write_config(&path, &config).unwrap();

        let reloaded = fs::read_to_string(&path).unwrap();
        let reloaded: ReleashConfig = toml::from_str(&reloaded).unwrap();
        assert_eq!(
            reloaded.server.notify.webhook_url,
            "https://hooks.slack.com/test"
        );
        assert!(reloaded.server.notify.on_running);
        assert!(!reloaded.server.notify.on_done);
        assert_eq!(
            reloaded.server.notify.desktop_mode,
            DesktopNotifyMode::WhenInactive
        );
        assert_eq!(reloaded.server.notify.inactive_timeout_minutes, 5);
    }

    #[test]
    fn existing_config_without_notify_fields_gets_defaults() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let content = r#"
[server]
bind = "127.0.0.1"
port = 9700
token = "existing_token_value_here_with_enough_length_!!"

[server.notify]
webhook_url = "https://hooks.slack.com/old"
"#;
        fs::write(&path, content).unwrap();

        let config = load_or_create_config(&path).unwrap();
        assert_eq!(
            config.server.notify.webhook_url,
            "https://hooks.slack.com/old"
        );
        assert!(!config.server.notify.on_running);
        assert!(config.server.notify.on_done);
        assert!(config.server.notify.on_error);
        assert!(config.server.notify.on_waiting);
        assert_eq!(config.server.notify.desktop_mode, DesktopNotifyMode::Always);
        assert_eq!(config.server.notify.inactive_timeout_minutes, 2);
    }

    #[test]
    fn desktop_mode_serializes_snake_case() {
        let always = serde_json::to_string(&DesktopNotifyMode::Always).unwrap();
        assert_eq!(always, r#""always""#);

        let when_inactive = serde_json::to_string(&DesktopNotifyMode::WhenInactive).unwrap();
        assert_eq!(when_inactive, r#""when_inactive""#);
    }

    #[test]
    fn remote_section_defaults() {
        let remote = RemoteSection::default();
        assert!(!remote.auto_start);
        assert!(!remote.auto_start_on_lan);
    }

    #[test]
    fn remote_section_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let mut config = ReleashConfig::default();
        config.server.token = generate_token();
        config.remote.auto_start = true;
        config.remote.auto_start_on_lan = true;
        write_config(&path, &config).unwrap();

        let reloaded = fs::read_to_string(&path).unwrap();
        let reloaded: ReleashConfig = toml::from_str(&reloaded).unwrap();
        assert!(reloaded.remote.auto_start);
        assert!(reloaded.remote.auto_start_on_lan);
    }

    #[test]
    fn existing_config_without_remote_gets_defaults() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let content = r#"
[server]
bind = "127.0.0.1"
port = 9700
token = "existing_token_value_here_with_enough_length_!!"
"#;
        fs::write(&path, content).unwrap();

        let config = load_or_create_config(&path).unwrap();
        assert!(!config.remote.auto_start);
        assert!(!config.remote.auto_start_on_lan);
    }

    #[test]
    fn existing_config_without_notion_gets_empty_default() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let content = r#"
[server]
bind = "127.0.0.1"
port = 9700
token = "existing_token_value_here_with_enough_length_!!"
"#;
        fs::write(&path, content).unwrap();

        let config = load_or_create_config(&path).unwrap();
        assert!(config.notion.is_empty());
    }

    #[test]
    fn notion_config_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let mut config = ReleashConfig::default();
        config.server.token = generate_token();
        config.notion.insert(
            "/path/to/repo".to_string(),
            NotionRepoConfig {
                api_token: "ntn_test_token".to_string(),
                database_id: "db-id-456".to_string(),
                property_mapping: crate::notion::types::PropertyMapping::default(),
            },
        );
        write_config(&path, &config).unwrap();

        let reloaded = fs::read_to_string(&path).unwrap();
        let reloaded: ReleashConfig = toml::from_str(&reloaded).unwrap();
        assert_eq!(reloaded.notion.len(), 1);
        let repo_config = reloaded.notion.get("/path/to/repo").unwrap();
        assert_eq!(repo_config.api_token, "ntn_test_token");
        assert_eq!(repo_config.database_id, "db-id-456");
        assert_eq!(repo_config.property_mapping.title, "Name");
    }
}
