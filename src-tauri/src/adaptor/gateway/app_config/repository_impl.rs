use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use crate::domain::agent_session::aggregates::ProviderExecutable;
use crate::domain::agent_session::{
    ProviderExecutableConfigRepository, ProviderExecutableConfigRepositoryError,
};
use crate::domain::app_config::error::AppConfigError;
use crate::domain::app_config::repository::{
    ConfigRepository, ConfigSecretRepository, ConfigUpdate, NotionConfigRepository,
};
use crate::domain::app_config::services::generate_token;
use crate::domain::app_config::value_objects as domain_vo;
use crate::domain::provider_lifecycle::ProviderKind;

use super::config_models::{
    apply_domain_to_config, config_to_domain, NotionLabelPropertyModel, NotionPropertyMappingModel,
    NotionRepoConfigModel, ReleashConfig,
};

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

impl ProviderExecutableConfigRepository for AppConfig {
    fn configured_executable(
        &self,
        provider: ProviderKind,
    ) -> Result<Option<ProviderExecutable>, ProviderExecutableConfigRepositoryError> {
        let config = self
            .get_config()
            .map_err(|_| ProviderExecutableConfigRepositoryError::Unavailable)?;
        let value = match provider {
            ProviderKind::Claude => config.agents.claude.cli_path,
            ProviderKind::Codex => config.agents.codex.cli_path,
        };
        value
            .map(ProviderExecutable::new)
            .transpose()
            .map_err(|_| ProviderExecutableConfigRepositoryError::InvalidInput)
    }

    fn save_configured_executable(
        &self,
        provider: ProviderKind,
        executable: Option<&ProviderExecutable>,
    ) -> Result<(), ProviderExecutableConfigRepositoryError> {
        let mut config = self
            .config
            .lock()
            .map_err(|_| ProviderExecutableConfigRepositoryError::Unavailable)?;
        let mut next = config.clone();
        let value = executable.map(|executable| executable.as_str().to_string());
        match provider {
            ProviderKind::Claude => next.agents.claude.cli_path = value,
            ProviderKind::Codex => next.agents.codex.cli_path = value,
        }
        write_config(&self.config_path, &next)
            .map_err(|_| ProviderExecutableConfigRepositoryError::Unavailable)?;
        *config = next;
        Ok(())
    }
}

impl ConfigSecretRepository for AppConfig {
    fn configured_secret_values(&self) -> Result<Vec<String>, AppConfigError> {
        self.get_config()
            .map(|config| {
                let mut values = Vec::new();
                if config.server.token.len() >= 8 {
                    values.push(config.server.token);
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

fn notion_to_domain(config: NotionRepoConfigModel) -> domain_vo::NotionRepoConfig {
    domain_vo::NotionRepoConfig {
        api_token: config.api_token,
        database_id: config.database_id,
        property_mapping: notion_mapping_to_domain(config.property_mapping),
    }
}

fn notion_to_model(config: domain_vo::NotionRepoConfig) -> NotionRepoConfigModel {
    NotionRepoConfigModel {
        api_token: config.api_token,
        database_id: config.database_id,
        property_mapping: notion_mapping_to_model(config.property_mapping),
    }
}

fn notion_mapping_to_domain(
    mapping: NotionPropertyMappingModel,
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
) -> NotionPropertyMappingModel {
    NotionPropertyMappingModel {
        title: mapping.title,
        labels: mapping
            .labels
            .into_iter()
            .map(|label| NotionLabelPropertyModel {
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

#[derive(Debug, Default, serde::Deserialize)]
struct LegacyConfigProbe {
    telemetry_enabled: Option<bool>,
    #[serde(default)]
    telemetry: LegacyTelemetryProbe,
}

#[derive(Debug, Default, serde::Deserialize)]
struct LegacyTelemetryProbe {
    performance_telemetry: Option<bool>,
}

pub fn load_or_create_config(path: &Path) -> Result<ReleashConfig, String> {
    let mut needs_write = false;
    let mut config = if path.exists() {
        let content =
            fs::read_to_string(path).map_err(|e| format!("設定ファイル読み込み失敗: {e}"))?;
        let mut config = toml::from_str::<ReleashConfig>(&content)
            .map_err(|e| format!("設定ファイルのパース失敗: {e}"))?;
        let probe = toml::from_str::<LegacyConfigProbe>(&content)
            .map_err(|e| format!("設定ファイルのパース失敗: {e}"))?;
        if probe.telemetry.performance_telemetry.is_none() && probe.telemetry_enabled == Some(false)
        {
            config.telemetry.performance_telemetry = false;
            needs_write = true;
        }
        config
    } else {
        ReleashConfig::default()
    };

    if config.server.token.is_empty() {
        config.server.token = generate_token();
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
        AgentsSection, AppSection, NotionPropertyMappingModel, NotionRepoConfigModel,
        WorkflowSection,
    };
    use super::*;
    use crate::domain::app_config::services::TOKEN_LENGTH;
    use tempfile::TempDir;

    fn config_path(dir: &TempDir) -> PathBuf {
        dir.path().join("releash.toml")
    }

    #[test]
    fn provider_executable_config既存tomlのoverrideをupdateしresetする() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);
        let app_config = AppConfig::new(ReleashConfig::default(), path.clone());
        let executable = ProviderExecutable::new("/opt/custom/claude").unwrap();

        ProviderExecutableConfigRepository::save_configured_executable(
            &app_config,
            ProviderKind::Claude,
            Some(&executable),
        )
        .unwrap();
        assert_eq!(
            ProviderExecutableConfigRepository::configured_executable(
                &app_config,
                ProviderKind::Claude,
            )
            .unwrap()
            .unwrap(),
            executable
        );
        assert!(fs::read_to_string(&path)
            .unwrap()
            .contains("cli_path = \"/opt/custom/claude\""));

        ProviderExecutableConfigRepository::save_configured_executable(
            &app_config,
            ProviderKind::Claude,
            None,
        )
        .unwrap();
        assert_eq!(
            ProviderExecutableConfigRepository::configured_executable(
                &app_config,
                ProviderKind::Claude,
            )
            .unwrap(),
            None
        );
    }

    #[test]
    fn provider_executable_config保存失敗時はmemory上のoverrideも進めない() {
        let dir = TempDir::new().unwrap();
        let mut config = ReleashConfig::default();
        config.agents.claude.cli_path = Some("/before/claude".to_string());
        let blocked_parent = dir.path().join("blocked-parent");
        fs::write(&blocked_parent, "not a directory").unwrap();
        let app_config = AppConfig::new(config, blocked_parent.join("releash.toml"));
        let next = ProviderExecutable::new("/after/claude").unwrap();

        assert_eq!(
            ProviderExecutableConfigRepository::save_configured_executable(
                &app_config,
                ProviderKind::Claude,
                Some(&next),
            )
            .unwrap_err(),
            ProviderExecutableConfigRepositoryError::Unavailable
        );
        assert_eq!(
            ProviderExecutableConfigRepository::configured_executable(
                &app_config,
                ProviderKind::Claude,
            )
            .unwrap()
            .unwrap()
            .as_str(),
            "/before/claude"
        );
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
        assert!(config.telemetry.performance_telemetry);
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
    fn loads_existing_config_with_legacy_mcp_fields() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let content = r#"
[server]
bind = "0.0.0.0"
port = 8080
token = "existing_token_value_here_with_enough_length_!!"
mcp_port = 19801
mcp_token = "legacy_mcp_token_value"
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
    fn performance_telemetry_defaults_to_true() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let content = "[server]\nport = 9700\n";
        fs::write(&path, content).unwrap();

        let config = load_or_create_config(&path).unwrap();
        assert!(config.telemetry.performance_telemetry);
    }

    #[test]
    fn performance_telemetry_disabled_persists() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let content = "[server]\nport = 9700\n\n[telemetry]\nperformance_telemetry = false\n";
        fs::write(&path, content).unwrap();

        let config = load_or_create_config(&path).unwrap();
        assert!(!config.telemetry.performance_telemetry);
    }

    #[test]
    fn legacy_telemetry_enabled_false_migrates_to_performance_telemetry_false() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let content = r#"
telemetry_enabled = false

[server]
token = "existing_token_value_here_with_enough_length_!!"
"#;
        fs::write(&path, content).unwrap();

        let config = load_or_create_config(&path).unwrap();

        assert!(!config.telemetry.performance_telemetry);
        let saved = fs::read_to_string(&path).unwrap();
        assert!(!saved.contains("telemetry_enabled"));
        assert!(saved.contains("performance_telemetry = false"));
    }

    #[test]
    fn legacy_telemetry_enabled_true_defaults_performance_telemetry_to_true() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let content = r#"
telemetry_enabled = true

[server]
token = "existing_token_value_here_with_enough_length_!!"
"#;
        fs::write(&path, content).unwrap();

        let config = load_or_create_config(&path).unwrap();

        assert!(config.telemetry.performance_telemetry);
    }

    #[test]
    fn missing_legacy_telemetry_enabled_defaults_performance_telemetry_to_true() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let content = r#"
[server]
token = "existing_token_value_here_with_enough_length_!!"
"#;
        fs::write(&path, content).unwrap();

        let config = load_or_create_config(&path).unwrap();

        assert!(config.telemetry.performance_telemetry);
    }

    #[test]
    fn explicit_new_performance_telemetry_true_wins_over_legacy_false() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let content = r#"
telemetry_enabled = false

[server]
token = "existing_token_value_here_with_enough_length_!!"

[telemetry]
performance_telemetry = true
"#;
        fs::write(&path, content).unwrap();

        let config = load_or_create_config(&path).unwrap();

        assert!(config.telemetry.performance_telemetry);
    }

    #[test]
    fn explicit_new_performance_telemetry_false_wins_over_legacy_true() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let content = r#"
telemetry_enabled = true

[server]
token = "existing_token_value_here_with_enough_length_!!"

[telemetry]
performance_telemetry = false
"#;
        fs::write(&path, content).unwrap();

        let config = load_or_create_config(&path).unwrap();

        assert!(!config.telemetry.performance_telemetry);
    }

    #[test]
    fn read_config_if_exists_does_not_migrate_legacy_telemetry_enabled() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let content = r#"
telemetry_enabled = false

[server]
token = "existing_token_value_here_with_enough_length_!!"
"#;
        fs::write(&path, content).unwrap();

        let config = read_config_if_exists(&path).unwrap().unwrap();
        let unchanged = fs::read_to_string(&path).unwrap();

        assert!(config.telemetry.performance_telemetry);
        assert!(unchanged.contains("telemetry_enabled = false"));
        assert!(!unchanged.contains("performance_telemetry = false"));
    }

    #[test]
    fn legacy_app_agent_shortcuts_are_ignored() {
        let config: ReleashConfig = toml::from_str(
            r#"
[app]
close_to_tray = false

[app.agent_shortcuts.overrides]
new_thread = "Ctrl Shift N"
"#,
        )
        .unwrap();

        let encoded = toml::to_string(&config).unwrap();

        assert!(!config.app.close_to_tray);
        assert!(!encoded.contains("agent_shortcuts"));
        assert!(!encoded.contains("new_thread"));
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
        assert!(config.telemetry.performance_telemetry);
    }

    #[test]
    fn telemetry_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let mut config = ReleashConfig::default();
        config.server.token = generate_token();
        config.telemetry.crash_reporting = false;
        config.telemetry.performance_telemetry = false;
        write_config(&path, &config).unwrap();

        let reloaded = fs::read_to_string(&path).unwrap();
        let reloaded: ReleashConfig = toml::from_str(&reloaded).unwrap();
        assert!(!reloaded.telemetry.crash_reporting);
        assert!(!reloaded.telemetry.performance_telemetry);
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
        assert!(config.telemetry.performance_telemetry);
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
            NotionRepoConfigModel {
                api_token: "ntn_test_token".to_string(),
                database_id: "db-id-456".to_string(),
                property_mapping: NotionPropertyMappingModel::default(),
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
    fn notion_config_repository_upsert_get_remove_preserves_all_fields() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);
        let app_config = AppConfig::new(ReleashConfig::default(), path);
        let repo_path = "/path/to/repo";
        let config = domain_vo::NotionRepoConfig {
            api_token: "ntn_test_token".to_string(),
            database_id: "db-id-456".to_string(),
            property_mapping: domain_vo::NotionPropertyMapping {
                title: "Task Name".to_string(),
                labels: vec![
                    domain_vo::NotionLabelProperty {
                        name: "Status".to_string(),
                        property_type: "status".to_string(),
                    },
                    domain_vo::NotionLabelProperty {
                        name: "Tags".to_string(),
                        property_type: "multi_select".to_string(),
                    },
                ],
                branch_name: "Branch".to_string(),
                branch_prefix: "feat/".to_string(),
            },
        };

        NotionConfigRepository::upsert(&app_config, repo_path.to_string(), config.clone()).unwrap();
        let stored = NotionConfigRepository::get(&app_config, repo_path)
            .unwrap()
            .unwrap();

        assert_eq!(stored.api_token, config.api_token);
        assert_eq!(stored.database_id, config.database_id);
        assert_eq!(stored.property_mapping.title, config.property_mapping.title);
        assert_eq!(
            stored.property_mapping.labels,
            config.property_mapping.labels
        );
        assert_eq!(
            stored.property_mapping.branch_name,
            config.property_mapping.branch_name
        );
        assert_eq!(
            stored.property_mapping.branch_prefix,
            config.property_mapping.branch_prefix
        );

        NotionConfigRepository::remove(&app_config, repo_path).unwrap();
        assert!(NotionConfigRepository::get(&app_config, repo_path)
            .unwrap()
            .is_none());
    }

    #[test]
    fn notion_config_model_domain_conversion_roundtrip_preserves_mapping() {
        let model = NotionRepoConfigModel {
            api_token: "ntn_test_token".to_string(),
            database_id: "db-id-456".to_string(),
            property_mapping: NotionPropertyMappingModel {
                title: "Task Name".to_string(),
                labels: vec![
                    NotionLabelPropertyModel {
                        name: "Status".to_string(),
                        property_type: "status".to_string(),
                    },
                    NotionLabelPropertyModel {
                        name: "Tags".to_string(),
                        property_type: "multi_select".to_string(),
                    },
                ],
                branch_name: "Branch".to_string(),
                branch_prefix: "feat/".to_string(),
            },
        };

        let domain = notion_to_domain(model.clone());
        let model_again = notion_to_model(domain.clone());

        assert_eq!(model_again.api_token, model.api_token);
        assert_eq!(model_again.database_id, model.database_id);
        assert_eq!(
            model_again.property_mapping.title,
            model.property_mapping.title
        );
        assert_eq!(
            model_again.property_mapping.labels,
            model.property_mapping.labels
        );
        assert_eq!(
            model_again.property_mapping.branch_name,
            model.property_mapping.branch_name
        );
        assert_eq!(
            model_again.property_mapping.branch_prefix,
            model.property_mapping.branch_prefix
        );
        assert_eq!(notion_to_domain(model_again), domain);
    }

    #[test]
    fn agents_section_defaults() {
        let agents = AgentsSection::default();
        assert!(agents.claude.cli_path.is_none());
        assert!(agents.codex.cli_path.is_none());
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
    fn agents_claude_cli_path_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);

        let mut config = ReleashConfig::default();
        config.server.token = generate_token();
        config.agents.claude.cli_path = Some("/opt/bin/claude".to_string());
        write_config(&path, &config).unwrap();

        let reloaded = fs::read_to_string(&path).unwrap();
        let reloaded: ReleashConfig = toml::from_str(&reloaded).unwrap();
        assert_eq!(
            reloaded.agents.claude.cli_path,
            Some("/opt/bin/claude".to_string())
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
        assert!(config.agents.codex.cli_path.is_none());
    }

    #[test]
    fn legacy_agent_selection_fields_are_ignored_and_not_reserialized() {
        let dir = TempDir::new().unwrap();
        let path = config_path(&dir);
        let legacy = r#"
[agents]
default = "claude"

[agents.claude]
cli_path = "/opt/bin/claude"
models = ["legacy-claude"]

[agents.codex]
cli_path = "/opt/bin/codex"
models = ["legacy-codex"]
"#;
        let config: ReleashConfig = toml::from_str(legacy).unwrap();
        write_config(&path, &config).unwrap();
        let serialized = fs::read_to_string(&path).unwrap();

        assert!(!serialized.contains("default ="), "{serialized}");
        assert!(!serialized.contains("models ="), "{serialized}");
        assert!(serialized.contains("cli_path = \"/opt/bin/claude\""));
        assert!(serialized.contains("cli_path = \"/opt/bin/codex\""));
    }
}
