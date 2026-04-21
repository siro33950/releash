use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{Emitter, Manager};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DiffComment {
    pub id: String,
    pub file_path: String,
    pub line_number: Option<u32>,
    pub end_line: Option<u32>,
    pub content: String,
    pub status: String,
    pub created_at: f64,
}

pub struct DiffCommentStore {
    entries: RwLock<HashMap<String, Vec<DiffComment>>>,
    file_lock: Mutex<()>,
}

impl Default for DiffCommentStore {
    fn default() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
            file_lock: Mutex::new(()),
        }
    }
}

fn state_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("diff-comments")
}

fn state_file(app_data_dir: &Path, worktree_name: &str) -> PathBuf {
    let safe_name = worktree_name.replace(['/', '\\'], "_");
    state_dir(app_data_dir).join(format!("{safe_name}.json"))
}

impl DiffCommentStore {
    pub fn load(&self, app_data_dir: &Path, worktree_name: &str) -> Vec<DiffComment> {
        let file_path = state_file(app_data_dir, worktree_name);

        if !file_path.exists() {
            return Vec::new();
        }

        let data = match std::fs::read_to_string(&file_path) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("[DiffCommentStore] Failed to read {}: {e}", file_path.display());
                return Vec::new();
            }
        };

        let comments: Vec<DiffComment> = match serde_json::from_str(&data) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[DiffCommentStore] Failed to parse {}: {e}", file_path.display());
                return Vec::new();
            }
        };

        self.entries
            .write()
            .insert(worktree_name.to_string(), comments.clone());
        comments
    }

    pub fn save(&self, app_data_dir: &Path, worktree_name: &str) -> Result<(), String> {
        let _guard = self.file_lock.lock();

        let dir = state_dir(app_data_dir);
        std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create dir: {e}"))?;

        let file_path = state_file(app_data_dir, worktree_name);
        let comments = {
            let entries = self.entries.read();
            match entries.get(worktree_name) {
                Some(c) => c.clone(),
                None => Vec::new(),
            }
        };
        let json = serde_json::to_string_pretty(&comments)
            .map_err(|e| format!("Failed to serialize: {e}"))?;
        std::fs::write(&file_path, json).map_err(|e| format!("Failed to write: {e}"))?;
        Ok(())
    }

    pub fn add(
        &self,
        worktree_name: &str,
        file_path: String,
        line_number: Option<u32>,
        end_line: Option<u32>,
        content: String,
    ) -> DiffComment {
        let comment = DiffComment {
            id: Uuid::new_v4().to_string(),
            file_path,
            line_number,
            end_line,
            content,
            status: "unsent".to_string(),
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64(),
        };

        self.entries
            .write()
            .entry(worktree_name.to_string())
            .or_default()
            .push(comment.clone());

        comment
    }

    pub fn update(
        &self,
        worktree_name: &str,
        comment_id: &str,
        content: String,
    ) -> Result<(), String> {
        let mut entries = self.entries.write();
        let comments = entries
            .get_mut(worktree_name)
            .ok_or_else(|| "Worktree not found".to_string())?;

        let comment = comments
            .iter_mut()
            .find(|c| c.id == comment_id)
            .ok_or_else(|| "Comment not found".to_string())?;

        comment.content = content;
        Ok(())
    }

    pub fn delete(&self, worktree_name: &str, comment_id: &str) -> Result<(), String> {
        let mut entries = self.entries.write();
        let comments = entries
            .get_mut(worktree_name)
            .ok_or_else(|| "Worktree not found".to_string())?;

        let len_before = comments.len();
        comments.retain(|c| c.id != comment_id);

        if comments.len() == len_before {
            return Err("Comment not found".to_string());
        }
        Ok(())
    }

    pub fn mark_sent(&self, worktree_name: &str, comment_ids: &[String]) -> Result<(), String> {
        let mut entries = self.entries.write();
        let comments = entries
            .get_mut(worktree_name)
            .ok_or_else(|| "Worktree not found".to_string())?;

        for comment in comments.iter_mut() {
            if comment_ids.contains(&comment.id) {
                comment.status = "sent".to_string();
            }
        }
        Ok(())
    }

    pub fn get_by_ids(&self, worktree_name: &str, comment_ids: &[String]) -> Vec<DiffComment> {
        let entries = self.entries.read();
        match entries.get(worktree_name) {
            Some(comments) => comments
                .iter()
                .filter(|c| comment_ids.contains(&c.id))
                .cloned()
                .collect(),
            None => Vec::new(),
        }
    }

    pub fn get_unsent(&self, worktree_name: &str) -> Vec<DiffComment> {
        let entries = self.entries.read();
        match entries.get(worktree_name) {
            Some(comments) => comments
                .iter()
                .filter(|c| c.status == "unsent")
                .cloned()
                .collect(),
            None => Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

fn emit_changed(app: &tauri::AppHandle, worktree_name: &str) {
    let _ = app.emit("diff-comments-changed", worktree_name);
}

#[tauri::command]
pub fn load_diff_comments(
    app: tauri::AppHandle,
    store: tauri::State<'_, Arc<DiffCommentStore>>,
    worktree_name: String,
) -> Vec<DiffComment> {
    let data_dir = match app.path().app_data_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[DiffCommentStore] Failed to get app data dir: {e}");
            return Vec::new();
        }
    };
    store.load(&data_dir, &worktree_name)
}

#[tauri::command]
pub fn add_diff_comment(
    app: tauri::AppHandle,
    store: tauri::State<'_, Arc<DiffCommentStore>>,
    worktree_name: String,
    file_path: String,
    line_number: Option<u32>,
    end_line: Option<u32>,
    content: String,
) -> Result<DiffComment, String> {
    let comment = store.add(
        &worktree_name,
        file_path,
        line_number,
        end_line,
        content,
    );

    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {e}"))?;
    store.save(&data_dir, &worktree_name)?;
    emit_changed(&app, &worktree_name);
    Ok(comment)
}

#[tauri::command]
pub fn update_diff_comment(
    app: tauri::AppHandle,
    store: tauri::State<'_, Arc<DiffCommentStore>>,
    worktree_name: String,
    comment_id: String,
    content: String,
) -> Result<(), String> {
    store.update(&worktree_name, &comment_id, content)?;

    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {e}"))?;
    store.save(&data_dir, &worktree_name)?;
    emit_changed(&app, &worktree_name);
    Ok(())
}

#[tauri::command]
pub fn delete_diff_comment(
    app: tauri::AppHandle,
    store: tauri::State<'_, Arc<DiffCommentStore>>,
    worktree_name: String,
    comment_id: String,
) -> Result<(), String> {
    store.delete(&worktree_name, &comment_id)?;

    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {e}"))?;
    store.save(&data_dir, &worktree_name)?;
    emit_changed(&app, &worktree_name);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn add_and_load_roundtrip() {
        let dir = TempDir::new().unwrap();
        let store = DiffCommentStore::default();

        let comment = store.add(
            "wt1",
            "src/main.rs".to_string(),
            Some(42),
            None,
            "Fix this logic".to_string(),
        );

        assert_eq!(comment.file_path, "src/main.rs");
        assert_eq!(comment.line_number, Some(42));
        assert_eq!(comment.end_line, None);
        assert_eq!(comment.content, "Fix this logic");
        assert_eq!(comment.status, "unsent");
        assert!(!comment.id.is_empty());

        store.save(dir.path(), "wt1").unwrap();

        let store2 = DiffCommentStore::default();
        let loaded = store2.load(dir.path(), "wt1");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0], comment);
    }

    #[test]
    fn load_nonexistent_returns_empty() {
        let dir = TempDir::new().unwrap();
        let store = DiffCommentStore::default();
        let result = store.load(dir.path(), "nonexistent");
        assert!(result.is_empty());
    }

    #[test]
    fn add_multiple_comments() {
        let store = DiffCommentStore::default();

        store.add(
            "wt1",
            "src/a.rs".to_string(),
            Some(10),
            None,
            "Comment A".to_string(),
        );
        store.add(
            "wt1",
            "src/b.rs".to_string(),
            None,
            None,
            "Comment B".to_string(),
        );
        store.add(
            "wt1",
            "src/a.rs".to_string(),
            Some(5),
            Some(15),
            "Range comment".to_string(),
        );

        let entries = store.entries.read();
        let comments = entries.get("wt1").unwrap();
        assert_eq!(comments.len(), 3);
    }

    #[test]
    fn update_comment() {
        let store = DiffCommentStore::default();

        let comment = store.add(
            "wt1",
            "src/main.rs".to_string(),
            Some(1),
            None,
            "Old content".to_string(),
        );

        store
            .update("wt1", &comment.id, "New content".to_string())
            .unwrap();

        let entries = store.entries.read();
        let comments = entries.get("wt1").unwrap();
        assert_eq!(comments[0].content, "New content");
    }

    #[test]
    fn update_nonexistent_comment_fails() {
        let store = DiffCommentStore::default();
        store.add(
            "wt1",
            "src/main.rs".to_string(),
            Some(1),
            None,
            "Content".to_string(),
        );

        let result = store.update("wt1", "nonexistent-id", "New".to_string());
        assert!(result.is_err());
    }

    #[test]
    fn delete_comment() {
        let store = DiffCommentStore::default();

        let c1 = store.add(
            "wt1",
            "src/a.rs".to_string(),
            Some(1),
            None,
            "A".to_string(),
        );
        let _c2 = store.add(
            "wt1",
            "src/b.rs".to_string(),
            Some(2),
            None,
            "B".to_string(),
        );

        store.delete("wt1", &c1.id).unwrap();

        let entries = store.entries.read();
        let comments = entries.get("wt1").unwrap();
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].content, "B");
    }

    #[test]
    fn delete_nonexistent_comment_fails() {
        let store = DiffCommentStore::default();
        store.add(
            "wt1",
            "src/main.rs".to_string(),
            Some(1),
            None,
            "Content".to_string(),
        );

        let result = store.delete("wt1", "nonexistent-id");
        assert!(result.is_err());
    }

    #[test]
    fn mark_sent() {
        let store = DiffCommentStore::default();

        let c1 = store.add(
            "wt1",
            "src/a.rs".to_string(),
            Some(1),
            None,
            "A".to_string(),
        );
        let c2 = store.add(
            "wt1",
            "src/b.rs".to_string(),
            Some(2),
            None,
            "B".to_string(),
        );
        let _c3 = store.add(
            "wt1",
            "src/c.rs".to_string(),
            Some(3),
            None,
            "C".to_string(),
        );

        store
            .mark_sent("wt1", &[c1.id.clone(), c2.id.clone()])
            .unwrap();

        let entries = store.entries.read();
        let comments = entries.get("wt1").unwrap();
        assert_eq!(comments[0].status, "sent");
        assert_eq!(comments[1].status, "sent");
        assert_eq!(comments[2].status, "unsent");
    }

    #[test]
    fn get_unsent() {
        let store = DiffCommentStore::default();

        let c1 = store.add(
            "wt1",
            "src/a.rs".to_string(),
            Some(1),
            None,
            "A".to_string(),
        );
        store.add(
            "wt1",
            "src/b.rs".to_string(),
            Some(2),
            None,
            "B".to_string(),
        );

        store.mark_sent("wt1", &[c1.id.clone()]).unwrap();

        let unsent = store.get_unsent("wt1");
        assert_eq!(unsent.len(), 1);
        assert_eq!(unsent[0].content, "B");
    }

    #[test]
    fn get_by_ids() {
        let store = DiffCommentStore::default();

        let c1 = store.add(
            "wt1",
            "src/a.rs".to_string(),
            Some(1),
            None,
            "A".to_string(),
        );
        let _c2 = store.add(
            "wt1",
            "src/b.rs".to_string(),
            Some(2),
            None,
            "B".to_string(),
        );
        let c3 = store.add(
            "wt1",
            "src/c.rs".to_string(),
            Some(3),
            None,
            "C".to_string(),
        );

        let result = store.get_by_ids("wt1", &[c1.id.clone(), c3.id.clone()]);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].content, "A");
        assert_eq!(result[1].content, "C");
    }

    #[test]
    fn file_comment_without_line_number() {
        let dir = TempDir::new().unwrap();
        let store = DiffCommentStore::default();

        let comment = store.add(
            "wt1",
            "src/main.rs".to_string(),
            None,
            None,
            "This file needs refactoring".to_string(),
        );

        assert_eq!(comment.line_number, None);
        assert_eq!(comment.end_line, None);

        store.save(dir.path(), "wt1").unwrap();

        let store2 = DiffCommentStore::default();
        let loaded = store2.load(dir.path(), "wt1");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].line_number, None);
    }

    #[test]
    fn worktree_name_sanitization() {
        let dir = TempDir::new().unwrap();
        let store = DiffCommentStore::default();

        store.add(
            "path/to/worktree",
            "src/main.rs".to_string(),
            Some(1),
            None,
            "Content".to_string(),
        );
        store.save(dir.path(), "path/to/worktree").unwrap();

        let expected_file = dir
            .path()
            .join("diff-comments")
            .join("path_to_worktree.json");
        assert!(expected_file.exists());
    }
}
