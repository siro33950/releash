use rand::distributions::Alphanumeric;
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const TOKEN_LENGTH: usize = 48;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReleashConfig {
    #[serde(default)]
    pub server: ServerSection,
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NotifySection {
    #[serde(default)]
    pub webhook_url: String,
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
}

pub fn generate_token() -> String {
    rand::thread_rng()
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
                        "printf '{{\"worktree_path\":\"%s\",\"event\":\"%s\"}}' \"$(pwd)\" \"prompt_submit\" | curl -s -X POST http://localhost:{port}/hooks/agent -H 'Authorization: Bearer {token}' -H 'Content-Type: application/json' -d @- || true"
                    )
                }]
            }],
            "Stop": [{
                "matcher": "",
                "hooks": [{
                    "type": "command",
                    "command": format!(
                        "printf '{{\"worktree_path\":\"%s\",\"event\":\"%s\"}}' \"$(pwd)\" \"stop\" | curl -s -X POST http://localhost:{port}/hooks/agent -H 'Authorization: Bearer {token}' -H 'Content-Type: application/json' -d @- || true"
                    )
                }]
            }],
            "Notification": [
                {
                    "matcher": "permission_prompt",
                    "hooks": [{
                        "type": "command",
                        "command": format!(
                            "printf '{{\"worktree_path\":\"%s\",\"event\":\"%s\"}}' \"$(pwd)\" \"notification\" | curl -s -X POST http://localhost:{port}/hooks/agent -H 'Authorization: Bearer {token}' -H 'Content-Type: application/json' -d @- || true"
                        )
                    }]
                },
                {
                    "matcher": "elicitation_dialog",
                    "hooks": [{
                        "type": "command",
                        "command": format!(
                            "printf '{{\"worktree_path\":\"%s\",\"event\":\"%s\"}}' \"$(pwd)\" \"notification\" | curl -s -X POST http://localhost:{port}/hooks/agent -H 'Authorization: Bearer {token}' -H 'Content-Type: application/json' -d @- || true"
                        )
                    }]
                }
            ],
            "PostToolUse": [{
                "matcher": "",
                "hooks": [{
                    "type": "command",
                    "command": format!(
                        "printf '{{\"worktree_path\":\"%s\",\"event\":\"%s\"}}' \"$(pwd)\" \"post_tool_use\" | curl -s -X POST http://localhost:{port}/hooks/agent -H 'Authorization: Bearer {token}' -H 'Content-Type: application/json' -d @- || true"
                    )
                }]
            }]
        }
    });

    serde_json::to_string_pretty(&hooks_json).map_err(|e| format!("JSON生成失敗: {e}"))
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

        if let Some(serde_json::Value::Object(new_hooks)) = new_config.get("hooks") {
            let existing_hooks = existing
                .as_object_mut()
                .ok_or("settings.jsonがオブジェクトではありません")?
                .entry("hooks")
                .or_insert_with(|| serde_json::json!({}));
            if let serde_json::Value::Object(map) = existing_hooks {
                for (key, value) in new_hooks {
                    map.insert(key.clone(), value.clone());
                }
            } else {
                *existing_hooks = serde_json::Value::Object(new_hooks.clone());
            }
        }

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
    fn atomic_write_leaves_no_tmp_file() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let config = ReleashConfig::default();
        write_config(&path, &config).unwrap();

        assert!(path.exists());
        assert!(!path.with_extension("toml.tmp").exists());
    }
}
