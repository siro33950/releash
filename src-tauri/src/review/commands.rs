use std::sync::Arc;

use tauri::{AppHandle, State};

use crate::review_prompt::get_per_file_review_tasks_internal;
use crate::thread_store::ThreadStore;

use super::orchestrator::{ReviewOrchestrator, ReviewSessionStatus};

#[tauri::command]
pub fn start_review(
    app: AppHandle,
    orchestrator: State<'_, Arc<ReviewOrchestrator>>,
    thread_store: State<'_, Arc<ThreadStore>>,
    worktree_path: String,
    command_template: String,
    concurrency: usize,
) -> Result<Option<String>, String> {
    let tasks = get_per_file_review_tasks_internal(&worktree_path, &thread_store)?;

    if tasks.is_empty() {
        return Ok(None);
    }

    let session_id =
        orchestrator.start_review(&app, &worktree_path, &command_template, concurrency, tasks);
    Ok(Some(session_id))
}

#[tauri::command]
pub fn cancel_review(
    app: AppHandle,
    orchestrator: State<'_, Arc<ReviewOrchestrator>>,
    review_session_id: String,
) -> Result<(), String> {
    orchestrator
        .cancel_review(&app, &review_session_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_review_status(
    orchestrator: State<'_, Arc<ReviewOrchestrator>>,
    review_session_id: String,
) -> Option<ReviewSessionStatus> {
    orchestrator.get_status(&review_session_id)
}

#[tauri::command]
pub fn reset_review(
    app: AppHandle,
    orchestrator: State<'_, Arc<ReviewOrchestrator>>,
    review_session_id: String,
) {
    orchestrator.reset(&app, &review_session_id);
}
