use std::path::PathBuf;
use std::sync::Arc;

use tauri::{Emitter, Manager};

use super::{
    build_review_thread_handoff_message, ReviewActor, ReviewCommentStore, ReviewHistoryEntry,
    ReviewTarget, ReviewThread, ReviewThreadFilter,
};
use crate::path_aliases::{alias_name_for_profile, BuildProfile};

fn data_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {e}"))
}

fn emit_changed(app: &tauri::AppHandle, worktree_name: &str) {
    let _ = app.emit("review-comments-changed", worktree_name);
}

/// `ReviewCommentStore` への同期 I/O を `tokio::task::spawn_blocking` で逃がす共通ヘルパー。
/// Tauri command の async ワーカースレッドをブロックしないようにするため、
/// 全ての store 操作はこの helper を経由して実行する。
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
    store: tauri::State<'_, Arc<ReviewCommentStore>>,
    worktree_name: String,
    filter: Option<ReviewThreadFilter>,
) -> Result<Vec<ReviewThread>, String> {
    let data_dir = data_dir(&app)?;
    let store = Arc::clone(&store);
    blocking(move || {
        store
            .list_threads(&data_dir, &worktree_name, filter, ReviewActor::human())
            .map_err(String::from)
    })
    .await
}

#[tauri::command]
pub async fn get_review_thread(
    app: tauri::AppHandle,
    store: tauri::State<'_, Arc<ReviewCommentStore>>,
    worktree_name: String,
    thread_id: String,
) -> Result<ReviewThread, String> {
    let data_dir = data_dir(&app)?;
    let store = Arc::clone(&store);
    blocking(move || {
        store
            .get_thread(&data_dir, &worktree_name, &thread_id)
            .map_err(String::from)
    })
    .await
}

#[tauri::command]
pub async fn create_review_thread(
    app: tauri::AppHandle,
    store: tauri::State<'_, Arc<ReviewCommentStore>>,
    worktree_name: String,
    file_path: Option<String>,
    line_number: Option<u32>,
    end_line: Option<u32>,
    content: String,
) -> Result<ReviewThread, String> {
    let data_dir = data_dir(&app)?;
    let store = Arc::clone(&store);
    let worktree_name_for_event = worktree_name.clone();
    let thread = blocking(move || {
        store
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
            .map_err(String::from)
    })
    .await?;
    emit_changed(&app, &worktree_name_for_event);
    Ok(thread)
}

#[tauri::command]
pub async fn append_review_comment(
    app: tauri::AppHandle,
    store: tauri::State<'_, Arc<ReviewCommentStore>>,
    worktree_name: String,
    thread_id: String,
    content: String,
) -> Result<ReviewThread, String> {
    let data_dir = data_dir(&app)?;
    let store = Arc::clone(&store);
    let worktree_name_for_event = worktree_name.clone();
    let thread = blocking(move || {
        store
            .append_comment(
                &data_dir,
                &worktree_name,
                ReviewActor::human(),
                &thread_id,
                content,
            )
            .map_err(String::from)
    })
    .await?;
    emit_changed(&app, &worktree_name_for_event);
    Ok(thread)
}

#[tauri::command]
pub async fn resolve_review_thread(
    app: tauri::AppHandle,
    store: tauri::State<'_, Arc<ReviewCommentStore>>,
    worktree_name: String,
    thread_id: String,
    outcome: String,
    summary: String,
) -> Result<ReviewThread, String> {
    let data_dir = data_dir(&app)?;
    let store = Arc::clone(&store);
    let worktree_name_for_event = worktree_name.clone();
    let thread = blocking(move || {
        store
            .resolve_thread(
                &data_dir,
                &worktree_name,
                ReviewActor::human(),
                &thread_id,
                outcome,
                summary,
            )
            .map_err(String::from)
    })
    .await?;
    emit_changed(&app, &worktree_name_for_event);
    Ok(thread)
}

#[tauri::command]
pub async fn delete_review_thread(
    app: tauri::AppHandle,
    store: tauri::State<'_, Arc<ReviewCommentStore>>,
    worktree_name: String,
    thread_id: String,
) -> Result<(), String> {
    let data_dir = data_dir(&app)?;
    let store = Arc::clone(&store);
    let worktree_name_for_event = worktree_name.clone();
    blocking(move || {
        store
            .delete_thread(&data_dir, &worktree_name, ReviewActor::human(), &thread_id)
            .map_err(String::from)
    })
    .await?;
    emit_changed(&app, &worktree_name_for_event);
    Ok(())
}

/// spec issues-1022 "Thread handoff contract":
/// 対象 Thread の参照情報を含む Agent 共有メッセージを Rust 側で組み立てて返す。
/// フロントエンドはこの文字列を active な AgentChat session の入力としてそのまま送信する。
/// メッセージ本文の整形は Rust が owner であり、フロントエンドは本文の作成を行わない。
///
/// Follow-up: メッセージ内の CLI 実行例は `path_aliases` の wrapper が PATH に登録する
/// 名前 (`releash` / `releash-dev`) と一致させる必要がある。ここで build profile から
/// alias 名を解決して builder に渡し、Dev (`releash-dev`) / Production (`releash`) 双方で
/// Agent process から CLI を呼べる文面を保証する。
#[tauri::command]
pub async fn build_review_thread_handoff(
    app: tauri::AppHandle,
    store: tauri::State<'_, Arc<ReviewCommentStore>>,
    worktree_name: String,
    thread_id: String,
) -> Result<String, String> {
    let data_dir = data_dir(&app)?;
    let store = Arc::clone(&store);
    blocking(move || {
        let thread = store
            .get_thread(&data_dir, &worktree_name, &thread_id)
            .map_err(String::from)?;
        let releash_alias = alias_name_for_profile(BuildProfile::current());
        Ok(build_review_thread_handoff_message(releash_alias, &thread))
    })
    .await
}

#[tauri::command]
pub async fn get_review_thread_history(
    app: tauri::AppHandle,
    store: tauri::State<'_, Arc<ReviewCommentStore>>,
    worktree_name: String,
    thread_id: String,
) -> Result<Vec<ReviewHistoryEntry>, String> {
    let data_dir = data_dir(&app)?;
    let store = Arc::clone(&store);
    blocking(move || {
        store
            .history(&data_dir, &worktree_name, &thread_id)
            .map_err(String::from)
    })
    .await
}
