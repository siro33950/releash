use std::io::BufReader;
use std::path::Path;
use std::sync::atomic::Ordering;

use super::layout::{
    invalid_session_error_message_with_id, meta_file_in_dir, session_dir, sessions_dir,
    validate_meta, UUID_RE,
};
#[cfg(test)]
use super::layout::{legacy_meta_file, session_file, write_json_pretty_atomic};
use super::private_context::hydrate_meta_private_context;
#[cfg(test)]
use super::private_context::write_private_context_to_dir;
#[cfg(test)]
use super::transaction::SessionMetaEventTransaction;
use super::FileSessionStorage;
#[cfg(test)]
use crate::usecase::agent_session::event_log::AgentSessionEvent;
use crate::usecase::agent_session::session::{SessionMeta, SessionReviewContext};

impl FileSessionStorage {
    pub(super) fn read_meta_from_dir(
        &self,
        dir: &Path,
        expected_id: &str,
    ) -> Result<SessionMeta, String> {
        #[cfg(test)]
        self.meta_read_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let file = std::fs::File::open(meta_file_in_dir(dir))
            .map_err(|_| invalid_session_error_message_with_id(expected_id))?;
        let mut meta: SessionMeta = serde_json::from_reader(BufReader::new(file))
            .map_err(|_| invalid_session_error_message_with_id(expected_id))?;
        hydrate_meta_private_context(dir, &mut meta);
        validate_meta(meta, expected_id)
    }

    #[cfg(test)]
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
        self.materialization_pending_sessions
            .write()
            .remove(session_id);
    }

    pub fn list_metas(&self, app_data_dir: &Path) -> Result<Vec<SessionMeta>, String> {
        crate::other::telemetry::measure_result(
            crate::other::telemetry::HotPath::SessionList,
            || {
                self.ensure_loaded(app_data_dir)?;
                #[cfg(test)]
                {
                    let session_ids = self
                        .materialization_pending_sessions
                        .read()
                        .iter()
                        .cloned()
                        .collect::<Vec<_>>();
                    for session_id in session_ids {
                        if let Err(error) =
                            self.reconcile_session_transaction(app_data_dir, &session_id)
                        {
                            log::warn!(
                            "failed to reconcile pending session transaction while listing {session_id}: {error}"
                        );
                        }
                    }
                }
                Ok(self.cache.read().values().cloned().collect())
            },
        )
    }

    #[cfg(test)]
    pub fn remove_session(&self, app_data_dir: &Path, session_id: &str) {
        self.remove_session_file_and_cache(app_data_dir, session_id);
    }

    pub fn get_session_meta(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<SessionMeta>, String> {
        crate::other::telemetry::measure_result(
            crate::other::telemetry::HotPath::SessionGetMeta,
            || {
                if !self.reconcile_session_transaction(app_data_dir, session_id)? {
                    return Ok(None);
                }
                if let Some(err) = self.invalid_sessions.read().get(session_id) {
                    return Err(err.clone());
                }
                Ok(self.cache.read().get(session_id).cloned())
            },
        )
    }

    pub fn get_session_review_context(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<SessionReviewContext>, String> {
        crate::other::telemetry::measure_result(
            crate::other::telemetry::HotPath::SessionGetMeta,
            || {
                if !UUID_RE.is_match(session_id) {
                    return Ok(None);
                }
                let dir = session_dir(app_data_dir, session_id)?;
                #[cfg(test)]
                if self
                    .materialization_pending_sessions
                    .read()
                    .contains(session_id)
                {
                    let _lock = self.file_lock.lock();
                    self.apply_pending_session_transaction(&dir, session_id)?;
                }
                if meta_file_in_dir(&dir).exists() {
                    return self
                        .read_meta_from_dir(&dir, session_id)
                        .map(SessionReviewContext::from)
                        .map(Some);
                }

                Ok(None)
            },
        )
    }

    /// `file_lock` を保持したまま meta を read-modify-write する原子更新 API。
    /// SessionStore 側で読み込んだ meta を後から書き戻すと、間に走った
    /// `append_message` 等の更新が上書きされてしまうため、ストレージ層で
    /// ロック内 RMW を完結させる。
    #[cfg(test)]
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
        let _lock = self.file_lock.lock();
        let dir = session_dir(app_data_dir, session_id)?;
        self.apply_pending_session_transaction(&dir, session_id)?;
        let mut meta = self.read_meta_from_dir(&dir, session_id)?;
        update(&mut meta)?;
        let meta = validate_meta(meta, session_id)?;
        write_private_context_to_dir(&dir, &meta)?;
        write_json_pretty_atomic(&meta_file_in_dir(&dir), &meta, "session meta")?;
        self.cache.write().insert(meta.id.clone(), meta.clone());
        Ok(meta)
    }

    #[cfg(test)]
    pub fn update_session_meta_and_append_session_events(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        update: &mut dyn FnMut(&mut SessionMeta) -> Result<(), String>,
        events: &[AgentSessionEvent],
    ) -> Result<SessionMeta, String> {
        self.ensure_loaded(app_data_dir)?;
        if let Some(err) = self.invalid_sessions.read().get(session_id) {
            return Err(err.clone());
        }
        if !self.cache.read().contains_key(session_id) {
            return Err(format!("Session not found: {session_id}"));
        }
        let _lock = self.file_lock.lock();
        let dir = session_dir(app_data_dir, session_id)?;
        self.apply_pending_session_transaction(&dir, session_id)?;
        let mut meta = self.read_meta_from_dir(&dir, session_id)?;
        update(&mut meta)?;
        let meta = validate_meta(meta, session_id)?;

        let base_event_count = self.read_session_events_from_dir(&dir)?.len();
        let transaction =
            SessionMetaEventTransaction::new(session_id, base_event_count, meta.clone(), events);
        self.commit_meta_event_transaction(&dir, &transaction)?;
        self.cache.write().insert(meta.id.clone(), meta.clone());
        Ok(meta)
    }

    pub(super) fn reconcile_session_transaction(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<bool, String> {
        self.ensure_loaded(app_data_dir)?;
        if let Some(err) = self.invalid_sessions.read().get(session_id) {
            return Err(err.clone());
        }
        if !self.cache.read().contains_key(session_id) {
            return Ok(false);
        }
        #[cfg(not(test))]
        return Ok(true);
        #[cfg(test)]
        {
            if !self
                .materialization_pending_sessions
                .read()
                .contains(session_id)
            {
                return Ok(true);
            }
            let _lock = self.file_lock.lock();
            if !self
                .materialization_pending_sessions
                .read()
                .contains(session_id)
            {
                return Ok(true);
            }
            let dir = session_dir(app_data_dir, session_id)?;
            self.apply_committed_meta_event_transaction(&dir, session_id)
                .map_err(|error| error.into_message())?;
            let meta = self.read_meta_from_dir(&dir, session_id)?;
            self.cache.write().insert(session_id.to_string(), meta);
            Ok(true)
        }
    }

    #[cfg(test)]
    pub(super) fn apply_pending_session_transaction(
        &self,
        dir: &Path,
        session_id: &str,
    ) -> Result<(), String> {
        if self
            .materialization_pending_sessions
            .read()
            .contains(session_id)
        {
            self.apply_committed_meta_event_transaction(dir, session_id)
                .map_err(|error| error.into_message())?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn reset_meta_read_count(&self) {
        self.meta_read_count.store(0, Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(crate) fn meta_read_count(&self) -> usize {
        self.meta_read_count.load(Ordering::SeqCst)
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
                {
                    match self.apply_committed_meta_event_transaction(&path, &session_id) {
                        Ok(()) => {}
                        Err(error) if error.is_corrupt() => {
                            let error = error.into_message();
                            log::error!(
                                "Failed to recover corrupt session transaction {:?}: {error}",
                                path.display()
                            );
                            invalid_sessions.insert(session_id, error);
                            continue;
                        }
                        Err(error) => {
                            log::warn!(
                                "Session transaction materialization remains pending after startup failure {:?}: {error}",
                                path.display()
                            );
                            self.materialization_pending_sessions
                                .write()
                                .insert(session_id.clone());
                        }
                    }
                }
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
        }
        self.loaded.store(true, Ordering::Release);
        Ok(())
    }
}
