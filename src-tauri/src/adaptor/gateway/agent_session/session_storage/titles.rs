use std::collections::HashMap;
use std::path::Path;

use super::layout::{invalid_session_error_message, session_titles_file};
use super::FileSessionStorage;

impl FileSessionStorage {
    pub(super) fn load_session_titles(
        &self,
        app_data_dir: &Path,
    ) -> Result<HashMap<String, String>, String> {
        let path = session_titles_file(app_data_dir);
        match std::fs::read_to_string(&path) {
            Ok(content) => {
                serde_json::from_str(&content).map_err(|_| invalid_session_error_message())
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(HashMap::new()),
            Err(e) => Err(format!("Failed to read session titles: {e}")),
        }
    }

    pub(super) fn save_session_titles(
        &self,
        app_data_dir: &Path,
        titles: &HashMap<String, String>,
    ) -> Result<(), String> {
        std::fs::create_dir_all(app_data_dir)
            .map_err(|e| format!("Failed to create app data dir: {e}"))?;
        let path = session_titles_file(app_data_dir);
        let json = serde_json::to_string_pretty(titles)
            .map_err(|e| format!("Failed to serialize session titles: {e}"))?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json)
            .map_err(|e| format!("Failed to write session titles temp file: {e}"))?;
        std::fs::rename(&tmp, &path)
            .map_err(|e| format!("Failed to rename session titles temp file: {e}"))
    }

    pub fn session_title(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<String>, String> {
        Ok(self
            .load_session_titles(app_data_dir)?
            .get(session_id)
            .cloned())
    }

    pub fn write_session_title(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        title: Option<&str>,
    ) -> Result<(), String> {
        let _lock = self.file_lock.lock();
        let mut titles = self.load_session_titles(app_data_dir)?;
        match title {
            Some(title) if !title.is_empty() => {
                titles.insert(session_id.to_string(), title.to_string());
            }
            _ => {
                titles.remove(session_id);
            }
        }
        self.save_session_titles(app_data_dir, &titles)
    }
}
