use parking_lot::RwLock;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::protocol::thread::{Thread, ThreadEntry};
use crate::protocol::CommentItem;

#[derive(Clone, serde::Serialize)]
pub struct ThreadsChangedPayload {
    pub worktree_name: String,
    pub source: String,
    pub threads: Vec<Thread>,
}

pub struct ThreadStore {
    entries: RwLock<HashMap<String, Vec<Thread>>>,
    file_lock: parking_lot::Mutex<()>,
}

impl Default for ThreadStore {
    fn default() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
            file_lock: parking_lot::Mutex::new(()),
        }
    }
}

fn threads_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("threads")
}

fn threads_file(app_data_dir: &Path, worktree_name: &str) -> PathBuf {
    let safe_name = worktree_name.replace(['/', '\\'], "_");
    threads_dir(app_data_dir).join(format!("{safe_name}.json"))
}

fn old_comments_file(app_data_dir: &Path, worktree_name: &str) -> PathBuf {
    let safe_name = worktree_name.replace(['/', '\\'], "_");
    app_data_dir
        .join("comments")
        .join(format!("{safe_name}.json"))
}

/// Migrate old CommentItem[] JSON to Thread[]
fn migrate_comments_to_threads(comments: Vec<CommentItem>, worktree_name: &str) -> Vec<Thread> {
    // Group by parentId: root comments (no parentId) become threads,
    // child comments become entries within parent thread
    let mut root_comments: Vec<CommentItem> = Vec::new();
    let mut children: HashMap<String, Vec<CommentItem>> = HashMap::new();

    for mut comment in comments {
        // Normalize file path
        let prefix = format!("{}/", worktree_name);
        if let Some(stripped) = comment.file_path.strip_prefix(&prefix) {
            comment.file_path = stripped.to_string();
        }

        if let Some(ref parent_id) = comment.parent_id {
            children.entry(parent_id.clone()).or_default().push(comment);
        } else {
            root_comments.push(comment);
        }
    }

    root_comments
        .into_iter()
        .map(|root| {
            let mut entries = vec![ThreadEntry {
                id: format!("{}-e0", root.id),
                content: root.content.clone(),
                action: None,
                author_name: None,
                author_avatar_url: None,
                pr_comment_id: None,
                created_at: root.created_at,
            }];

            // Add child comments as additional entries
            if let Some(mut child_list) = children.remove(&root.id) {
                child_list.sort_by(|a, b| {
                    a.created_at
                        .partial_cmp(&b.created_at)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                for (i, child) in child_list.into_iter().enumerate() {
                    entries.push(ThreadEntry {
                        id: format!("{}-e{}", root.id, i + 1),
                        content: child.content,
                        action: None,
                        author_name: None,
                        author_avatar_url: None,
                        pr_comment_id: None,
                        created_at: child.created_at,
                    });
                }
            }

            Thread {
                id: root.id,
                file_path: root.file_path,
                line_number: root.line_number,
                end_line: root.end_line,
                entries,
                resolved: root.resolved,
                severity: root.severity,
                anchor: None,
                created_at: root.created_at,
            }
        })
        .collect()
}

/// Normalize and validate a thread's file_path.
/// Strips the worktree prefix if present, rejects absolute paths and `..` traversal.
fn normalize_file_path(file_path: &str, worktree_name: &str) -> Result<String, String> {
    let prefix = format!("{}/", worktree_name);
    let path = file_path.strip_prefix(&prefix).unwrap_or(file_path);

    if path.starts_with('/') || path.starts_with('\\') {
        return Err(format!("Absolute path not allowed: {path}"));
    }

    for component in Path::new(path).components() {
        if matches!(component, std::path::Component::ParentDir) {
            return Err(format!("Path traversal not allowed: {path}"));
        }
    }

    Ok(path.to_string())
}

impl ThreadStore {
    pub fn load(&self, app_data_dir: &Path, worktree_name: &str) -> Result<Vec<Thread>, String> {
        let file_path = threads_file(app_data_dir, worktree_name);

        if file_path.exists() {
            let data =
                std::fs::read_to_string(&file_path).map_err(|e| format!("Failed to read: {e}"))?;
            let threads: Vec<Thread> =
                serde_json::from_str(&data).map_err(|e| format!("Failed to parse threads: {e}"))?;
            self.entries
                .write()
                .insert(worktree_name.to_string(), threads.clone());
            return Ok(threads);
        }

        // Migration: try loading old comments format
        let old_path = old_comments_file(app_data_dir, worktree_name);
        if old_path.exists() {
            let data = std::fs::read_to_string(&old_path)
                .map_err(|e| format!("Failed to read old comments: {e}"))?;
            let comments: Vec<CommentItem> = serde_json::from_str(&data)
                .map_err(|e| format!("Failed to parse old comments: {e}"))?;
            let threads = migrate_comments_to_threads(comments, worktree_name);
            self.entries
                .write()
                .insert(worktree_name.to_string(), threads.clone());
            // Save migrated data in new format
            self.save(app_data_dir, worktree_name)?;
            return Ok(threads);
        }

        Ok(vec![])
    }

    pub fn save(&self, app_data_dir: &Path, worktree_name: &str) -> Result<(), String> {
        let _guard = self.file_lock.lock();

        let dir = threads_dir(app_data_dir);
        std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create dir: {e}"))?;

        let file_path = threads_file(app_data_dir, worktree_name);
        let threads = {
            let entries = self.entries.read();
            entries.get(worktree_name).cloned().unwrap_or_default()
        };
        let json = serde_json::to_string_pretty(&threads)
            .map_err(|e| format!("Failed to serialize: {e}"))?;
        std::fs::write(&file_path, json).map_err(|e| format!("Failed to write: {e}"))?;
        Ok(())
    }

    pub fn add_thread(&self, worktree_name: &str, mut thread: Thread) -> Result<(), String> {
        thread.file_path = normalize_file_path(&thread.file_path, worktree_name)?;
        let mut entries = self.entries.write();
        entries
            .entry(worktree_name.to_string())
            .or_default()
            .push(thread);
        Ok(())
    }

    pub fn add_entry(&self, worktree_name: &str, thread_id: &str, entry: ThreadEntry) -> bool {
        let mut entries = self.entries.write();
        if let Some(threads) = entries.get_mut(worktree_name) {
            if let Some(thread) = threads.iter_mut().find(|t| t.id == thread_id) {
                thread.entries.push(entry);
                return true;
            }
        }
        false
    }

    pub fn remove_thread(&self, worktree_name: &str, thread_id: &str) -> bool {
        let mut entries = self.entries.write();
        if let Some(threads) = entries.get_mut(worktree_name) {
            let before = threads.len();
            threads.retain(|t| t.id != thread_id);
            return threads.len() < before;
        }
        false
    }

    pub fn update_entry(
        &self,
        worktree_name: &str,
        thread_id: &str,
        entry_id: &str,
        content: &str,
    ) -> bool {
        let mut entries = self.entries.write();
        if let Some(threads) = entries.get_mut(worktree_name) {
            if let Some(thread) = threads.iter_mut().find(|t| t.id == thread_id) {
                if let Some(entry) = thread.entries.iter_mut().find(|e| e.id == entry_id) {
                    entry.content = content.to_string();
                    return true;
                }
            }
        }
        false
    }

    pub fn resolve_thread(&self, worktree_name: &str, thread_id: &str) -> Option<bool> {
        let mut entries = self.entries.write();
        if let Some(threads) = entries.get_mut(worktree_name) {
            if let Some(thread) = threads.iter_mut().find(|t| t.id == thread_id) {
                thread.resolved = !thread.resolved;
                return Some(thread.resolved);
            }
        }
        None
    }

    pub fn set_all(&self, worktree_name: &str, mut threads: Vec<Thread>) -> Result<(), String> {
        for thread in &mut threads {
            thread.file_path = normalize_file_path(&thread.file_path, worktree_name)?;
        }
        self.entries
            .write()
            .insert(worktree_name.to_string(), threads);
        Ok(())
    }

    pub fn get_all(&self, worktree_name: &str) -> Vec<Thread> {
        let entries = self.entries.read();
        entries.get(worktree_name).cloned().unwrap_or_default()
    }

    pub fn get_thread(&self, worktree_name: &str, thread_id: &str) -> Option<Thread> {
        let entries = self.entries.read();
        entries
            .get(worktree_name)?
            .iter()
            .find(|t| t.id == thread_id)
            .cloned()
    }

    pub fn get_filtered(
        &self,
        worktree_name: &str,
        file_path: Option<&str>,
        severity: Option<&str>,
        resolved: Option<bool>,
    ) -> Vec<Thread> {
        let entries = self.entries.read();
        let threads = match entries.get(worktree_name) {
            Some(t) => t,
            None => return Vec::new(),
        };
        threads
            .iter()
            .filter(|t| {
                if let Some(fp) = file_path {
                    if t.file_path != fp {
                        return false;
                    }
                }
                if let Some(s) = severity {
                    if t.severity.as_deref() != Some(s) {
                        return false;
                    }
                }
                if let Some(r) = resolved {
                    if t.resolved != r {
                        return false;
                    }
                }
                true
            })
            .cloned()
            .collect()
    }

    pub fn update_thread(&self, worktree_name: &str, mut updated: Thread) -> Result<bool, String> {
        updated.file_path = normalize_file_path(&updated.file_path, worktree_name)?;
        let mut entries = self.entries.write();
        if let Some(threads) = entries.get_mut(worktree_name) {
            if let Some(thread) = threads.iter_mut().find(|t| t.id == updated.id) {
                *thread = updated;
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn cleanup(&self, app_data_dir: &Path, worktree_name: &str) -> Result<(), String> {
        self.entries.write().remove(worktree_name);
        let file_path = threads_file(app_data_dir, worktree_name);
        if file_path.exists() {
            std::fs::remove_file(&file_path).map_err(|e| format!("Failed to remove: {e}"))?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn persist_and_notify(
    app: &tauri::AppHandle,
    store: &ThreadStore,
    worktree_name: &str,
    source: &str,
) -> Result<Vec<Thread>, String> {
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {e}"))?;
    store.save(&data_dir, worktree_name)?;
    let threads = store.get_all(worktree_name);
    let _ = app.emit(
        "threads-changed",
        ThreadsChangedPayload {
            worktree_name: worktree_name.to_string(),
            source: source.to_string(),
            threads: threads.clone(),
        },
    );
    Ok(threads)
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

use tauri::Emitter;
use tauri::Manager;

#[tauri::command]
pub fn load_threads(
    app: tauri::AppHandle,
    store: tauri::State<'_, Arc<ThreadStore>>,
    worktree_name: String,
) -> Result<Vec<Thread>, String> {
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {e}"))?;
    store.load(&data_dir, &worktree_name)
}

#[tauri::command]
pub fn save_threads(
    app: tauri::AppHandle,
    store: tauri::State<'_, Arc<ThreadStore>>,
    worktree_name: String,
    threads: Vec<Thread>,
) -> Result<(), String> {
    store.set_all(&worktree_name, threads)?;
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {e}"))?;
    store.save(&data_dir, &worktree_name)
}

#[tauri::command]
pub fn cleanup_threads(
    app: tauri::AppHandle,
    store: tauri::State<'_, Arc<ThreadStore>>,
    worktree_name: String,
) -> Result<(), String> {
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {e}"))?;
    store.cleanup(&data_dir, &worktree_name)
}

#[tauri::command]
pub fn add_thread(
    app: tauri::AppHandle,
    store: tauri::State<'_, Arc<ThreadStore>>,
    worktree_name: String,
    thread: Thread,
    source: String,
) -> Result<Vec<Thread>, String> {
    store.add_thread(&worktree_name, thread)?;
    persist_and_notify(&app, &store, &worktree_name, &source)
}

#[tauri::command]
pub fn add_thread_entry(
    app: tauri::AppHandle,
    store: tauri::State<'_, Arc<ThreadStore>>,
    worktree_name: String,
    thread_id: String,
    entry: ThreadEntry,
    source: String,
) -> Result<Vec<Thread>, String> {
    if !store.add_entry(&worktree_name, &thread_id, entry) {
        return Err("Thread not found".into());
    }
    persist_and_notify(&app, &store, &worktree_name, &source)
}

#[tauri::command]
pub fn remove_thread(
    app: tauri::AppHandle,
    store: tauri::State<'_, Arc<ThreadStore>>,
    worktree_name: String,
    thread_id: String,
    source: String,
) -> Result<Vec<Thread>, String> {
    if !store.remove_thread(&worktree_name, &thread_id) {
        return Err("Thread not found".into());
    }
    persist_and_notify(&app, &store, &worktree_name, &source)
}

#[tauri::command]
pub fn update_thread_entry_content(
    app: tauri::AppHandle,
    store: tauri::State<'_, Arc<ThreadStore>>,
    worktree_name: String,
    thread_id: String,
    entry_id: String,
    content: String,
    source: String,
) -> Result<Vec<Thread>, String> {
    if !store.update_entry(&worktree_name, &thread_id, &entry_id, &content) {
        return Err("Entry not found".into());
    }
    persist_and_notify(&app, &store, &worktree_name, &source)
}

#[tauri::command]
pub fn update_thread(
    app: tauri::AppHandle,
    store: tauri::State<'_, Arc<ThreadStore>>,
    worktree_name: String,
    thread: Thread,
    source: String,
) -> Result<Vec<Thread>, String> {
    if !store.update_thread(&worktree_name, thread)? {
        return Err("Thread not found".into());
    }
    persist_and_notify(&app, &store, &worktree_name, &source)
}

#[tauri::command]
pub fn toggle_resolve_thread(
    app: tauri::AppHandle,
    store: tauri::State<'_, Arc<ThreadStore>>,
    worktree_name: String,
    thread_id: String,
    source: String,
) -> Result<Vec<Thread>, String> {
    store
        .resolve_thread(&worktree_name, &thread_id)
        .ok_or_else(|| "Thread not found".to_string())?;
    persist_and_notify(&app, &store, &worktree_name, &source)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_thread(id: &str, file_path: &str, content: &str) -> Thread {
        Thread {
            id: id.to_string(),
            file_path: file_path.to_string(),
            line_number: 10,
            end_line: None,
            entries: vec![ThreadEntry {
                id: format!("{id}-e0"),
                content: content.to_string(),
                action: None,
                author_name: None,
                author_avatar_url: None,
                pr_comment_id: None,
                created_at: 1234567890.0,
            }],
            resolved: false,
            severity: None,
            anchor: None,
            created_at: 1234567890.0,
        }
    }

    fn make_entry(id: &str, content: &str) -> ThreadEntry {
        ThreadEntry {
            id: id.to_string(),
            content: content.to_string(),
            action: None,
            author_name: None,
            author_avatar_url: None,
            pr_comment_id: None,
            created_at: 1234567891.0,
        }
    }

    #[test]
    fn add_and_get_all() {
        let store = ThreadStore::default();
        store
            .add_thread("wt1", make_thread("t1", "file.rs", "comment 1"))
            .unwrap();
        store
            .add_thread("wt1", make_thread("t2", "file.rs", "comment 2"))
            .unwrap();
        let all = store.get_all("wt1");
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn add_entry_to_thread() {
        let store = ThreadStore::default();
        store
            .add_thread("wt1", make_thread("t1", "file.rs", "original"))
            .unwrap();
        assert!(store.add_entry("wt1", "t1", make_entry("e1", "reply")));
        let all = store.get_all("wt1");
        assert_eq!(all[0].entries.len(), 2);
        assert_eq!(all[0].entries[1].content, "reply");
    }

    #[test]
    fn add_entry_to_nonexistent_thread() {
        let store = ThreadStore::default();
        assert!(!store.add_entry("wt1", "nonexistent", make_entry("e1", "reply")));
    }

    #[test]
    fn remove_thread() {
        let store = ThreadStore::default();
        store
            .add_thread("wt1", make_thread("t1", "file.rs", "comment"))
            .unwrap();
        assert!(store.remove_thread("wt1", "t1"));
        assert!(!store.remove_thread("wt1", "t1"));
        assert_eq!(store.get_all("wt1").len(), 0);
    }

    #[test]
    fn update_entry() {
        let store = ThreadStore::default();
        store
            .add_thread("wt1", make_thread("t1", "file.rs", "original"))
            .unwrap();
        assert!(store.update_entry("wt1", "t1", "t1-e0", "updated"));
        assert_eq!(store.get_all("wt1")[0].entries[0].content, "updated");
    }

    #[test]
    fn update_entry_nonexistent() {
        let store = ThreadStore::default();
        store
            .add_thread("wt1", make_thread("t1", "file.rs", "original"))
            .unwrap();
        assert!(!store.update_entry("wt1", "t1", "nonexistent", "updated"));
        assert!(!store.update_entry("wt1", "nonexistent", "e0", "updated"));
    }

    #[test]
    fn resolve_toggles() {
        let store = ThreadStore::default();
        store
            .add_thread("wt1", make_thread("t1", "file.rs", "comment"))
            .unwrap();
        assert_eq!(store.resolve_thread("wt1", "t1"), Some(true));
        assert_eq!(store.resolve_thread("wt1", "t1"), Some(false));
        assert_eq!(store.resolve_thread("wt1", "nonexistent"), None);
    }

    #[test]
    fn save_and_load() {
        let dir = TempDir::new().unwrap();
        let store = ThreadStore::default();
        store
            .add_thread("wt1", make_thread("t1", "file.rs", "comment"))
            .unwrap();
        store.save(dir.path(), "wt1").unwrap();

        let store2 = ThreadStore::default();
        let loaded = store2.load(dir.path(), "wt1").unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "t1");
        assert_eq!(loaded[0].entries[0].content, "comment");
    }

    #[test]
    fn load_nonexistent_returns_empty() {
        let dir = TempDir::new().unwrap();
        let store = ThreadStore::default();
        let loaded = store.load(dir.path(), "nonexistent").unwrap();
        assert!(loaded.is_empty());
    }

    #[test]
    fn cleanup_removes_file_and_memory() {
        let dir = TempDir::new().unwrap();
        let store = ThreadStore::default();
        store
            .add_thread("wt1", make_thread("t1", "file.rs", "comment"))
            .unwrap();
        store.save(dir.path(), "wt1").unwrap();

        store.cleanup(dir.path(), "wt1").unwrap();
        assert!(store.get_all("wt1").is_empty());
        assert!(!threads_file(dir.path(), "wt1").exists());
    }

    #[test]
    fn set_all_replaces() {
        let store = ThreadStore::default();
        store
            .add_thread("wt1", make_thread("t1", "file.rs", "first"))
            .unwrap();
        store
            .set_all("wt1", vec![make_thread("t2", "other.rs", "second")])
            .unwrap();
        let all = store.get_all("wt1");
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, "t2");
    }

    #[test]
    fn add_normalizes_absolute_path() {
        let store = ThreadStore::default();
        store
            .add_thread(
                "/Users/dev/project",
                make_thread("t1", "/Users/dev/project/src/main.rs", "comment"),
            )
            .unwrap();
        let all = store.get_all("/Users/dev/project");
        assert_eq!(all[0].file_path, "src/main.rs");
    }

    #[test]
    fn add_rejects_path_traversal() {
        let store = ThreadStore::default();
        assert!(store
            .add_thread("wt1", make_thread("t1", "../etc/passwd", "bad"))
            .is_err());
        assert!(store
            .add_thread("wt1", make_thread("t2", "src/../../etc/passwd", "bad"))
            .is_err());
    }

    #[test]
    fn add_rejects_absolute_path() {
        let store = ThreadStore::default();
        assert!(store
            .add_thread("wt1", make_thread("t1", "/etc/passwd", "bad"))
            .is_err());
    }

    #[test]
    fn migration_from_old_comments_format() {
        let dir = TempDir::new().unwrap();
        let old_json = r#"[
            {
                "id": "c1",
                "file_path": "src/main.rs",
                "line_number": 10,
                "content": "root comment",
                "status": "unsent",
                "created_at": 1000.0,
                "resolved": false,
                "target": "local"
            },
            {
                "id": "c2",
                "file_path": "src/main.rs",
                "line_number": 10,
                "content": "reply to root",
                "status": "unsent",
                "created_at": 2000.0,
                "parent_id": "c1",
                "resolved": false,
                "target": "local"
            },
            {
                "id": "c3",
                "file_path": "src/lib.rs",
                "line_number": 5,
                "content": "another root",
                "status": "sent",
                "created_at": 3000.0,
                "resolved": false,
                "target": "local"
            }
        ]"#;
        let comments_dir = dir.path().join("comments");
        std::fs::create_dir_all(&comments_dir).unwrap();
        std::fs::write(comments_dir.join("wt1.json"), old_json).unwrap();

        let store = ThreadStore::default();
        let threads = store.load(dir.path(), "wt1").unwrap();

        // Should produce 2 threads: one from c1 (with c2 as reply), one from c3
        assert_eq!(threads.len(), 2);

        let t1 = threads.iter().find(|t| t.id == "c1").unwrap();
        assert_eq!(t1.entries.len(), 2);
        assert_eq!(t1.entries[0].content, "root comment");
        assert_eq!(t1.entries[1].content, "reply to root");

        let t2 = threads.iter().find(|t| t.id == "c3").unwrap();
        assert_eq!(t2.entries.len(), 1);
        assert_eq!(t2.entries[0].content, "another root");

        // Verify new format was saved
        assert!(threads_file(dir.path(), "wt1").exists());
    }

    #[test]
    fn get_thread_by_id() {
        let store = ThreadStore::default();
        store
            .add_thread("wt1", make_thread("t1", "a.rs", "first"))
            .unwrap();
        store
            .add_thread("wt1", make_thread("t2", "b.rs", "second"))
            .unwrap();
        let thread = store.get_thread("wt1", "t1").unwrap();
        assert_eq!(thread.id, "t1");
        assert_eq!(thread.entries[0].content, "first");
    }

    #[test]
    fn get_thread_not_found() {
        let store = ThreadStore::default();
        store
            .add_thread("wt1", make_thread("t1", "a.rs", "first"))
            .unwrap();
        assert!(store.get_thread("wt1", "nonexistent").is_none());
        assert!(store.get_thread("nonexistent", "t1").is_none());
    }

    #[test]
    fn get_filtered_by_file_path() {
        let store = ThreadStore::default();
        store
            .add_thread("wt1", make_thread("t1", "a.rs", "aaa"))
            .unwrap();
        store
            .add_thread("wt1", make_thread("t2", "b.rs", "bbb"))
            .unwrap();
        let result = store.get_filtered("wt1", Some("a.rs"), None, None);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "t1");
    }

    #[test]
    fn get_filtered_by_severity() {
        let store = ThreadStore::default();
        let mut t = make_thread("t1", "a.rs", "warning");
        t.severity = Some("warning".to_string());
        store.add_thread("wt1", t).unwrap();
        store
            .add_thread("wt1", make_thread("t2", "a.rs", "no severity"))
            .unwrap();
        let result = store.get_filtered("wt1", None, Some("warning"), None);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "t1");
    }

    #[test]
    fn get_filtered_by_resolved() {
        let store = ThreadStore::default();
        store
            .add_thread("wt1", make_thread("t1", "a.rs", "open"))
            .unwrap();
        store
            .add_thread("wt1", make_thread("t2", "a.rs", "resolved"))
            .unwrap();
        store.resolve_thread("wt1", "t2");
        let result = store.get_filtered("wt1", None, None, Some(false));
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "t1");
    }

    #[test]
    fn get_filtered_nonexistent_worktree() {
        let store = ThreadStore::default();
        let result = store.get_filtered("nonexistent", None, None, None);
        assert!(result.is_empty());
    }

    #[test]
    fn migration_fills_defaults() {
        let dir = TempDir::new().unwrap();
        let old_json = r#"[{
            "id": "c1",
            "file_path": "file.rs",
            "line_number": 10,
            "content": "old comment",
            "status": "unsent",
            "created_at": 1234567890.0
        }]"#;
        let comments_dir = dir.path().join("comments");
        std::fs::create_dir_all(&comments_dir).unwrap();
        std::fs::write(comments_dir.join("wt1.json"), old_json).unwrap();

        let store = ThreadStore::default();
        let threads = store.load(dir.path(), "wt1").unwrap();
        assert_eq!(threads.len(), 1);
        assert!(!threads[0].resolved);
        assert_eq!(threads[0].entries[0].content, "old comment");
    }
}
