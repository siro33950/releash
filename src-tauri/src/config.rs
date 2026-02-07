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
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub tls: TlsSection,
    #[serde(default)]
    pub notify: NotifySection,
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NotifySection {
    #[serde(default)]
    pub webhook_url: String,
}

fn default_bind() -> String {
    "127.0.0.1".to_string()
}

fn default_port() -> u16 {
    9700
}

impl Default for ServerSection {
    fn default() -> Self {
        Self {
            bind: default_bind(),
            port: default_port(),
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
pub fn update_server_port(
    state: tauri::State<'_, Arc<AppConfig>>,
    port: u16,
) -> Result<(), String> {
    let mut config = state
        .config
        .lock()
        .map_err(|e| format!("ロック取得失敗: {e}"))?;
    config.server.port = port;
    write_config(&state.config_path, &config)?;
    Ok(())
}

#[tauri::command]
pub fn regenerate_token(state: tauri::State<'_, Arc<AppConfig>>) -> Result<String, String> {
    let mut config = state
        .config
        .lock()
        .map_err(|e| format!("ロック取得失敗: {e}"))?;
    config.server.token = generate_token();
    write_config(&state.config_path, &config)?;
    Ok(config.server.token.clone())
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
