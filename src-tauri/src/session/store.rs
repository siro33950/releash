use parking_lot::RwLock;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::LazyLock;

use super::{ChatSession, SessionState, SessionSummary};

pub struct SessionStore {
    cache: RwLock<HashMap<String, ChatSession>>,
    file_lock: parking_lot::Mutex<()>,
    loaded: AtomicBool,
}

impl Default for SessionStore {
    fn default() -> Self {
        Self {
            cache: RwLock::new(HashMap::new()),
            file_lock: parking_lot::Mutex::new(()),
            loaded: AtomicBool::new(false),
        }
    }
}

fn sessions_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("sessions")
}

static UUID_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$")
        .unwrap()
});

fn session_file(app_data_dir: &Path, session_id: &str) -> Result<PathBuf, String> {
    if !UUID_RE.is_match(session_id) {
        return Err(format!("Invalid session_id: {session_id}"));
    }
    Ok(sessions_dir(app_data_dir).join(format!("{session_id}.json")))
}

impl SessionStore {
    fn list_sessions_filtered(
        &self,
        app_data_dir: &Path,
        worktree_path: &str,
        predicate: impl Fn(&ChatSession) -> bool,
    ) -> Result<Vec<SessionSummary>, String> {
        self.ensure_loaded(app_data_dir)?;
        let cache = self.cache.read();
        let mut summaries: Vec<SessionSummary> = cache
            .values()
            .filter(|s| s.worktree_path == worktree_path && predicate(s))
            .map(|s| s.to_summary())
            .collect();
        summaries.sort_by(|a, b| {
            b.updated_at
                .partial_cmp(&a.updated_at)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(summaries)
    }

    pub fn list_sessions(
        &self,
        app_data_dir: &Path,
        worktree_path: &str,
    ) -> Result<Vec<SessionSummary>, String> {
        self.list_sessions_filtered(app_data_dir, worktree_path, |s| {
            s.state != SessionState::Closed
        })
    }

    pub fn list_closed_sessions(
        &self,
        app_data_dir: &Path,
        worktree_path: &str,
    ) -> Result<Vec<SessionSummary>, String> {
        self.list_sessions_filtered(app_data_dir, worktree_path, |s| {
            s.state == SessionState::Closed
        })
    }

    pub fn get_session(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<ChatSession>, String> {
        self.ensure_loaded(app_data_dir)?;
        let cache = self.cache.read();
        Ok(cache.get(session_id).cloned())
    }

    pub fn save_session(&self, app_data_dir: &Path, session: &ChatSession) -> Result<(), String> {
        let _lock = self.file_lock.lock();
        let dir = sessions_dir(app_data_dir);
        std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create sessions dir: {e}"))?;
        let file = session_file(app_data_dir, &session.id)?;
        let json = serde_json::to_string_pretty(session)
            .map_err(|e| format!("Failed to serialize session: {e}"))?;
        // Atomic write: write to .tmp then rename to avoid partial reads on crash
        let tmp_file = file.with_extension("json.tmp");
        std::fs::write(&tmp_file, json)
            .map_err(|e| format!("Failed to write session temp file: {e}"))?;
        std::fs::rename(&tmp_file, &file)
            .map_err(|e| format!("Failed to rename session temp file: {e}"))?;
        self.cache
            .write()
            .insert(session.id.clone(), session.clone());
        Ok(())
    }

    pub fn update_permission_mode(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        permission_mode: &str,
    ) -> Result<(), String> {
        self.ensure_loaded(app_data_dir)?;
        let mut session = {
            let cache = self.cache.read();
            cache
                .get(session_id)
                .cloned()
                .ok_or_else(|| format!("Session not found: {session_id}"))?
        };
        session.permission_mode = permission_mode.to_string();
        self.save_session(app_data_dir, &session)
    }

    fn ensure_loaded(&self, app_data_dir: &Path) -> Result<(), String> {
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
        for entry in entries {
            let entry = entry.map_err(|e| format!("Failed to read dir entry: {e}"))?;
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "json") {
                match std::fs::read_to_string(&path) {
                    Ok(content) => match serde_json::from_str::<ChatSession>(&content) {
                        Ok(session) => {
                            cache.insert(session.id.clone(), session);
                        }
                        Err(e) => {
                            log::warn!("Failed to parse session file {:?}: {e}", path);
                        }
                    },
                    Err(e) => {
                        log::warn!("Failed to read session file {:?}: {e}", path);
                    }
                }
            }
        }
        self.loaded.store(true, Ordering::Release);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{ChatMessage, MessageRole, SessionState};
    use tempfile::TempDir;

    const UUID1: &str = "a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d";
    const UUID2: &str = "b2c3d4e5-f6a7-4b8c-9d0e-1f2a3b4c5d6e";
    const UUID3: &str = "c3d4e5f6-a7b8-4c9d-ae0f-1a2b3c4d5e6f";

    fn make_session(id: &str, worktree: &str) -> ChatSession {
        ChatSession {
            id: id.to_string(),
            worktree_path: worktree.to_string(),
            messages: vec![ChatMessage {
                id: "m1".to_string(),
                role: MessageRole::Human,
                content: "Hello".to_string(),
                thinking: None,
                activities: None,
                parts: None,
                timestamp: 1000.0,
            }],
            state: SessionState::Active,
            created_at: 1000.0,
            updated_at: 1000.0,
            agent_session_id: None,
            permission_mode: "acceptEdits".to_string(),
        }
    }

    #[test]
    fn save_and_load_session() {
        let tmp = TempDir::new().unwrap();
        let store = SessionStore::default();
        let session = make_session(UUID1, "/repo");

        store.save_session(tmp.path(), &session).unwrap();

        let loaded = store.get_session(tmp.path(), UUID1).unwrap();
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.id, UUID1);
        assert_eq!(loaded.messages.len(), 1);
    }

    #[test]
    fn list_sessions_filters_by_worktree() {
        let tmp = TempDir::new().unwrap();
        let store = SessionStore::default();

        store
            .save_session(tmp.path(), &make_session(UUID1, "/repo-a"))
            .unwrap();
        store
            .save_session(tmp.path(), &make_session(UUID2, "/repo-b"))
            .unwrap();
        store
            .save_session(tmp.path(), &make_session(UUID3, "/repo-a"))
            .unwrap();

        let sessions = store.list_sessions(tmp.path(), "/repo-a").unwrap();
        assert_eq!(sessions.len(), 2);
        assert!(sessions.iter().all(|s| s.worktree_path == "/repo-a"));
    }

    #[test]
    fn get_nonexistent_session_returns_none() {
        let tmp = TempDir::new().unwrap();
        let store = SessionStore::default();
        let result = store.get_session(tmp.path(), UUID1).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn save_overwrites_existing_session() {
        let tmp = TempDir::new().unwrap();
        let store = SessionStore::default();
        let mut session = make_session(UUID1, "/repo");

        store.save_session(tmp.path(), &session).unwrap();

        session.messages.push(ChatMessage {
            id: "m2".to_string(),
            role: MessageRole::Agent,
            content: "Response".to_string(),
            thinking: None,
            activities: None,
            parts: None,
            timestamp: 1001.0,
        });
        session.updated_at = 1001.0;
        store.save_session(tmp.path(), &session).unwrap();

        let loaded = store.get_session(tmp.path(), UUID1).unwrap().unwrap();
        assert_eq!(loaded.messages.len(), 2);
    }

    #[test]
    fn list_sessions_sorted_by_updated_at_desc() {
        let tmp = TempDir::new().unwrap();
        let store = SessionStore::default();

        let mut s1 = make_session(UUID1, "/repo");
        s1.updated_at = 1000.0;
        let mut s2 = make_session(UUID2, "/repo");
        s2.updated_at = 2000.0;
        let mut s3 = make_session(UUID3, "/repo");
        s3.updated_at = 1500.0;

        store.save_session(tmp.path(), &s1).unwrap();
        store.save_session(tmp.path(), &s2).unwrap();
        store.save_session(tmp.path(), &s3).unwrap();

        let sessions = store.list_sessions(tmp.path(), "/repo").unwrap();
        assert_eq!(sessions[0].id, UUID2);
        assert_eq!(sessions[1].id, UUID3);
        assert_eq!(sessions[2].id, UUID1);
    }

    #[test]
    fn persistence_across_store_instances() {
        let tmp = TempDir::new().unwrap();
        let store1 = SessionStore::default();
        store1
            .save_session(tmp.path(), &make_session(UUID1, "/repo"))
            .unwrap();

        let store2 = SessionStore::default();
        let loaded = store2.get_session(tmp.path(), UUID1).unwrap();
        assert!(loaded.is_some());
    }

    #[test]
    fn session_file_validates_uuid() {
        let tmp = TempDir::new().unwrap();
        assert!(session_file(tmp.path(), UUID1).is_ok());
    }

    #[test]
    fn session_file_rejects_non_uuid() {
        let tmp = TempDir::new().unwrap();
        assert!(session_file(tmp.path(), "not-a-uuid").is_err());
    }

    #[test]
    fn session_file_rejects_path_traversal() {
        let tmp = TempDir::new().unwrap();
        assert!(session_file(tmp.path(), "../../../etc/passwd").is_err());
        assert!(session_file(tmp.path(), "..").is_err());
        assert!(session_file(tmp.path(), "foo/bar").is_err());
    }

    #[test]
    fn save_session_rejects_invalid_id() {
        let tmp = TempDir::new().unwrap();
        let store = SessionStore::default();
        let session = make_session("bad-id", "/repo");
        assert!(store.save_session(tmp.path(), &session).is_err());
    }

    #[test]
    fn list_sessions_excludes_closed() {
        let tmp = TempDir::new().unwrap();
        let store = SessionStore::default();

        store
            .save_session(tmp.path(), &make_session(UUID1, "/repo"))
            .unwrap();
        let mut closed = make_session(UUID2, "/repo");
        closed.state = SessionState::Closed;
        store.save_session(tmp.path(), &closed).unwrap();

        let sessions = store.list_sessions(tmp.path(), "/repo").unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, UUID1);
    }

    #[test]
    fn update_permission_mode_persists() {
        let tmp = TempDir::new().unwrap();
        let store = SessionStore::default();
        let session = make_session(UUID1, "/repo");
        assert_eq!(session.permission_mode, "acceptEdits");

        store.save_session(tmp.path(), &session).unwrap();
        store
            .update_permission_mode(tmp.path(), UUID1, "plan")
            .unwrap();

        let loaded = store.get_session(tmp.path(), UUID1).unwrap().unwrap();
        assert_eq!(loaded.permission_mode, "plan");
    }

    #[test]
    fn update_permission_mode_nonexistent_session_returns_error() {
        let tmp = TempDir::new().unwrap();
        let store = SessionStore::default();
        let result = store.update_permission_mode(tmp.path(), UUID1, "plan");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Session not found"));
    }

    #[test]
    fn list_closed_sessions_returns_only_closed() {
        let tmp = TempDir::new().unwrap();
        let store = SessionStore::default();

        store
            .save_session(tmp.path(), &make_session(UUID1, "/repo"))
            .unwrap();
        let mut closed1 = make_session(UUID2, "/repo");
        closed1.state = SessionState::Closed;
        closed1.updated_at = 2000.0;
        store.save_session(tmp.path(), &closed1).unwrap();
        let mut closed2 = make_session(UUID3, "/repo");
        closed2.state = SessionState::Closed;
        closed2.updated_at = 3000.0;
        store.save_session(tmp.path(), &closed2).unwrap();

        let closed = store.list_closed_sessions(tmp.path(), "/repo").unwrap();
        assert_eq!(closed.len(), 2);
        assert_eq!(closed[0].id, UUID3);
        assert_eq!(closed[1].id, UUID2);
    }
}
