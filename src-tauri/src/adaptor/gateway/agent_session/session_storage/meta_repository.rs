use std::path::Path;
use std::sync::atomic::Ordering;

use super::layout::{
    legacy_meta_file, meta_file_in_dir, session_dir, session_file, sessions_dir, validate_meta,
    write_json_pretty_atomic, UUID_RE,
};
use super::FileSessionStorage;
use crate::usecase::agent_session::session::SessionMeta;

impl FileSessionStorage {
    pub(super) fn remove_session_file_and_cache(&self, app_data_dir: &Path, session_id: &str) {
        if let Ok(file) = session_file(app_data_dir, session_id) {
            let _ = std::fs::remove_file(file);
        }
        if let Ok(file) = legacy_meta_file(app_data_dir, session_id) {
            let _ = std::fs::remove_file(file);
        }
        if let Ok(dir) = session_dir(app_data_dir, session_id) {
            let _ = std::fs::remove_dir_all(dir);
        }
        self.cache.write().remove(session_id);
        self.invalid_sessions.write().remove(session_id);
    }

    pub fn list_metas(&self, app_data_dir: &Path) -> Result<Vec<SessionMeta>, String> {
        self.ensure_loaded(app_data_dir)?;
        Ok(self.cache.read().values().cloned().collect())
    }

    pub fn remove_session(&self, app_data_dir: &Path, session_id: &str) {
        self.remove_session_file_and_cache(app_data_dir, session_id);
    }

    pub fn get_session_meta(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<SessionMeta>, String> {
        self.ensure_loaded(app_data_dir)?;
        if let Some(err) = self.invalid_sessions.read().get(session_id) {
            return Err(err.clone());
        }
        Ok(self.cache.read().get(session_id).cloned())
    }

    /// `file_lock` を保持したまま meta を read-modify-write する原子更新 API。
    /// SessionStore 側で読み込んだ meta を後から書き戻すと、間に走った
    /// `append_message` 等の更新が上書きされてしまうため、ストレージ層で
    /// ロック内 RMW を完結させる。
    pub fn update_session_meta(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        update: &mut dyn FnMut(&mut SessionMeta) -> Result<(), String>,
    ) -> Result<SessionMeta, String> {
        self.ensure_loaded(app_data_dir)?;
        if let Some(err) = self.invalid_sessions.read().get(session_id) {
            return Err(err.clone());
        }
        if !self.cache.read().contains_key(session_id) {
            return Err(format!("Session not found: {session_id}"));
        }
        self.ensure_session_layout(app_data_dir, session_id)?;
        let _lock = self.file_lock.lock();
        let dir = session_dir(app_data_dir, session_id)?;
        let mut meta = self.read_meta_from_dir(&dir, session_id)?;
        update(&mut meta)?;
        let meta = validate_meta(meta, session_id)?;
        write_json_pretty_atomic(&meta_file_in_dir(&dir), &meta, "session meta")?;
        self.cache.write().insert(meta.id.clone(), meta.clone());
        Ok(meta)
    }

    pub(super) fn ensure_loaded(&self, app_data_dir: &Path) -> Result<(), String> {
        if self.loaded.load(Ordering::Acquire) {
            return Ok(());
        }
        let _lock = self.file_lock.lock();
        // Double-check after acquiring lock
        if self.loaded.load(Ordering::Acquire) {
            return Ok(());
        }
        let dir = sessions_dir(app_data_dir);
        if !dir.exists() {
            self.loaded.store(true, Ordering::Release);
            return Ok(());
        }
        let entries =
            std::fs::read_dir(&dir).map_err(|e| format!("Failed to read sessions dir: {e}"))?;
        let mut cache = self.cache.write();
        let mut invalid_sessions = self.invalid_sessions.write();
        for entry in entries {
            let entry = entry.map_err(|e| format!("Failed to read dir entry: {e}"))?;
            let path = entry.path();
            if path.is_dir() {
                let Some(session_id) = path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .filter(|s| UUID_RE.is_match(s))
                    .map(str::to_string)
                else {
                    continue;
                };
                match self.read_meta_from_dir(&path, &session_id) {
                    Ok(meta) => {
                        cache.insert(session_id, meta);
                    }
                    Err(err) => {
                        log::error!("Failed to load session meta {:?}: {err}", path.display());
                        invalid_sessions.insert(session_id, err);
                    }
                }
                continue;
            }
            if path.extension().is_none_or(|ext| ext != "json") {
                continue;
            }
            let Some(file_session_id) = path
                .file_stem()
                .and_then(|s| s.to_str())
                .filter(|s| UUID_RE.is_match(s))
                .map(str::to_string)
            else {
                continue;
            };
            if session_dir(app_data_dir, &file_session_id)
                .is_ok_and(|dir| meta_file_in_dir(&dir).exists())
            {
                continue;
            }
            match self.read_legacy_flat_meta(app_data_dir, &path, &file_session_id) {
                Ok(meta) => {
                    cache.insert(meta.id.clone(), meta);
                }
                Err(err) => {
                    log::error!("Failed to parse session file {:?}: {err}", path.display());
                    invalid_sessions.insert(file_session_id, err);
                }
            }
        }
        self.loaded.store(true, Ordering::Release);
        Ok(())
    }
}
