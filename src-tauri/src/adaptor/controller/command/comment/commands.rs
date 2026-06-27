use std::path::PathBuf;
use std::sync::Arc;

use tauri::{Emitter, Manager};

use crate::domain::comment::{ReviewActor, ReviewTarget};
use crate::infrastructure::platform::path_aliases::{alias_name_for_profile, BuildProfile};
use crate::usecase::comment::{
    review_error_to_json_string, ReviewCommentUsecase, ReviewHistoryEntryDto, ReviewThreadDto,
    ReviewThreadFilterDto,
};

fn data_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {e}"))
}

fn emit_changed(app: &tauri::AppHandle, worktree_name: &str) {
    let _ = app.emit("review-comments-changed", worktree_name);
}

async fn blocking<T, F>(f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn list_review_threads(
    app: tauri::AppHandle,
    usecase: tauri::State<'_, Arc<ReviewCommentUsecase>>,
    worktree_name: String,
    filter: Option<ReviewThreadFilterDto>,
) -> Result<Vec<ReviewThreadDto>, String> {
    let data_dir = data_dir(&app)?;
    let usecase = Arc::clone(&usecase);
    blocking(move || {
        usecase
            .list_threads(
                &data_dir,
                &worktree_name,
                filter.map(Into::into),
                ReviewActor::human(),
            )
            .map(|threads| threads.into_iter().map(ReviewThreadDto::from).collect())
            .map_err(review_error_to_json_string)
    })
    .await
}

#[tauri::command]
pub async fn get_review_thread(
    app: tauri::AppHandle,
    usecase: tauri::State<'_, Arc<ReviewCommentUsecase>>,
    worktree_name: String,
    thread_id: String,
) -> Result<ReviewThreadDto, String> {
    let data_dir = data_dir(&app)?;
    let usecase = Arc::clone(&usecase);
    blocking(move || {
        usecase
            .get_thread(&data_dir, &worktree_name, &thread_id)
            .map(ReviewThreadDto::from)
            .map_err(review_error_to_json_string)
    })
    .await
}

#[tauri::command]
pub async fn create_review_thread(
    app: tauri::AppHandle,
    usecase: tauri::State<'_, Arc<ReviewCommentUsecase>>,
    worktree_name: String,
    file_path: Option<String>,
    line_number: Option<u32>,
    end_line: Option<u32>,
    content: String,
) -> Result<ReviewThreadDto, String> {
    let data_dir = data_dir(&app)?;
    let usecase = Arc::clone(&usecase);
    let worktree_name_for_event = worktree_name.clone();
    let thread = blocking(move || {
        usecase
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
            .map(ReviewThreadDto::from)
            .map_err(review_error_to_json_string)
    })
    .await?;
    emit_changed(&app, &worktree_name_for_event);
    Ok(thread)
}

#[tauri::command]
pub async fn append_review_comment(
    app: tauri::AppHandle,
    usecase: tauri::State<'_, Arc<ReviewCommentUsecase>>,
    worktree_name: String,
    thread_id: String,
    content: String,
) -> Result<ReviewThreadDto, String> {
    let data_dir = data_dir(&app)?;
    let usecase = Arc::clone(&usecase);
    let worktree_name_for_event = worktree_name.clone();
    let thread = blocking(move || {
        usecase
            .append_comment(
                &data_dir,
                &worktree_name,
                ReviewActor::human(),
                &thread_id,
                content,
            )
            .map(ReviewThreadDto::from)
            .map_err(review_error_to_json_string)
    })
    .await?;
    emit_changed(&app, &worktree_name_for_event);
    Ok(thread)
}

#[tauri::command]
pub async fn resolve_review_thread(
    app: tauri::AppHandle,
    usecase: tauri::State<'_, Arc<ReviewCommentUsecase>>,
    worktree_name: String,
    thread_id: String,
    outcome: String,
    summary: String,
) -> Result<ReviewThreadDto, String> {
    let data_dir = data_dir(&app)?;
    let usecase = Arc::clone(&usecase);
    let worktree_name_for_event = worktree_name.clone();
    let thread = blocking(move || {
        usecase
            .resolve_thread(
                &data_dir,
                &worktree_name,
                ReviewActor::human(),
                &thread_id,
                outcome,
                summary,
            )
            .map(ReviewThreadDto::from)
            .map_err(review_error_to_json_string)
    })
    .await?;
    emit_changed(&app, &worktree_name_for_event);
    Ok(thread)
}

#[tauri::command]
pub async fn delete_review_thread(
    app: tauri::AppHandle,
    usecase: tauri::State<'_, Arc<ReviewCommentUsecase>>,
    worktree_name: String,
    thread_id: String,
) -> Result<(), String> {
    let data_dir = data_dir(&app)?;
    let usecase = Arc::clone(&usecase);
    let worktree_name_for_event = worktree_name.clone();
    blocking(move || {
        usecase
            .delete_thread(&data_dir, &worktree_name, ReviewActor::human(), &thread_id)
            .map_err(review_error_to_json_string)
    })
    .await?;
    emit_changed(&app, &worktree_name_for_event);
    Ok(())
}

#[tauri::command]
pub async fn build_review_thread_handoff(
    app: tauri::AppHandle,
    usecase: tauri::State<'_, Arc<ReviewCommentUsecase>>,
    worktree_name: String,
    thread_id: String,
) -> Result<String, String> {
    let data_dir = data_dir(&app)?;
    let usecase = Arc::clone(&usecase);
    blocking(move || {
        let releash_alias = alias_name_for_profile(BuildProfile::current());
        usecase
            .build_handoff(&data_dir, &worktree_name, &thread_id, releash_alias)
            .map_err(review_error_to_json_string)
    })
    .await
}

#[tauri::command]
pub async fn get_review_thread_history(
    app: tauri::AppHandle,
    usecase: tauri::State<'_, Arc<ReviewCommentUsecase>>,
    worktree_name: String,
    thread_id: String,
) -> Result<Vec<ReviewHistoryEntryDto>, String> {
    let data_dir = data_dir(&app)?;
    let usecase = Arc::clone(&usecase);
    blocking(move || {
        usecase
            .history(&data_dir, &worktree_name, &thread_id)
            .map(|events| {
                events
                    .into_iter()
                    .map(ReviewHistoryEntryDto::from)
                    .collect()
            })
            .map_err(review_error_to_json_string)
    })
    .await
}
