use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use crate::domain::app_config::error::AppConfigError;
use crate::domain::app_config::repository::{
    AgentConfigRepository, ConfigRepository, ConfigSecretRepository, ConfigUpdate,
    NotionConfigRepository,
};
use crate::domain::app_config::services::generate_token;
use crate::domain::app_config::value_objects as domain_vo;

use super::config_models::{apply_domain_to_config, config_to_domain, ReleashConfig};

static CONFIG_WRITE_TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

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

impl ConfigRepository for AppConfig {
    fn load(&self) -> Result<domain_vo::AppConfigDocument, AppConfigError> {
        self.get_config()
            .map(|config| config_to_domain(&config))
            .map_err(AppConfigError::Repository)
    }

    fn save(&self, config: domain_vo::AppConfigDocument) -> Result<(), AppConfigError> {
        self.with_config_mut(|current| {
            apply_domain_to_config(current, config);
            Ok(())
        })
        .map_err(AppConfigError::Repository)
    }

    fn update(&self, f: ConfigUpdate) -> Result<(), AppConfigError> {
        let mut config = self
            .config
            .lock()
            .map_err(|e| AppConfigError::Repository(format!("ロック取得失敗: {e}")))?;
        let mut domain = config_to_domain(&config);
        f(&mut domain)?;
        apply_domain_to_config(&mut config, domain);
        write_config(&self.config_path, &config).map_err(AppConfigError::Repository)
    }
}

impl AgentConfigRepository for AppConfig {
    fn default_agent_backend(&self) -> Result<Option<String>, AppConfigError> {
        self.get_config()
            .map(|config| config.agents.default)
            .map_err(AppConfigError::Repository)
    }

    fn models_for_backend(&self, backend_id: &str) -> Result<Vec<String>, AppConfigError> {
        let config = self.get_config().map_err(AppConfigError::Repository)?;
        match backend_id {
            "claude" => Ok(config.agents.claude.models),
            "codex" => Ok(config.agents.codex.models),
            _ => Err(AppConfigError::InvalidInput(format!(
                "config schema にバックエンド '{backend_id}' のモデル一覧が存在しません"
            ))),
        }
    }

    fn codex_cli_path(&self) -> Result<Option<String>, AppConfigError> {
        self.get_config()
            .map(|config| config.agents.codex.cli_path)
            .map_err(AppConfigError::Repository)
    }
}

impl ConfigSecretRepository for AppConfig {
    fn configured_secret_values(&self) -> Result<Vec<String>, AppConfigError> {
        self.get_config()
            .map(|config| {
                let mut values = Vec::new();
                for value in [
                    config.server.token,
                    config.server.mcp_token,
                    config.server.notify.webhook_url,
                ] {
                    if value.len() >= 8 {
                        values.push(value);
                    }
                }
                for notion in config.notion.into_values() {
                    if notion.api_token.len() >= 8 {
                        values.push(notion.api_token);
                    }
                }
                values
            })
            .map_err(AppConfigError::Repository)
    }
}

impl NotionConfigRepository for AppConfig {
    fn get(&self, repo_path: &str) -> Result<Option<domain_vo::NotionRepoConfig>, AppConfigError> {
        self.get_config()
            .map(|config| config.notion.get(repo_path).cloned().map(notion_to_domain))
            .map_err(AppConfigError::Repository)
    }

    fn upsert(
        &self,
        repo_path: String,
        config: domain_vo::NotionRepoConfig,
    ) -> Result<(), AppConfigError> {
        self.with_config_mut(|current| {
            current.notion.insert(repo_path, notion_to_model(config));
            Ok(())
        })
        .map_err(AppConfigError::Repository)
    }

    fn remove(&self, repo_path: &str) -> Result<(), AppConfigError> {
        self.with_config_mut(|current| {
            current.notion.remove(repo_path);
            Ok(())
        })
        .map_err(AppConfigError::Repository)
    }
}

fn notion_to_domain(config: crate::notion::types::NotionRepoConfig) -> domain_vo::NotionRepoConfig {
    domain_vo::NotionRepoConfig {
        api_token: config.api_token,
        database_id: config.database_id,
        property_mapping: notion_mapping_to_domain(config.property_mapping),
    }
}

fn notion_to_model(config: domain_vo::NotionRepoConfig) -> crate::notion::types::NotionRepoConfig {
    crate::notion::types::NotionRepoConfig {
        api_token: config.api_token,
        database_id: config.database_id,
        property_mapping: notion_mapping_to_model(config.property_mapping),
    }
}

fn notion_mapping_to_domain(
    mapping: crate::notion::types::PropertyMapping,
) -> domain_vo::NotionPropertyMapping {
    domain_vo::NotionPropertyMapping {
        title: mapping.title,
        labels: mapping
            .labels
            .into_iter()
            .map(|label| domain_vo::NotionLabelProperty {
                name: label.name,
                property_type: label.property_type,
            })
            .collect(),
        branch_name: mapping.branch_name,
        branch_prefix: mapping.branch_prefix,
    }
}

fn notion_mapping_to_model(
    mapping: domain_vo::NotionPropertyMapping,
) -> crate::notion::types::PropertyMapping {
    crate::notion::types::PropertyMapping {
        title: mapping.title,
        labels: mapping
            .labels
            .into_iter()
            .map(|label| crate::notion::types::LabelProperty {
                name: label.name,
                property_type: label.property_type,
            })
            .collect(),
        branch_name: mapping.branch_name,
        branch_prefix: mapping.branch_prefix,
    }
}

/// 読み取り専用の config ローダ。
///
/// 設定ファイルが存在する場合のみ parse して返す。`load_or_create_config` と異なり
/// token 自動生成や `write_config` の副作用を持たず、観測専用 caller（CLI 等）が
/// hidden write を発生させないことを境界仕様として担保する（spec [05]
/// read-only と mutating の分離原則）。
///
/// 設定不在は `Ok(None)` として返す。parse / 読み取り失敗は原因付きで `Err` を返し、
/// caller が「設定読み取り失敗」を「managed worktree でない」と取り違えないようにする。
pub fn read_config_if_exists(path: &Path) -> Result<Option<ReleashConfig>, String> {
    // `try_exists()` を使うことで、metadata 取得失敗（権限不足・I/O エラー等）を
    // 「設定不在」と取り違えずに `Err` として呼出側に伝える。`exists()` は metadata
    // error を false に潰すため、CLI の InvalidInput 誤分類につながる
    // （spec [05] read-only と mutating の分離 / read 失敗は原因付き Err）。
    match path.try_exists() {
        Ok(true) => {}
        Ok(false) => return Ok(None),
        Err(e) => return Err(format!("設定ファイル存在確認失敗: {e}")),
    }
    let content = fs::read_to_string(path).map_err(|e| format!("設定ファイル読み込み失敗: {e}"))?;
    let config = toml::from_str::<ReleashConfig>(&content)
        .map_err(|e| format!("設定ファイルのパース失敗: {e}"))?;
    Ok(Some(config))
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

    let mut needs_write = false;
    if config.server.token.is_empty() {
        config.server.token = generate_token();
        needs_write = true;
    }
    if config.server.mcp_token.is_empty() {
        config.server.mcp_token = generate_token();
        needs_write = true;
    }
    if needs_write {
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

    let tmp_path = next_config_tmp_path(path);
    if let Err(e) = write_config_tmp_file(&tmp_path, &content) {
        let _ = fs::remove_file(&tmp_path);
        return Err(e);
    }
    if let Err(e) = fs::rename(&tmp_path, path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(format!("ファイルのリネーム失敗: {e}"));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("パーミッション設定失敗: {e}"))?;
    }

    Ok(())
}

fn next_config_tmp_path(path: &Path) -> PathBuf {
    let counter = CONFIG_WRITE_TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("releash.toml");
    let tmp_name = format!("{file_name}.{}.{}.tmp", std::process::id(), counter);
    path.parent()
        .map(|parent| parent.join(&tmp_name))
        .unwrap_or_else(|| PathBuf::from(tmp_name))
}

fn write_config_tmp_file(tmp_path: &Path, content: &str) -> Result<(), String> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    let mut file = options
        .open(tmp_path)
        .map_err(|e| format!("一時ファイル作成失敗: {e}"))?;
    file.write_all(content.as_bytes())
        .map_err(|e| format!("一時ファイル書き込み失敗: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::config_models::{
        AgentsSection, AppSection, DesktopNotifyMode, NotifySection, WorkflowSection,
    };
    use super::*;
    use crate::domain::app_config::services::TOKEN_LENGTH;
    use crate::domain::hooks::services::build_hooks_json;
    use crate::notion::types::NotionRepoConfig;
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
        assert_eq!(config.server.mcp_port, 19801);
        assert_eq!(config.server.mcp_token.len(), TOKEN_LENGTH);
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
    fn app_agent_shortcuts_defaults_when_missing() {
        let config: ReleashConfig = toml::from_str(
            r#"
[app]
close_to_tray = false
"#,
        )
        .unwrap();

        assert!(config.app.agent_shortcuts.overrides.is_empty());
    }

    #[test]
    fn app_agent_shortcut_overrides_roundtrip() {
        let mut config = ReleashConfig::default();
        config
            .app
            .agent_shortcuts
            .overrides
            .insert("send".to_string(), "Ctrl+Enter".to_string());

        let encoded = toml::to_string(&config).unwrap();
        let decoded: ReleashConfig = toml::from_str(&encoded).unwrap();

        assert_eq!(
            decoded.app.agent_shortcuts.overrides.get("send"),
            Some(&"Ctrl+Enter".to_string())
        );
    }

    #[test]
    fn atomic_write_leaves_no_tmp_file() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let config = ReleashConfig::default();
        write_config(&path, &config).unwrap();

        assert!(path.exists());
        assert!(!path.with_extension("toml.tmp").exists());
        let tmp_files = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .count();
        assert_eq!(tmp_files, 0);
    }

    #[cfg(unix)]
    #[test]
    fn config_tmp_file_is_created_with_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().unwrap();
        let tmp_path = dir.path().join("releash.toml.tmp");

        write_config_tmp_file(&tmp_path, "secret").unwrap();

        let mode = fs::metadata(&tmp_path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
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
    fn old_remote_section_is_ignored() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let content = r#"
[server]
bind = "127.0.0.1"
port = 9700
token = "existing_token_value_here_with_enough_length_!!"

[remote]
auto_start = true
auto_start_on_lan = true
"#;
        fs::write(&path, content).unwrap();

        let config = load_or_create_config(&path).unwrap();
        assert_eq!(config.server.bind, "127.0.0.1");
        assert_eq!(config.server.port, 9700);
    }

    #[test]
    fn workflow_section_defaults_to_manual_approval() {
        let workflow = WorkflowSection::default();
        assert!(!workflow.approval_auto_approve);
    }

    #[test]
    fn workflow_section_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let mut config = ReleashConfig::default();
        config.server.token = generate_token();
        config.workflow.approval_auto_approve = true;
        write_config(&path, &config).unwrap();

        let reloaded = fs::read_to_string(&path).unwrap();
        let reloaded: ReleashConfig = toml::from_str(&reloaded).unwrap();
        assert!(reloaded.workflow.approval_auto_approve);
    }

    #[test]
    fn existing_config_without_workflow_gets_defaults() {
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
        assert!(!config.workflow.approval_auto_approve);
    }

    #[test]
    fn app_section_defaults() {
        let app = AppSection::default();
        assert!(app.close_to_tray);
        assert!(!app.auto_launch);
        assert!(!app.start_minimized);
        assert!(app.last_root_path.is_empty());
        assert!(app.last_repo_paths.is_empty());
        assert!(app.external_editor.is_empty());
    }

    #[test]
    fn app_section_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let mut config = ReleashConfig::default();
        config.server.token = generate_token();
        config.app.close_to_tray = false;
        config.app.auto_launch = true;
        config.app.start_minimized = true;
        config.app.last_root_path = "/repo/path".to_string();
        config.app.last_repo_paths = vec!["/repo/path".to_string(), "/repo/path2".to_string()];
        config.app.external_editor = "/Applications/Cursor.app".to_string();
        write_config(&path, &config).unwrap();

        let reloaded = fs::read_to_string(&path).unwrap();
        let reloaded: ReleashConfig = toml::from_str(&reloaded).unwrap();
        assert!(!reloaded.app.close_to_tray);
        assert!(reloaded.app.auto_launch);
        assert!(reloaded.app.start_minimized);
        assert_eq!(reloaded.app.last_root_path, "/repo/path");
        assert_eq!(
            reloaded.app.last_repo_paths,
            vec!["/repo/path", "/repo/path2"]
        );
        assert_eq!(reloaded.app.external_editor, "/Applications/Cursor.app");
    }

    #[test]
    fn existing_config_without_app_gets_defaults() {
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
        assert!(config.app.close_to_tray);
        assert!(!config.app.auto_launch);
        assert!(!config.app.start_minimized);
        assert!(config.app.last_root_path.is_empty());
        assert!(config.app.last_repo_paths.is_empty());
    }

    #[test]
    fn old_config_without_last_repo_paths_gets_empty_default_and_ignores_last_bind_ip() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let content = r#"
[server]
bind = "127.0.0.1"
port = 9700
token = "existing_token_value_here_with_enough_length_!!"

[app]
last_root_path = "/old/single/repo"
last_bind_ip = "192.168.1.1"
"#;
        fs::write(&path, content).unwrap();

        let config = load_or_create_config(&path).unwrap();
        assert_eq!(config.app.last_root_path, "/old/single/repo");
        assert!(config.app.last_repo_paths.is_empty());
    }

    #[test]
    fn last_repo_paths_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let mut config = ReleashConfig::default();
        config.server.token = generate_token();
        config.app.last_repo_paths = vec![
            "/repo/a".to_string(),
            "/repo/b".to_string(),
            "/repo/c".to_string(),
        ];
        config.app.last_root_path = "/repo/a".to_string();
        write_config(&path, &config).unwrap();

        let reloaded = load_or_create_config(&path).unwrap();
        assert_eq!(
            reloaded.app.last_repo_paths,
            vec!["/repo/a", "/repo/b", "/repo/c"]
        );
        assert_eq!(reloaded.app.last_root_path, "/repo/a");
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

    #[test]
    fn build_hooks_json_no_python3_or_session_id() {
        let json = build_hooks_json(19700, "test-token");
        let hooks = json.get("hooks").expect("hooks key should exist");
        let event_keys = [
            "UserPromptSubmit",
            "Stop",
            "Notification",
            "PostToolUse",
            "PostToolUseFailure",
            "SessionStart",
        ];

        for key in &event_keys {
            let entries = hooks
                .get(*key)
                .unwrap_or_else(|| panic!("{key} should exist"));
            let arr = entries
                .as_array()
                .unwrap_or_else(|| panic!("{key} should be array"));
            for entry in arr {
                let cmd = entry["hooks"][0]["command"]
                    .as_str()
                    .expect("command should be string");
                assert!(
                    !cmd.contains("python3"),
                    "{key} command should not contain python3"
                );
                assert!(
                    !cmd.contains("session_id"),
                    "{key} command should not contain session_id"
                );
            }
        }
    }

    #[test]
    fn build_hooks_json_has_7_event_entries() {
        let json = build_hooks_json(19700, "test-token");
        let hooks = json.get("hooks").unwrap().as_object().unwrap();
        // 6 keys, but Notification has 2 entries
        let total_entries: usize = hooks.values().map(|v| v.as_array().unwrap().len()).sum();
        assert_eq!(total_entries, 7);
    }

    #[test]
    fn agents_section_defaults() {
        let agents = AgentsSection::default();
        assert!(agents.default.is_none());
        assert!(agents.codex.cli_path.is_none());
    }

    #[test]
    fn agents_section_with_default_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let mut config = ReleashConfig::default();
        config.server.token = generate_token();
        config.agents.default = Some("claude".to_string());
        write_config(&path, &config).unwrap();

        let reloaded = fs::read_to_string(&path).unwrap();
        let reloaded: ReleashConfig = toml::from_str(&reloaded).unwrap();
        assert_eq!(reloaded.agents.default, Some("claude".to_string()));
    }

    #[test]
    fn agents_codex_cli_path_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let mut config = ReleashConfig::default();
        config.server.token = generate_token();
        config.agents.codex.cli_path = Some("/opt/bin/codex".to_string());
        write_config(&path, &config).unwrap();

        let reloaded = fs::read_to_string(&path).unwrap();
        let reloaded: ReleashConfig = toml::from_str(&reloaded).unwrap();
        assert_eq!(
            reloaded.agents.codex.cli_path,
            Some("/opt/bin/codex".to_string())
        );
    }

    #[test]
    fn existing_config_without_agents_gets_defaults() {
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
        assert!(config.agents.default.is_none());
        assert!(config.agents.codex.cli_path.is_none());
    }

    #[test]
    fn models_for_backend_returns_persisted_values() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);
        let mut config = ReleashConfig::default();
        config.agents.claude.models = vec!["a".to_string(), "b".to_string()];
        config.agents.codex.models = vec!["c".to_string()];
        let app_config = AppConfig::new(config, path);

        assert_eq!(
            app_config.models_for_backend("claude").unwrap(),
            vec!["a".to_string(), "b".to_string()]
        );
        assert_eq!(
            app_config.models_for_backend("codex").unwrap(),
            vec!["c".to_string()]
        );
    }

    #[test]
    fn models_for_backend_errors_for_unknown_backend() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);
        let app_config = AppConfig::new(ReleashConfig::default(), path);
        assert!(app_config.models_for_backend("unknown").is_err());
    }
}
