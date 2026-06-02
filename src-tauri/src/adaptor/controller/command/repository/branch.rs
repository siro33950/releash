use tauri::State;

use super::run_blocking;
use crate::adaptor::controller::state::AppState;
use crate::domain::repository::Branch;
use crate::other::AppError;

#[tauri::command]
pub async fn list_branches(
    state: State<'_, AppState>,
    repo_path: String,
) -> Result<Vec<Branch>, AppError> {
    let uc = state.repository_usecase.clone();
    run_blocking(move || uc.list_branches(&repo_path)).await
}

#[tauri::command]
pub async fn get_current_branch(
    state: State<'_, AppState>,
    repo_path: String,
) -> Result<String, AppError> {
    let uc = state.repository_usecase.clone();
    run_blocking(move || uc.get_current_branch(&repo_path)).await
}

#[tauri::command]
pub async fn get_default_branch(
    state: State<'_, AppState>,
    repo_path: String,
) -> Result<String, AppError> {
    let uc = state.repository_usecase.clone();
    run_blocking(move || uc.get_default_branch(&repo_path)).await
}

#[tauri::command]
pub async fn git_create_branch(
    state: State<'_, AppState>,
    repo_path: String,
    branch_name: String,
) -> Result<(), AppError> {
    let uc = state.repository_usecase.clone();
    run_blocking(move || uc.create_branch(&repo_path, &branch_name)).await
}

#[tauri::command]
pub async fn delete_branch(
    state: State<'_, AppState>,
    repo_path: String,
    branch_name: String,
    force: bool,
) -> Result<(), AppError> {
    let uc = state.repository_usecase.clone();
    run_blocking(move || uc.delete_branch(&repo_path, &branch_name, force)).await
}
