use parking_lot::RwLock;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::protocol::CommentItem;

#[derive(Clone, serde::Serialize)]
pub struct CommentsChangedPayload {
    pub worktree_name: String,
    pub source: String,
}

pub struct CommentStore {
    entries: RwLock<HashMap<String, Vec<CommentItem>>>,
}

impl Default for CommentStore {
    fn default() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
        }
    }
}

fn comments_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("comments")
}

fn comments_file(app_data_dir: &Path, worktree_name: &str) -> PathBuf {
    let safe_name = worktree_name.replace(['/', '\\'], "_");
    comments_dir(app_data_dir).join(format!("{safe_name}.json"))
}

impl CommentStore {
    pub fn load(
        &self,
        app_data_dir: &Path,
        worktree_name: &str,
    ) -> Result<Vec<CommentItem>, String> {
        let file_path = comments_file(app_data_dir, worktree_name);
        if !file_path.exists() {
            return Ok(vec![]);
        }

        let data =
            std::fs::read_to_string(&file_path).map_err(|e| format!("Failed to read: {e}"))?;

        let mut items: Vec<CommentItem> =
            serde_json::from_str(&data).map_err(|e| format!("Failed to parse: {e}"))?;

        // Migration: normalize absolute file_path to relative
        let prefix = format!("{}/", worktree_name);
        for item in &mut items {
            if let Some(stripped) = item.file_path.strip_prefix(&prefix) {
                item.file_path = stripped.to_string();
            }
        }

        // Migration: fill defaults for old data missing new fields
        for item in &mut items {
            if item.target.is_empty() {
                item.target = "local".to_string();
            }
        }

        self.entries
            .write()
            .insert(worktree_name.to_string(), items.clone());
        Ok(items)
    }

    pub fn save(&self, app_data_dir: &Path, worktree_name: &str) -> Result<(), String> {
        let dir = comments_dir(app_data_dir);
        std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create dir: {e}"))?;

        let file_path = comments_file(app_data_dir, worktree_name);
        let entries = self.entries.read();
        let items = entries.get(worktree_name).cloned().unwrap_or_default();
        let json = serde_json::to_string_pretty(&items)
            .map_err(|e| format!("Failed to serialize: {e}"))?;
        std::fs::write(&file_path, json).map_err(|e| format!("Failed to write: {e}"))?;
        Ok(())
    }

    pub fn add(&self, worktree_name: &str, mut comment: CommentItem) {
        // Normalize absolute file_path to relative (strip worktree prefix)
        let prefix = format!("{}/", worktree_name);
        if let Some(stripped) = comment.file_path.strip_prefix(&prefix) {
            comment.file_path = stripped.to_string();
        }
        let mut entries = self.entries.write();
        entries
            .entry(worktree_name.to_string())
            .or_default()
            .push(comment);
    }

    pub fn mark_sent(&self, worktree_name: &str, ids: &[String]) {
        let mut entries = self.entries.write();
        if let Some(comments) = entries.get_mut(worktree_name) {
            for c in comments.iter_mut() {
                if ids.contains(&c.id) {
                    c.status = "sent".to_string();
                }
            }
        }
    }

    pub fn remove(&self, worktree_name: &str, id: &str) -> bool {
        let mut entries = self.entries.write();
        if let Some(comments) = entries.get_mut(worktree_name) {
            let before = comments.len();
            comments.retain(|c| c.id != id);
            return comments.len() < before;
        }
        false
    }

    pub fn update(&self, worktree_name: &str, id: &str, content: &str) -> bool {
        let mut entries = self.entries.write();
        if let Some(comments) = entries.get_mut(worktree_name) {
            if let Some(c) = comments.iter_mut().find(|c| c.id == id) {
                c.content = content.to_string();
                return true;
            }
        }
        false
    }

    pub fn resolve(&self, worktree_name: &str, id: &str) -> Option<bool> {
        let mut entries = self.entries.write();
        if let Some(comments) = entries.get_mut(worktree_name) {
            if let Some(c) = comments.iter_mut().find(|c| c.id == id) {
                c.resolved = !c.resolved;
                return Some(c.resolved);
            }
        }
        None
    }

    #[cfg(test)]
    pub fn get_all(&self, worktree_name: &str) -> Vec<CommentItem> {
        let entries = self.entries.read();
        entries.get(worktree_name).cloned().unwrap_or_default()
    }

    pub fn get_filtered(
        &self,
        worktree_name: &str,
        file_path: Option<&str>,
        severity: Option<&str>,
        resolved: Option<bool>,
    ) -> Vec<CommentItem> {
        let entries = self.entries.read();
        let comments = match entries.get(worktree_name) {
            Some(c) => c,
            None => return vec![],
        };

        comments
            .iter()
            .filter(|c| {
                if let Some(fp) = file_path {
                    if c.file_path != fp {
                        return false;
                    }
                }
                if let Some(sev) = severity {
                    if c.severity.as_deref() != Some(sev) {
                        return false;
                    }
                }
                if let Some(res) = resolved {
                    if c.resolved != res {
                        return false;
                    }
                }
                true
            })
            .cloned()
            .collect()
    }

    pub fn set_all(&self, worktree_name: &str, comments: Vec<CommentItem>) {
        self.entries
            .write()
            .insert(worktree_name.to_string(), comments);
    }

    pub fn cleanup(&self, app_data_dir: &Path, worktree_name: &str) -> Result<(), String> {
        self.entries.write().remove(worktree_name);
        let file_path = comments_file(app_data_dir, worktree_name);
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
    store: &CommentStore,
    worktree_name: &str,
    source: &str,
) -> Result<(), String> {
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {e}"))?;
    store.save(&data_dir, worktree_name)?;
    let _ = app.emit(
        "comments-changed",
        CommentsChangedPayload {
            worktree_name: worktree_name.to_string(),
            source: source.to_string(),
        },
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn load_comments(
    app: tauri::AppHandle,
    store: tauri::State<'_, Arc<CommentStore>>,
    worktree_name: String,
) -> Result<Vec<CommentItem>, String> {
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {e}"))?;
    store.load(&data_dir, &worktree_name)
}

#[tauri::command]
pub fn save_comments(
    app: tauri::AppHandle,
    store: tauri::State<'_, Arc<CommentStore>>,
    worktree_name: String,
    comments: Vec<CommentItem>,
) -> Result<(), String> {
    store.set_all(&worktree_name, comments);
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {e}"))?;
    store.save(&data_dir, &worktree_name)
}

#[tauri::command]
pub fn cleanup_comments(
    app: tauri::AppHandle,
    store: tauri::State<'_, Arc<CommentStore>>,
    worktree_name: String,
) -> Result<(), String> {
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {e}"))?;
    store.cleanup(&data_dir, &worktree_name)
}

use tauri::Emitter;
use tauri::Manager;

#[tauri::command]
pub fn add_comment(
    app: tauri::AppHandle,
    store: tauri::State<'_, Arc<CommentStore>>,
    worktree_name: String,
    comment: CommentItem,
    source: String,
) -> Result<(), String> {
    store.add(&worktree_name, comment);
    persist_and_notify(&app, &store, &worktree_name, &source)
}

#[tauri::command]
pub fn remove_comment(
    app: tauri::AppHandle,
    store: tauri::State<'_, Arc<CommentStore>>,
    worktree_name: String,
    id: String,
    source: String,
) -> Result<bool, String> {
    let removed = store.remove(&worktree_name, &id);
    persist_and_notify(&app, &store, &worktree_name, &source)?;
    Ok(removed)
}

#[tauri::command]
pub fn update_comment_content(
    app: tauri::AppHandle,
    store: tauri::State<'_, Arc<CommentStore>>,
    worktree_name: String,
    id: String,
    content: String,
    source: String,
) -> Result<bool, String> {
    let updated = store.update(&worktree_name, &id, &content);
    persist_and_notify(&app, &store, &worktree_name, &source)?;
    Ok(updated)
}

#[tauri::command]
pub fn mark_comments_sent(
    app: tauri::AppHandle,
    store: tauri::State<'_, Arc<CommentStore>>,
    worktree_name: String,
    ids: Vec<String>,
    source: String,
) -> Result<(), String> {
    store.mark_sent(&worktree_name, &ids);
    persist_and_notify(&app, &store, &worktree_name, &source)
}

#[tauri::command]
pub fn toggle_resolve_comment(
    app: tauri::AppHandle,
    store: tauri::State<'_, Arc<CommentStore>>,
    worktree_name: String,
    id: String,
    source: String,
) -> Result<Option<bool>, String> {
    let resolved = store.resolve(&worktree_name, &id);
    persist_and_notify(&app, &store, &worktree_name, &source)?;
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_comment(id: &str, file_path: &str, severity: Option<&str>) -> CommentItem {
        CommentItem {
            id: id.to_string(),
            file_path: file_path.to_string(),
            line_number: 10,
            end_line: None,
            content: "test comment".to_string(),
            status: "unsent".to_string(),
            created_at: 1234567890.0,
            parent_id: None,
            severity: severity.map(|s| s.to_string()),
            resolved: false,
            target: "local".to_string(),
        }
    }

    #[test]
    fn add_and_get_all() {
        let store = CommentStore::default();
        store.add("wt1", make_comment("c1", "file.rs", None));
        store.add("wt1", make_comment("c2", "file.rs", Some("error")));

        let all = store.get_all("wt1");
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn remove_comment() {
        let store = CommentStore::default();
        store.add("wt1", make_comment("c1", "file.rs", None));
        assert!(store.remove("wt1", "c1"));
        assert!(!store.remove("wt1", "c1"));
        assert_eq!(store.get_all("wt1").len(), 0);
    }

    #[test]
    fn update_comment() {
        let store = CommentStore::default();
        store.add("wt1", make_comment("c1", "file.rs", None));
        assert!(store.update("wt1", "c1", "updated"));
        assert_eq!(store.get_all("wt1")[0].content, "updated");
    }

    #[test]
    fn resolve_toggles() {
        let store = CommentStore::default();
        store.add("wt1", make_comment("c1", "file.rs", None));
        assert_eq!(store.resolve("wt1", "c1"), Some(true));
        assert_eq!(store.resolve("wt1", "c1"), Some(false));
        assert_eq!(store.resolve("wt1", "nonexistent"), None);
    }

    #[test]
    fn get_filtered() {
        let store = CommentStore::default();
        store.add("wt1", make_comment("c1", "a.rs", Some("error")));
        store.add("wt1", make_comment("c2", "b.rs", Some("warning")));
        store.add("wt1", make_comment("c3", "a.rs", Some("info")));

        let filtered = store.get_filtered("wt1", Some("a.rs"), None, None);
        assert_eq!(filtered.len(), 2);

        let filtered = store.get_filtered("wt1", None, Some("error"), None);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "c1");
    }

    #[test]
    fn save_and_load() {
        let dir = TempDir::new().unwrap();
        let store = CommentStore::default();
        store.add("wt1", make_comment("c1", "file.rs", Some("error")));
        store.save(dir.path(), "wt1").unwrap();

        let store2 = CommentStore::default();
        let loaded = store2.load(dir.path(), "wt1").unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "c1");
        assert_eq!(loaded[0].severity, Some("error".to_string()));
    }

    #[test]
    fn load_nonexistent_returns_empty() {
        let dir = TempDir::new().unwrap();
        let store = CommentStore::default();
        let loaded = store.load(dir.path(), "nonexistent").unwrap();
        assert!(loaded.is_empty());
    }

    #[test]
    fn cleanup_removes_file_and_memory() {
        let dir = TempDir::new().unwrap();
        let store = CommentStore::default();
        store.add("wt1", make_comment("c1", "file.rs", None));
        store.save(dir.path(), "wt1").unwrap();

        store.cleanup(dir.path(), "wt1").unwrap();
        assert!(store.get_all("wt1").is_empty());
        assert!(!comments_file(dir.path(), "wt1").exists());
    }

    #[test]
    fn set_all_replaces() {
        let store = CommentStore::default();
        store.add("wt1", make_comment("c1", "file.rs", None));
        store.set_all("wt1", vec![make_comment("c2", "other.rs", None)]);
        let all = store.get_all("wt1");
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, "c2");
    }

    #[test]
    fn mark_sent_updates_status() {
        let store = CommentStore::default();
        store.add("wt1", make_comment("c1", "file.rs", None));
        store.add("wt1", make_comment("c2", "file.rs", None));
        store.mark_sent("wt1", &["c1".to_string()]);
        let all = store.get_all("wt1");
        assert_eq!(all[0].status, "sent");
        assert_eq!(all[1].status, "unsent");
    }

    #[test]
    fn mark_sent_ignores_unknown_ids() {
        let store = CommentStore::default();
        store.add("wt1", make_comment("c1", "file.rs", None));
        store.mark_sent("wt1", &["nonexistent".to_string()]);
        assert_eq!(store.get_all("wt1")[0].status, "unsent");
    }

    #[test]
    fn add_normalizes_absolute_path() {
        let store = CommentStore::default();
        store.add(
            "/Users/dev/project",
            make_comment("c1", "/Users/dev/project/src/main.rs", None),
        );
        let all = store.get_all("/Users/dev/project");
        assert_eq!(all[0].file_path, "src/main.rs");
    }

    #[test]
    fn add_keeps_relative_path() {
        let store = CommentStore::default();
        store.add("myproject", make_comment("c1", "src/main.rs", None));
        let all = store.get_all("myproject");
        assert_eq!(all[0].file_path, "src/main.rs");
    }

    #[test]
    fn load_normalizes_absolute_path() {
        let dir = TempDir::new().unwrap();
        let worktree = "/Users/dev/project";
        let old_json = r#"[{
            "id": "c1",
            "file_path": "/Users/dev/project/src/main.rs",
            "line_number": 10,
            "content": "old comment",
            "status": "unsent",
            "created_at": 1234567890.0
        }]"#;
        let comments_dir = dir.path().join("comments");
        std::fs::create_dir_all(&comments_dir).unwrap();
        let safe_name = worktree.replace(['/', '\\'], "_");
        std::fs::write(comments_dir.join(format!("{safe_name}.json")), old_json).unwrap();

        let store = CommentStore::default();
        let loaded = store.load(dir.path(), worktree).unwrap();
        assert_eq!(loaded[0].file_path, "src/main.rs");
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

        let store = CommentStore::default();
        let loaded = store.load(dir.path(), "wt1").unwrap();
        assert_eq!(loaded.len(), 1);
        assert!(!loaded[0].resolved);
        assert_eq!(loaded[0].target, "local");
    }
}
