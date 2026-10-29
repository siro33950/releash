use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use crate::domain::app_config::error::AppConfigError;
use crate::domain::app_config::repository::{
    AppConfigRepository, ConfigSecretRepository, RepositoryConfigRepository,
};
use crate::domain::failure::{TechnicalFailure, TechnicalFailureNature};

use super::config_models::{
    apply_domain_to_config, try_config_to_domain, NotionLabelPropertyModel,
    NotionPropertyMappingModel, NotionRepoConfigModel, ReleashConfig,
};
use crate::adaptor::gateway::repository::git_config::{get_releash_base, set_releash_base};
use crate::domain::app_config::value_objects as domain_vo;

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
        let mut next = config.clone();
        let result = f(&mut next)?;
        write_config(&self.config_path, &next)?;
        *config = next;
        Ok(result)
    }

    pub fn read_legacy_server_token(&self) -> Result<Option<String>, AppConfigError> {
        let content = match fs::read_to_string(&self.config_path) {
            Ok(content) => content,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(AppConfigError::Technical(TechnicalFailure {
                    nature: TechnicalFailureNature::Other,
                    message: format!("設定ファイル読み込み失敗: {error}"),
                }))
            }
        };
        let legacy = toml::from_str::<toml::Value>(&content).map_err(|_| {
            AppConfigError::Technical(TechnicalFailure {
                nature: TechnicalFailureNature::Other,
                message: "秘匿対象の旧設定のパース失敗".to_string(),
            })
        })?;
        Ok(legacy
            .get("server")
            .and_then(|server| server.get("token"))
            .and_then(toml::Value::as_str)
            .filter(|token| token.len() >= 8)
            .map(str::to_string))
    }
}

impl AppConfigRepository for AppConfig {
    fn load(&self) -> Result<crate::domain::app_config::AppConfig, AppConfigError> {
        let config = self.get_config().map_err(|message| {
            AppConfigError::Technical(TechnicalFailure {
                nature: TechnicalFailureNature::Other,
                message,
            })
        })?;
        try_config_to_domain(&config)
    }

    fn save(&self, config: crate::domain::app_config::AppConfig) -> Result<(), AppConfigError> {
        self.with_config_mut(|current| {
            apply_domain_to_config(current, config);
            Ok(())
        })
        .map_err(|message| {
            AppConfigError::Technical(TechnicalFailure {
                nature: TechnicalFailureNature::Other,
                message,
            })
        })
    }
}

impl crate::usecase::notion::query_service::NotionConfigQueryService for AppConfig {
    fn get_config(
        &self,
        repo_path: &str,
    ) -> Result<Option<crate::usecase::notion::usecase::NotionRepoConfigDto>, AppConfigError> {
        use crate::usecase::notion::usecase::{
            NotionLabelPropertyDto, NotionPropertyMappingDto, NotionRepoConfigDto,
        };
        let config = self.config.lock().map_err(|error| {
            AppConfigError::Technical(TechnicalFailure {
                nature: TechnicalFailureNature::Other,
                message: format!("ロック取得失敗: {error}"),
            })
        })?;
        Ok(config
            .notion
            .get(repo_path)
            .map(|stored| NotionRepoConfigDto {
                api_token: stored.api_token.clone(),
                database_id: stored.database_id.clone(),
                property_mapping: NotionPropertyMappingDto {
                    title: stored.property_mapping.title.clone(),
                    labels: stored
                        .property_mapping
                        .labels
                        .iter()
                        .map(|label| NotionLabelPropertyDto {
                            name: label.name.clone(),
                            property_type: label.property_type.clone(),
                        })
                        .collect(),
                    branch_name: stored.property_mapping.branch_name.clone(),
                    branch_prefix: stored.property_mapping.branch_prefix.clone(),
                },
            }))
    }
}

impl ConfigSecretRepository for AppConfig {
    fn configured_secret_values(&self) -> Result<Vec<String>, AppConfigError> {
        let config = self.get_config().map_err(|message| {
            AppConfigError::Technical(TechnicalFailure {
                nature: TechnicalFailureNature::Other,
                message,
            })
        })?;
        let mut values = Vec::new();
        for notion in config.notion.into_values() {
            if notion.api_token.len() >= 8 {
                values.push(notion.api_token);
            }
        }
        match self.read_legacy_server_token() {
            Ok(token) => values.extend(token),
            Err(error) => log::warn!("{error}"),
        }
        Ok(values)
    }
}

impl RepositoryConfigRepository for AppConfig {
    fn load(
        &self,
        repo_path: &str,
    ) -> Result<crate::domain::app_config::RepositoryConfig, AppConfigError> {
        let config = self
            .config
            .lock()
            .map_err(|error| config_technical_error(error.to_string()))?;
        let base = get_releash_base(repo_path).map_err(repository_config_error)?;
        let notion = config.notion.get(repo_path).cloned().map(notion_to_domain);
        Ok(crate::domain::app_config::RepositoryConfig::new(
            repo_path.to_owned(),
            base,
            notion,
        ))
    }

    fn save(
        &self,
        config: crate::domain::app_config::RepositoryConfig,
    ) -> Result<(), AppConfigError> {
        let mut current = self
            .config
            .lock()
            .map_err(|error| config_technical_error(error.to_string()))?;
        let repo_path = config.repo_path();
        let previous_base = get_releash_base(repo_path).map_err(repository_config_error)?;
        let base_changed = previous_base.as_deref() != config.base_branch();
        let previous_notion = current.notion.get(repo_path).cloned().map(notion_to_domain);
        let notion_changed = previous_notion.as_ref() != config.notion();
        let mut next = current.clone();
        match config.notion() {
            Some(notion) => {
                next.notion
                    .insert(repo_path.to_owned(), notion_to_model(notion.clone()));
            }
            None => {
                next.notion.remove(repo_path);
            }
        }
        if base_changed {
            set_releash_base(repo_path, config.base_branch()).map_err(repository_config_error)?;
        }
        if notion_changed {
            if let Err(error) = write_config_typed(&self.config_path, &next) {
                if base_changed {
                    set_releash_base(repo_path, previous_base.as_deref()).map_err(|rollback| {
                        config_technical_error(format!(
                            "{error}; base branch の復元失敗: {rollback}"
                        ))
                    })?;
                }
                return Err(AppConfigError::Technical(error));
            }
            *current = next;
        }
        Ok(())
    }
}

fn config_technical_error(message: String) -> AppConfigError {
    AppConfigError::Technical(TechnicalFailure {
        nature: TechnicalFailureNature::Other,
        message,
    })
}

fn repository_config_error(error: crate::domain::repository::RepositoryError) -> AppConfigError {
    match error {
        crate::domain::repository::RepositoryError::Technical(failure) => {
            AppConfigError::Technical(failure)
        }
        error => config_technical_error(error.to_string()),
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
    let mut needs_write = !path.exists();
    let config = if path.exists() {
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

    if needs_write {
        write_config(path, &config)?;
    }

    Ok(config)
}

pub fn write_config(path: &Path, config: &ReleashConfig) -> Result<(), String> {
    write_config_typed(path, config).map_err(|error| error.to_string())
}

fn write_config_typed(
    path: &Path,
    config: &ReleashConfig,
) -> Result<(), crate::domain::failure::TechnicalFailure> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| config_io_failure(error, "ディレクトリ作成失敗"))?;
    }

    let content = toml::to_string_pretty(config).map_err(|error| {
        crate::domain::failure::TechnicalFailure {
            nature: crate::domain::failure::TechnicalFailureNature::Other,
            message: format!("設定のシリアライズ失敗: {error}"),
        }
    })?;

    let tmp_path = next_config_tmp_path(path);
    if let Err(e) = write_config_tmp_file(&tmp_path, &content) {
        let _ = fs::remove_file(&tmp_path);
        return Err(e);
    }
    if let Err(e) = fs::rename(&tmp_path, path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(config_io_failure(e, "ファイルのリネーム失敗"));
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

pub fn write_config_tmp_file(
    tmp_path: &Path,
    content: &str,
) -> Result<(), crate::domain::failure::TechnicalFailure> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    let mut file = options
        .open(tmp_path)
        .map_err(|error| config_io_failure(error, "一時ファイル作成失敗"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|error| config_io_failure(error, "パーミッション設定失敗"))?;
    }
    file.write_all(content.as_bytes())
        .map_err(|error| config_io_failure(error, "一時ファイル書き込み失敗"))?;
    Ok(())
}

fn config_io_failure(
    error: std::io::Error,
    context: &str,
) -> crate::domain::failure::TechnicalFailure {
    crate::domain::failure::TechnicalFailure {
        nature: crate::adaptor::gateway::shared::background_io::nature(&error),
        message: format!("{context}: {error}"),
    }
}

#[cfg(test)]
#[path = "repository_impl_test.rs"]
mod repository_impl_tests;
