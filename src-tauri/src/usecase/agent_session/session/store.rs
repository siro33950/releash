use parking_lot::RwLock;
use std::collections::HashMap;
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};

use super::{now_timestamp, ChatSession, SessionState, SessionSummary};

/// `SessionState` の遷移を観測する購読者向けコールバック。
/// 引数は `(session_id, worktree_path, new_state)`。
pub type SessionStateChangeListener =
    Arc<dyn Fn(&str, &str, &SessionState) + Send + Sync + 'static>;

pub struct SessionStore {
    cache: RwLock<HashMap<String, ChatSession>>,
    /// 壊れた / 旧形式の session JSON を session_id 単位で隔離する。
    /// Spec issues-947: 1つの不正セッションで全体ロードを Err にせず、無関係な正常セッションの
    /// 一覧取得・取得は素通しさせる。値は API に返す汎化済みエラー文言（フルパス・serde 生メッセージは含まない）。
    invalid_sessions: RwLock<HashMap<String, String>>,
    file_lock: parking_lot::Mutex<()>,
    loaded: AtomicBool,
    state_change_listeners: RwLock<Vec<SessionStateChangeListener>>,
}

impl Default for SessionStore {
    fn default() -> Self {
        Self {
            cache: RwLock::new(HashMap::new()),
            invalid_sessions: RwLock::new(HashMap::new()),
            file_lock: parking_lot::Mutex::new(()),
            loaded: AtomicBool::new(false),
            state_change_listeners: RwLock::new(Vec::new()),
        }
    }
}

fn sessions_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("sessions")
}

fn session_titles_file(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("session_titles.json")
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

/// API（Tauri invoke / WebSocket）に返す汎化済みエラー文言。
/// フルパスや serde の生メッセージは含めず、許可される抽象モード一覧のみを露出する
/// （Spec issues-947: リモート WS 応答経由でローカル app data パスが漏れないようにする）。
fn invalid_session_error_message_with_id(session_id: &str) -> String {
    format!(
        "Invalid session data (id={session_id}, allowed permission modes: {})",
        crate::permission::PermissionMode::allowed_list()
    )
}

fn invalid_session_error_message() -> String {
    format!(
        "Invalid session data (allowed permission modes: {})",
        crate::permission::PermissionMode::allowed_list()
    )
}

fn compact_session_title(title: &str) -> String {
    let compact = title.split_whitespace().collect::<Vec<_>>().join(" ");
    match compact.char_indices().nth(100) {
        Some((byte_pos, _)) => format!("{}…", &compact[..byte_pos]),
        None => compact,
    }
}

impl SessionStore {
    fn load_session_titles(&self, app_data_dir: &Path) -> Result<HashMap<String, String>, String> {
        let path = session_titles_file(app_data_dir);
        match std::fs::read_to_string(&path) {
            Ok(content) => {
                serde_json::from_str(&content).map_err(|_| invalid_session_error_message())
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(HashMap::new()),
            Err(e) => Err(format!("Failed to read session titles: {e}")),
        }
    }

    fn save_session_titles(
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

    fn remove_session_file_and_cache(&self, app_data_dir: &Path, session_id: &str) {
        if let Ok(file) = session_file(app_data_dir, session_id) {
            let _ = std::fs::remove_file(file);
        }
        self.cache.write().remove(session_id);
        self.invalid_sessions.write().remove(session_id);
    }

    fn apply_titles_to_summaries(
        &self,
        app_data_dir: &Path,
        summaries: &mut [SessionSummary],
    ) -> Result<(), String> {
        let titles = self.load_session_titles(app_data_dir)?;
        for summary in summaries {
            if let Some(title) = titles.get(&summary.id) {
                summary.first_message = title.clone();
            }
        }
        Ok(())
    }

    fn list_sessions_filtered(
        &self,
        app_data_dir: &Path,
        worktree_path: &str,
        predicate: impl Fn(&ChatSession) -> bool,
    ) -> Result<Vec<SessionSummary>, String> {
        self.ensure_loaded(app_data_dir)?;
        let cache = self.cache.read();
        let worktree_sessions: Vec<&ChatSession> = cache
            .values()
            .filter(|s| s.worktree_path == worktree_path && predicate(s))
            .collect();
        let mut summaries: Vec<SessionSummary> = worktree_sessions
            .into_iter()
            .map(ChatSession::to_summary)
            .collect();
        summaries.sort_by(|a, b| {
            b.updated_at
                .partial_cmp(&a.updated_at)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        self.apply_titles_to_summaries(app_data_dir, &mut summaries)?;
        Ok(summaries)
    }

    pub fn list_sessions(
        &self,
        app_data_dir: &Path,
        worktree_path: &str,
    ) -> Result<Vec<SessionSummary>, String> {
        self.list_sessions_filtered(app_data_dir, worktree_path, |s| {
            s.state != SessionState::Closed && s.state != SessionState::Archived
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

    pub fn archive_session(&self, app_data_dir: &Path, session_id: &str) -> Result<(), String> {
        let session = self
            .get_session(app_data_dir, session_id)?
            .ok_or_else(|| format!("Session not found: {session_id}"))?;
        if session.state != SessionState::Closed {
            return Err("Only closed sessions can be archived".to_string());
        }
        self.set_session_state(app_data_dir, session_id, SessionState::Archived)
    }

    pub fn archive_open_session(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<(), String> {
        let session = self
            .get_session(app_data_dir, session_id)?
            .ok_or_else(|| format!("Session not found: {session_id}"))?;
        if session.workflow_step_session {
            return Err("Workflow step sessions cannot be archived".to_string());
        }
        self.set_session_state(app_data_dir, session_id, SessionState::Archived)
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

    pub fn set_session_title(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        title: Option<&str>,
    ) -> Result<SessionSummary, String> {
        let session = self
            .get_session(app_data_dir, session_id)?
            .ok_or_else(|| format!("Session not found: {session_id}"))?;
        if session.workflow_step_session {
            return Err("Workflow step sessions cannot be renamed".to_string());
        }

        let title_for_summary = {
            let _lock = self.file_lock.lock();
            let mut titles = self.load_session_titles(app_data_dir)?;
            match title.map(compact_session_title) {
                Some(title) if !title.is_empty() => {
                    titles.insert(session_id.to_string(), title);
                }
                _ => {
                    titles.remove(session_id);
                }
            }
            self.save_session_titles(app_data_dir, &titles)?;
            titles.get(session_id).cloned()
        };

        let mut summary = session.to_summary();
        if let Some(title) = title_for_summary {
            summary.first_message = title;
        }
        Ok(summary)
    }

    pub fn fork_session(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<ChatSession, String> {
        let session = self
            .get_session(app_data_dir, session_id)?
            .ok_or_else(|| format!("Session not found: {session_id}"))?;
        if session.workflow_step_session {
            return Err("Workflow step sessions cannot be forked".to_string());
        }
        let now = now_timestamp();
        let forked = ChatSession {
            id: uuid::Uuid::new_v4().to_string(),
            worktree_path: session.worktree_path.clone(),
            messages: session.messages.clone(),
            state: SessionState::Idle,
            created_at: now,
            updated_at: now,
            agent_session_id: None,
            permission_mode: session.permission_mode.clone(),
            plan_mode: session.plan_mode,
            selected_model: session.selected_model.clone(),
            permission_profile_id: session.permission_profile_id.clone(),
            backend_id: session.backend_id.clone(),
            workflow_step_session: false,
        };
        self.save_session(app_data_dir, &forked)?;
        let title_result = {
            let _lock = self.file_lock.lock();
            let mut titles = self.load_session_titles(app_data_dir)?;
            if let Some(title) = titles.get(session_id).cloned() {
                titles.insert(forked.id.clone(), title);
                self.save_session_titles(app_data_dir, &titles)?;
            }
            Ok::<(), String>(())
        };
        if let Err(err) = title_result {
            self.remove_session_file_and_cache(app_data_dir, &forked.id);
            return Err(err);
        }
        Ok(forked)
    }

    pub fn get_session(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<ChatSession>, String> {
        self.ensure_loaded(app_data_dir)?;
        if let Some(err) = self.invalid_sessions.read().get(session_id) {
            return Err(err.clone());
        }
        let cache = self.cache.read();
        Ok(cache.get(session_id).cloned())
    }

    pub fn list_worktree_sessions(
        &self,
        app_data_dir: &Path,
        worktree_path: &str,
    ) -> Result<Vec<ChatSession>, String> {
        self.ensure_loaded(app_data_dir)?;
        let cache = self.cache.read();
        Ok(cache
            .values()
            .filter(|session| session.worktree_path == worktree_path)
            .cloned()
            .collect())
    }

    pub fn save_session(&self, app_data_dir: &Path, session: &ChatSession) -> Result<(), String> {
        // 保存層を AgentChat permission_mode の正典とする。
        // cache / ファイルへの書き込み前に検証・正規化し、legacy 入力を新規保存値へ漏らさない。
        let permission_mode = crate::permission::PermissionMode::parse(&session.permission_mode)
            .map_err(|e| e.to_string())?;
        let normalized_session;
        let session = if session.permission_mode == permission_mode.as_str() {
            session
        } else {
            normalized_session = {
                let mut session = session.clone();
                session.permission_mode = permission_mode.as_str().to_string();
                session
            };
            &normalized_session
        };
        // file_lock を保持したまま listener を同期実行すると、listener から
        // save_session / set_session_state などへ再入したときに parking_lot::Mutex の
        // 自己デッドロックが発生する。lock スコープは永続化と cache 更新までに限定し、
        // 通知に必要なデータを返してからスコープを抜けて listener を呼ぶ。
        let state_changed = self.persist_and_update_cache(app_data_dir, session)?;
        if state_changed {
            self.notify_state_change(&session.id, &session.worktree_path, &session.state);
        }
        Ok(())
    }

    fn persist_and_update_cache(
        &self,
        app_data_dir: &Path,
        session: &ChatSession,
    ) -> Result<bool, String> {
        let _lock = self.file_lock.lock();
        let dir = sessions_dir(app_data_dir);
        std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create sessions dir: {e}"))?;
        let file = session_file(app_data_dir, &session.id)?;
        // Atomic write: write to .tmp then rename to avoid partial reads on crash
        let tmp_file = file.with_extension("json.tmp");
        let write_result = (|| -> Result<(), String> {
            let file = std::fs::File::create(&tmp_file)
                .map_err(|e| format!("Failed to write session temp file: {e}"))?;
            let mut writer = BufWriter::new(file);
            serde_json::to_writer_pretty(&mut writer, session)
                .map_err(|e| format!("Failed to serialize session: {e}"))?;
            writer
                .flush()
                .map_err(|e| format!("Failed to flush session temp file: {e}"))?;
            Ok(())
        })();
        if let Err(err) = write_result {
            let _ = std::fs::remove_file(&tmp_file);
            return Err(err);
        }
        std::fs::rename(&tmp_file, &file)
            .map_err(|e| format!("Failed to rename session temp file: {e}"))?;
        let mut cache = self.cache.write();
        let state_changed = cache.get(&session.id).map(|p| &p.state) != Some(&session.state);
        cache.insert(session.id.clone(), session.clone());
        self.invalid_sessions.write().remove(&session.id);
        Ok(state_changed)
    }

    /// `SessionState` の遷移を購読するリスナーを登録する。
    /// 登録順に保存後に発火される。AgentStatusCenter のような中央管理が
    /// SessionStore からの状態変更を一方向に受け取るための入口。
    pub fn register_state_change_listener(&self, listener: SessionStateChangeListener) {
        self.state_change_listeners.write().push(listener);
    }

    fn notify_state_change(&self, session_id: &str, worktree_path: &str, new_state: &SessionState) {
        let listeners = self.state_change_listeners.read().clone();
        for listener in listeners {
            listener(session_id, worktree_path, new_state);
        }
    }

    pub fn set_session_state(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        state: SessionState,
    ) -> Result<(), String> {
        let mut session = self
            .get_session(app_data_dir, session_id)?
            .ok_or_else(|| format!("Session not found: {session_id}"))?;
        session.state = state;
        session.updated_at = crate::usecase::agent_session::session::now_timestamp();
        self.save_session(app_data_dir, &session)
    }

    pub fn update_permission_mode(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        permission_mode: &str,
    ) -> Result<(), String> {
        let permission_mode =
            crate::permission::PermissionMode::parse(permission_mode).map_err(|e| e.to_string())?;
        self.ensure_loaded(app_data_dir)?;
        if let Some(err) = self.invalid_sessions.read().get(session_id) {
            return Err(err.clone());
        }
        let mut session = {
            let cache = self.cache.read();
            cache
                .get(session_id)
                .cloned()
                .ok_or_else(|| format!("Session not found: {session_id}"))?
        };
        session.permission_mode = permission_mode.as_str().to_string();
        self.save_session(app_data_dir, &session)
    }

    pub fn update_plan_mode(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        plan_mode: bool,
    ) -> Result<(), String> {
        self.ensure_loaded(app_data_dir)?;
        if let Some(err) = self.invalid_sessions.read().get(session_id) {
            return Err(err.clone());
        }
        let mut session = {
            let cache = self.cache.read();
            cache
                .get(session_id)
                .cloned()
                .ok_or_else(|| format!("Session not found: {session_id}"))?
        };
        session.plan_mode = plan_mode;
        self.save_session(app_data_dir, &session)
    }

    pub fn update_permission_profile_id(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        permission_profile_id: Option<&str>,
    ) -> Result<(), String> {
        let profile_id = permission_profile_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| {
                if value.chars().any(char::is_control) {
                    Err("Permission profile id cannot contain control characters".to_string())
                } else {
                    Ok(value.to_string())
                }
            })
            .transpose()?;
        self.ensure_loaded(app_data_dir)?;
        if let Some(err) = self.invalid_sessions.read().get(session_id) {
            return Err(err.clone());
        }
        let mut session = {
            let cache = self.cache.read();
            cache
                .get(session_id)
                .cloned()
                .ok_or_else(|| format!("Session not found: {session_id}"))?
        };
        session.permission_profile_id = profile_id;
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
        let mut invalid_sessions = self.invalid_sessions.write();
        for entry in entries {
            let entry = entry.map_err(|e| format!("Failed to read dir entry: {e}"))?;
            let path = entry.path();
            if path.extension().is_none_or(|ext| ext != "json") {
                continue;
            }
            // ファイル名（UUID）から session_id を導出する。serde 失敗時の隔離キーとして使う。
            let file_session_id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .filter(|s| UUID_RE.is_match(s))
                .map(str::to_string);
            let Some(file_session_id) = file_session_id else {
                continue;
            };
            let file = match std::fs::File::open(&path) {
                Ok(file) => file,
                Err(e) => {
                    log::error!("Failed to read session file {:?}: {e}", path.display());
                    invalid_sessions.insert(file_session_id, invalid_session_error_message());
                    continue;
                }
            };
            match serde_json::from_reader::<_, ChatSession>(BufReader::new(file)) {
                Ok(session) => {
                    if session.id != file_session_id {
                        log::error!(
                            "Session id mismatch in session file {:?}: file id={}, json id={}",
                            path.display(),
                            file_session_id,
                            session.id
                        );
                        invalid_sessions.insert(
                            file_session_id.clone(),
                            invalid_session_error_message_with_id(&file_session_id),
                        );
                        continue;
                    }
                    let permission_mode =
                        match crate::permission::PermissionMode::parse(&session.permission_mode) {
                            Ok(permission_mode) => permission_mode,
                            Err(e) => {
                                log::error!(
                                    "Invalid permission_mode in session file {:?}: {e}",
                                    path.display()
                                );
                                invalid_sessions.insert(
                                    file_session_id.clone(),
                                    invalid_session_error_message_with_id(&file_session_id),
                                );
                                continue;
                            }
                        };
                    let mut session = session;
                    session.permission_mode = permission_mode.as_str().to_string();
                    cache.insert(session.id.clone(), session);
                }
                Err(e) => {
                    log::error!("Failed to parse session file {:?}: {e}", path.display());
                    invalid_sessions.insert(
                        file_session_id.clone(),
                        invalid_session_error_message_with_id(&file_session_id),
                    );
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
    use crate::usecase::agent_session::session::{ChatMessage, MessageRole, SessionState};
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
                mentions: None,
            }],
            state: SessionState::Active,
            created_at: 1000.0,
            updated_at: 1000.0,
            agent_session_id: None,
            permission_mode: "edit".to_string(),
            plan_mode: false,
            permission_profile_id: None,
            selected_model: None,
            backend_id: None,
            workflow_step_session: false,
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
    fn save_session_preserves_pretty_json_representation() {
        let tmp = TempDir::new().unwrap();
        let store = SessionStore::default();
        let session = make_session(UUID1, "/repo");

        store.save_session(tmp.path(), &session).unwrap();

        let saved = std::fs::read_to_string(session_file(tmp.path(), UUID1).unwrap()).unwrap();
        let expected = serde_json::to_string_pretty(&session).unwrap();
        assert_eq!(saved, expected);
    }

    #[test]
    fn save_and_load_session_with_backend_id() {
        let tmp = TempDir::new().unwrap();
        let store = SessionStore::default();
        let mut session = make_session(UUID1, "/repo");
        session.backend_id = Some("claude".to_string());

        store.save_session(tmp.path(), &session).unwrap();

        // Load from a fresh store to verify file persistence
        let store2 = SessionStore::default();
        let loaded = store2.get_session(tmp.path(), UUID1).unwrap().unwrap();
        assert_eq!(loaded.backend_id, Some("claude".to_string()));
    }

    #[test]
    fn save_and_load_session_with_none_backend_id() {
        let tmp = TempDir::new().unwrap();
        let store = SessionStore::default();
        let session = make_session(UUID1, "/repo");
        assert_eq!(session.backend_id, None);

        store.save_session(tmp.path(), &session).unwrap();

        let store2 = SessionStore::default();
        let loaded = store2.get_session(tmp.path(), UUID1).unwrap().unwrap();
        assert_eq!(loaded.backend_id, None);
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
            mentions: None,
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
    fn save_session_rejects_invalid_permission_mode() {
        // Spec issues-947: save_session は cache/ファイル書き込み前に PermissionMode::parse を通す。
        // 旧語彙・未知語彙・空文字は許可一覧付きエラーで拒否し、cache に残らない。
        let tmp = TempDir::new().unwrap();
        let store = SessionStore::default();
        let valid = make_session(UUID1, "/repo");
        store.save_session(tmp.path(), &valid).unwrap();

        for invalid in [
            "acceptEdits",
            "bypassPermissions",
            "plan",
            "default",
            "unknown",
            "",
        ] {
            let mut bad = make_session(UUID2, "/repo");
            bad.permission_mode = invalid.to_string();
            let err = store.save_session(tmp.path(), &bad).unwrap_err();
            assert!(
                err.contains("ask, edit, full"),
                "invalid '{invalid}' must include allowed list, got: {err}"
            );
            // cache に invalid なセッションが残らないこと。
            assert!(store.get_session(tmp.path(), UUID2).unwrap().is_none());
        }
        // 既存の valid セッションは破壊されない。
        let loaded = store.get_session(tmp.path(), UUID1).unwrap().unwrap();
        assert_eq!(loaded.permission_mode, "edit");
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
    fn list_sessions_excludes_archived() {
        let tmp = TempDir::new().unwrap();
        let store = SessionStore::default();

        store
            .save_session(tmp.path(), &make_session(UUID1, "/repo"))
            .unwrap();
        let mut archived = make_session(UUID2, "/repo");
        archived.state = SessionState::Archived;
        store.save_session(tmp.path(), &archived).unwrap();

        let sessions = store.list_sessions(tmp.path(), "/repo").unwrap();
        let closed = store.list_closed_sessions(tmp.path(), "/repo").unwrap();

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, UUID1);
        assert!(closed.is_empty());
    }

    #[test]
    fn archive_session_moves_closed_session_out_of_closed_history() {
        let tmp = TempDir::new().unwrap();
        let store = SessionStore::default();

        let mut closed = make_session(UUID1, "/repo");
        closed.state = SessionState::Closed;
        store.save_session(tmp.path(), &closed).unwrap();

        store.archive_session(tmp.path(), UUID1).unwrap();

        let saved = store.get_session(tmp.path(), UUID1).unwrap().unwrap();
        assert_eq!(saved.state, SessionState::Archived);
        assert!(store
            .list_closed_sessions(tmp.path(), "/repo")
            .unwrap()
            .is_empty());
    }

    #[test]
    fn archive_open_session_archives_active_session() {
        let tmp = TempDir::new().unwrap();
        let store = SessionStore::default();

        store
            .save_session(tmp.path(), &make_session(UUID1, "/repo"))
            .unwrap();

        store.archive_open_session(tmp.path(), UUID1).unwrap();

        let saved = store.get_session(tmp.path(), UUID1).unwrap().unwrap();
        assert_eq!(saved.state, SessionState::Archived);
        assert!(store.list_sessions(tmp.path(), "/repo").unwrap().is_empty());
        assert!(store
            .list_closed_sessions(tmp.path(), "/repo")
            .unwrap()
            .is_empty());
    }

    #[test]
    fn archive_open_session_rejects_workflow_step_sessions() {
        let tmp = TempDir::new().unwrap();
        let store = SessionStore::default();
        let mut session = make_session(UUID1, "/repo");
        session.workflow_step_session = true;
        store.save_session(tmp.path(), &session).unwrap();

        let err = store.archive_open_session(tmp.path(), UUID1).unwrap_err();

        assert_eq!(err, "Workflow step sessions cannot be archived");
    }

    #[test]
    fn set_session_title_overrides_summary_and_can_clear() {
        let tmp = TempDir::new().unwrap();
        let store = SessionStore::default();

        store
            .save_session(tmp.path(), &make_session(UUID1, "/repo"))
            .unwrap();

        let summary = store
            .set_session_title(tmp.path(), UUID1, Some("  Custom   title  "))
            .unwrap();

        assert_eq!(summary.first_message, "Custom title");
        let sessions = store.list_sessions(tmp.path(), "/repo").unwrap();
        assert_eq!(sessions[0].first_message, "Custom title");

        let summary = store.set_session_title(tmp.path(), UUID1, None).unwrap();

        assert_eq!(summary.first_message, "Hello");
        let sessions = store.list_sessions(tmp.path(), "/repo").unwrap();
        assert_eq!(sessions[0].first_message, "Hello");
    }

    #[test]
    fn set_session_title_rejects_workflow_step_sessions() {
        let tmp = TempDir::new().unwrap();
        let store = SessionStore::default();
        let mut session = make_session(UUID1, "/repo");
        session.workflow_step_session = true;
        store.save_session(tmp.path(), &session).unwrap();

        let err = store
            .set_session_title(tmp.path(), UUID1, Some("Step title"))
            .unwrap_err();

        assert_eq!(err, "Workflow step sessions cannot be renamed");
    }

    #[test]
    fn fork_session_creates_detached_copy() {
        let tmp = TempDir::new().unwrap();
        let store = SessionStore::default();

        let mut session = make_session(UUID1, "/repo");
        session.agent_session_id = Some("agent-session".to_string());
        session.selected_model = Some("claude-opus".to_string());
        session.backend_id = Some("claude".to_string());
        session.messages.push(ChatMessage {
            id: "m2".to_string(),
            role: MessageRole::Agent,
            content: "Response".to_string(),
            thinking: None,
            activities: None,
            parts: None,
            timestamp: 1001.0,
            mentions: None,
        });
        store.save_session(tmp.path(), &session).unwrap();

        let forked = store.fork_session(tmp.path(), UUID1).unwrap();

        assert_ne!(forked.id, UUID1);
        assert_eq!(forked.worktree_path, "/repo");
        assert_eq!(forked.state, SessionState::Idle);
        assert_eq!(forked.agent_session_id, None);
        assert_eq!(forked.permission_mode, "edit");
        assert_eq!(forked.selected_model, Some("claude-opus".to_string()));
        assert_eq!(forked.backend_id, Some("claude".to_string()));
        assert_eq!(
            forked
                .messages
                .iter()
                .map(|message| message.id.as_str())
                .collect::<Vec<_>>(),
            vec!["m1", "m2"]
        );
        assert!(store.get_session(tmp.path(), &forked.id).unwrap().is_some());
    }

    #[test]
    fn fork_session_copies_custom_title() {
        let tmp = TempDir::new().unwrap();
        let store = SessionStore::default();

        store
            .save_session(tmp.path(), &make_session(UUID1, "/repo"))
            .unwrap();
        store
            .set_session_title(tmp.path(), UUID1, Some("Custom title"))
            .unwrap();

        let forked = store.fork_session(tmp.path(), UUID1).unwrap();

        assert_eq!(
            store
                .session_title(tmp.path(), &forked.id)
                .unwrap()
                .as_deref(),
            Some("Custom title")
        );
    }

    #[test]
    fn fork_session_rejects_workflow_step_sessions() {
        let tmp = TempDir::new().unwrap();
        let store = SessionStore::default();
        let mut session = make_session(UUID1, "/repo");
        session.workflow_step_session = true;
        store.save_session(tmp.path(), &session).unwrap();

        let err = store.fork_session(tmp.path(), UUID1).unwrap_err();

        assert_eq!(err, "Workflow step sessions cannot be forked");
    }

    #[test]
    fn update_permission_mode_persists() {
        let tmp = TempDir::new().unwrap();
        let store = SessionStore::default();
        let session = make_session(UUID1, "/repo");
        assert_eq!(session.permission_mode, "edit");

        store.save_session(tmp.path(), &session).unwrap();
        store
            .update_permission_mode(tmp.path(), UUID1, "ask")
            .unwrap();

        let loaded = store.get_session(tmp.path(), UUID1).unwrap().unwrap();
        assert_eq!(loaded.permission_mode, "ask");
    }

    #[test]
    fn update_plan_mode_persists() {
        let tmp = TempDir::new().unwrap();
        let store = SessionStore::default();
        let session = make_session(UUID1, "/repo");
        assert!(!session.plan_mode);

        store.save_session(tmp.path(), &session).unwrap();
        store.update_plan_mode(tmp.path(), UUID1, true).unwrap();

        let loaded = store.get_session(tmp.path(), UUID1).unwrap().unwrap();
        assert!(loaded.plan_mode);
        let summary = loaded.to_summary();
        assert!(summary.plan_mode);
    }

    #[test]
    fn update_plan_mode_nonexistent_session_returns_error() {
        let tmp = TempDir::new().unwrap();
        let store = SessionStore::default();
        let result = store.update_plan_mode(tmp.path(), UUID1, true);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Session not found"));
    }

    #[test]
    fn update_permission_mode_nonexistent_session_returns_error() {
        let tmp = TempDir::new().unwrap();
        let store = SessionStore::default();
        let result = store.update_permission_mode(tmp.path(), UUID1, "ask");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Session not found"));
    }

    #[test]
    fn update_permission_mode_rejects_legacy_value() {
        let tmp = TempDir::new().unwrap();
        let store = SessionStore::default();
        let session = make_session(UUID1, "/repo");
        store.save_session(tmp.path(), &session).unwrap();
        for legacy in ["acceptEdits", "bypassPermissions", "plan", "default", ""] {
            let err = store
                .update_permission_mode(tmp.path(), UUID1, legacy)
                .unwrap_err();
            assert!(
                err.contains("ask, edit, full"),
                "legacy '{legacy}' must be rejected with allowed list, got: {err}"
            );
        }
        // Ensure the persisted value was not corrupted by failed attempts
        let loaded = store.get_session(tmp.path(), UUID1).unwrap().unwrap();
        assert_eq!(loaded.permission_mode, "edit");
    }

    fn write_session_json(dir: &Path, session_id: &str, json: &str) {
        let sessions = sessions_dir(dir);
        std::fs::create_dir_all(&sessions).unwrap();
        let file = sessions.join(format!("{session_id}.json"));
        std::fs::write(&file, json).unwrap();
    }

    fn session_json_with_permission(session_id: &str, permission_field: Option<&str>) -> String {
        let permission_segment = match permission_field {
            Some(value) => format!(",\"permissionMode\":\"{value}\""),
            None => String::new(),
        };
        format!(
            r#"{{"id":"{session_id}","worktreePath":"/repo","messages":[],"state":"active","createdAt":1000.0,"updatedAt":1000.0,"workflowStepSession":false{permission_segment}}}"#
        )
    }

    #[test]
    fn ensure_loaded_rejects_missing_permission_mode() {
        let tmp = TempDir::new().unwrap();
        write_session_json(
            tmp.path(),
            UUID1,
            &session_json_with_permission(UUID1, None),
        );
        let store = SessionStore::default();
        let err = store.get_session(tmp.path(), UUID1).unwrap_err();
        assert!(
            err.contains("ask, edit, full"),
            "missing permissionMode must be rejected with allowed list, got: {err}"
        );
    }

    #[test]
    fn ensure_loaded_rejects_legacy_and_unknown_permission_modes() {
        for invalid in [
            "acceptEdits",
            "bypassPermissions",
            "plan",
            "default",
            "unknown",
            "",
        ] {
            let tmp = TempDir::new().unwrap();
            write_session_json(
                tmp.path(),
                UUID1,
                &session_json_with_permission(UUID1, Some(invalid)),
            );
            let store = SessionStore::default();
            let err = store.get_session(tmp.path(), UUID1).unwrap_err();
            assert!(
                err.contains("ask, edit, full"),
                "invalid permissionMode '{invalid}' must be rejected with allowed list, got: {err}"
            );
        }
    }

    #[test]
    fn invalid_session_is_ignored_by_list_but_rejected_by_targeted_operations() {
        // Spec issues-947: 一覧では invalid session を隔離し、無関係な正常 session を返す。
        // 個別取得や更新では invalid session は汎化済みエラーを返す。
        let tmp = TempDir::new().unwrap();
        // valid session
        let store_for_save = SessionStore::default();
        store_for_save
            .save_session(tmp.path(), &make_session(UUID1, "/repo"))
            .unwrap();
        // invalid session（旧語彙を含む生 JSON を直接書き込み）
        write_session_json(
            tmp.path(),
            UUID2,
            &session_json_with_permission(UUID2, Some("acceptEdits")),
        );

        let store = SessionStore::default();

        let summaries = store.list_sessions(tmp.path(), "/repo").unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].id, UUID1);

        let sessions = store.list_worktree_sessions(tmp.path(), "/repo").unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, UUID1);

        // valid 単体取得は成功
        let loaded = store.get_session(tmp.path(), UUID1).unwrap();
        assert_eq!(loaded.unwrap().id, UUID1);

        // invalid 単体取得は許可一覧付きエラー（生パス・serde メッセージは含まない）
        let err = store.get_session(tmp.path(), UUID2).unwrap_err();
        assert!(err.contains("ask, edit, full"), "got: {err}");
        assert!(
            !err.contains(tmp.path().to_str().unwrap()),
            "path must not leak: {err}"
        );
        assert!(!err.contains(".json"), "filename must not leak: {err}");

        // update_permission_mode も invalid は弾く
        let err = store
            .update_permission_mode(tmp.path(), UUID2, "edit")
            .unwrap_err();
        assert!(err.contains("ask, edit, full"), "got: {err}");
    }

    #[test]
    fn invalid_session_isolation_key_uses_file_session_id_for_permission_errors() {
        let tmp = TempDir::new().unwrap();
        write_session_json(
            tmp.path(),
            UUID1,
            &session_json_with_permission(UUID2, Some("acceptEdits")),
        );
        let store = SessionStore::default();

        let err = store.get_session(tmp.path(), UUID1).unwrap_err();
        assert!(
            err.contains(UUID1),
            "file id must be the invalid key: {err}"
        );
        let by_json_id = store.get_session(tmp.path(), UUID2).unwrap();
        assert!(by_json_id.is_none());

        let listed = store.list_sessions(tmp.path(), "/repo").unwrap();
        assert!(listed.is_empty());
    }

    #[test]
    fn save_session_removes_stale_invalid_marker_for_same_id() {
        let tmp = TempDir::new().unwrap();
        write_session_json(
            tmp.path(),
            UUID1,
            &session_json_with_permission(UUID1, Some("acceptEdits")),
        );
        let store = SessionStore::default();
        let err = store.get_session(tmp.path(), UUID1).unwrap_err();
        assert!(err.contains("ask, edit, full"), "got: {err}");

        store
            .save_session(tmp.path(), &make_session(UUID1, "/repo"))
            .unwrap();

        let loaded = store.get_session(tmp.path(), UUID1).unwrap().unwrap();
        assert_eq!(loaded.permission_mode, "edit");
        let summaries = store.list_sessions(tmp.path(), "/repo").unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].id, UUID1);
    }

    #[test]
    fn ensure_loaded_normalizes_legacy_and_accepts_valid_permission_modes() {
        for (input, expected) in [
            ("readonly", "ask"),
            ("ask", "ask"),
            ("edit", "edit"),
            ("full", "full"),
        ] {
            let tmp = TempDir::new().unwrap();
            write_session_json(
                tmp.path(),
                UUID1,
                &session_json_with_permission(UUID1, Some(input)),
            );
            let store = SessionStore::default();
            let session = store
                .get_session(tmp.path(), UUID1)
                .unwrap()
                .expect("session loads with valid permission_mode");
            assert_eq!(session.permission_mode, expected);
        }
    }

    #[test]
    fn state_change_listener_fires_on_close_and_restore() {
        use parking_lot::Mutex as PlMutex;

        let tmp = TempDir::new().unwrap();
        let store = SessionStore::default();
        let session = make_session(UUID1, "/repo");
        store.save_session(tmp.path(), &session).unwrap();

        let events: Arc<PlMutex<Vec<(String, String, SessionState)>>> =
            Arc::new(PlMutex::new(Vec::new()));
        let events_for_listener = events.clone();
        store.register_state_change_listener(Arc::new(
            move |session_id, worktree_path, new_state| {
                events_for_listener.lock().push((
                    session_id.to_string(),
                    worktree_path.to_string(),
                    new_state.clone(),
                ));
            },
        ));

        // タブを閉じる: Active → Closed
        store
            .set_session_state(tmp.path(), UUID1, SessionState::Closed)
            .unwrap();
        // 復帰: Closed → Idle
        store
            .set_session_state(tmp.path(), UUID1, SessionState::Idle)
            .unwrap();

        let captured = events.lock().clone();
        assert_eq!(captured.len(), 2);
        assert_eq!(captured[0].0, UUID1);
        assert_eq!(captured[0].1, "/repo");
        assert_eq!(captured[0].2, SessionState::Closed);
        assert_eq!(captured[1].2, SessionState::Idle);
    }

    #[test]
    fn state_change_listener_does_not_fire_when_state_unchanged() {
        use parking_lot::Mutex as PlMutex;

        let tmp = TempDir::new().unwrap();
        let store = SessionStore::default();
        let session = make_session(UUID1, "/repo");
        store.save_session(tmp.path(), &session).unwrap();

        let count = Arc::new(PlMutex::new(0usize));
        let count_for_listener = count.clone();
        store.register_state_change_listener(Arc::new(move |_, _, _| {
            *count_for_listener.lock() += 1;
        }));

        // 状態は変えずに permission_mode を更新する（Spec issues-947 で抽象3値のみ受理）
        store
            .update_permission_mode(tmp.path(), UUID1, "ask")
            .unwrap();

        assert_eq!(*count.lock(), 0);
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
