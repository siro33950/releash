use std::fs;
use std::path::PathBuf;

pub struct ClaudeHooksSettingsRepository;

impl ClaudeHooksSettingsRepository {
    fn settings_path() -> Result<PathBuf, String> {
        let home = dirs::home_dir().ok_or("ホームディレクトリの取得失敗")?;
        Ok(home.join(".claude").join("settings.json"))
    }

    pub fn load_or_empty(&self) -> Result<serde_json::Value, String> {
        let settings_path = Self::settings_path()?;
        if settings_path.exists() {
            let content = fs::read_to_string(&settings_path)
                .map_err(|e| format!("settings.json読み込み失敗: {e}"))?;
            serde_json::from_str(&content).map_err(|e| format!("settings.jsonパース失敗: {e}"))
        } else {
            Ok(serde_json::json!({}))
        }
    }

    pub fn load_optional(&self) -> Result<Option<serde_json::Value>, String> {
        let settings_path = Self::settings_path()?;
        if !settings_path.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(&settings_path)
            .map_err(|e| format!("settings.json読み込み失敗: {e}"))?;
        let parsed =
            serde_json::from_str(&content).map_err(|e| format!("settings.jsonパース失敗: {e}"))?;
        Ok(Some(parsed))
    }

    pub fn save(&self, settings: &serde_json::Value) -> Result<(), String> {
        let settings_path = Self::settings_path()?;
        if let Some(parent) = settings_path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("ディレクトリ作成失敗: {e}"))?;
        }
        let content = serde_json::to_string_pretty(settings)
            .map_err(|e| format!("JSONシリアライズ失敗: {e}"))?;
        let tmp_path = settings_path.with_extension("json.tmp");
        fs::write(&tmp_path, &content).map_err(|e| format!("一時ファイル書き込み失敗: {e}"))?;
        fs::rename(&tmp_path, &settings_path).map_err(|e| format!("ファイルのリネーム失敗: {e}"))
    }
}
