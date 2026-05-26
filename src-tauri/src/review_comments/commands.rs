use std::path::PathBuf;
use std::sync::Arc;

use tauri::{Emitter, Manager};

use super::{
    ReviewActor, ReviewCommentStore, ReviewHistoryEntry, ReviewStanceValue, ReviewTarget,
    ReviewThread, ReviewThreadFilter,
};

fn data_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {e}"))
}

fn emit_changed(app: &tauri::AppHandle, worktree_name: &str) {
    let _ = app.emit("review-comments-changed", worktree_name);
}

#[tauri::command]
pub fn list_review_threads(
    app: tauri::AppHandle,
    store: tauri::State<'_, Arc<ReviewCommentStore>>,
    worktree_name: String,
    filter: Option<ReviewThreadFilter>,
) -> Result<Vec<ReviewThread>, String> {
    let data_dir = data_dir(&app)?;
    store
        .list_threads(&data_dir, &worktree_name, filter, ReviewActor::human())
        .map_err(String::from)
}

#[tauri::command]
pub fn get_review_thread(
    app: tauri::AppHandle,
    store: tauri::State<'_, Arc<ReviewCommentStore>>,
    worktree_name: String,
    thread_id: String,
) -> Result<ReviewThread, String> {
    let data_dir = data_dir(&app)?;
    store
        .get_thread(&data_dir, &worktree_name, &thread_id, ReviewActor::human())
        .map_err(String::from)
}

#[tauri::command]
pub fn create_review_thread(
    app: tauri::AppHandle,
    store: tauri::State<'_, Arc<ReviewCommentStore>>,
    worktree_name: String,
    file_path: Option<String>,
    line_number: Option<u32>,
    end_line: Option<u32>,
    content: String,
) -> Result<ReviewThread, String> {
    let data_dir = data_dir(&app)?;
    let thread = store
        .create_thread(
            &data_dir,
            &worktree_name,
            ReviewActor::human(),
            ReviewTarget {
                file_path,
                line_number,
                end_line,
            },
            content,
        )
        .map_err(String::from)?;
    emit_changed(&app, &worktree_name);
    Ok(thread)
}

#[tauri::command]
pub fn append_review_comment(
    app: tauri::AppHandle,
    store: tauri::State<'_, Arc<ReviewCommentStore>>,
    worktree_name: String,
    thread_id: String,
    content: String,
) -> Result<ReviewThread, String> {
    let data_dir = data_dir(&app)?;
    let thread = store
        .append_comment(
            &data_dir,
            &worktree_name,
            ReviewActor::human(),
            &thread_id,
            content,
        )
        .map_err(String::from)?;
    emit_changed(&app, &worktree_name);
    Ok(thread)
}

#[tauri::command]
pub fn set_review_stance(
    app: tauri::AppHandle,
    store: tauri::State<'_, Arc<ReviewCommentStore>>,
    worktree_name: String,
    thread_id: String,
    value: ReviewStanceValue,
) -> Result<ReviewThread, String> {
    let data_dir = data_dir(&app)?;
    let thread = store
        .set_stance(
            &data_dir,
            &worktree_name,
            ReviewActor::human(),
            &thread_id,
            value,
        )
        .map_err(String::from)?;
    emit_changed(&app, &worktree_name);
    Ok(thread)
}

#[tauri::command]
pub fn resolve_review_thread(
    app: tauri::AppHandle,
    store: tauri::State<'_, Arc<ReviewCommentStore>>,
    worktree_name: String,
    thread_id: String,
    outcome: String,
    summary: String,
) -> Result<ReviewThread, String> {
    let data_dir = data_dir(&app)?;
    let thread = store
        .resolve_thread(
            &data_dir,
            &worktree_name,
            ReviewActor::human(),
            &thread_id,
            outcome,
            summary,
        )
        .map_err(String::from)?;
    emit_changed(&app, &worktree_name);
    Ok(thread)
}

#[tauri::command]
pub fn get_review_thread_history(
    app: tauri::AppHandle,
    store: tauri::State<'_, Arc<ReviewCommentStore>>,
    worktree_name: String,
    thread_id: String,
) -> Result<Vec<ReviewHistoryEntry>, String> {
    let data_dir = data_dir(&app)?;
    store
        .history(&data_dir, &worktree_name, &thread_id)
        .map_err(String::from)
}
