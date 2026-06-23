//! ファイル内容参照（at_ref / at_branch_base / staged、テキスト／バイナリ）の Tauri コマンド。

use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::Serialize;
use tauri::State;

use super::run_blocking;
use crate::adaptor::controller::state::AppState;
use crate::other::AppError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewTextDiff {
    original: String,
    modified: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewImageDiff {
    original_base64: Option<String>,
    modified_base64: Option<String>,
}

fn is_branch_base(diff_base: &str) -> bool {
    diff_base == "branch-base"
}

fn is_staged_section(section: &str) -> bool {
    section == "staged"
}

fn read_working_tree_text(file_path: &str) -> Option<String> {
    std::fs::read_to_string(file_path).ok()
}

fn read_working_tree_base64(file_path: &str) -> Option<String> {
    std::fs::read(file_path)
        .ok()
        .map(|bytes| STANDARD.encode(bytes))
}

fn select_review_text_diff(
    diff_base: &str,
    section: &str,
    head: Option<String>,
    branch_base: Option<String>,
    staged: Option<String>,
    working_tree: Option<String>,
) -> ReviewTextDiff {
    if is_branch_base(diff_base) {
        return ReviewTextDiff {
            original: branch_base.unwrap_or_default(),
            modified: working_tree.unwrap_or_default(),
        };
    }
    if is_staged_section(section) {
        return ReviewTextDiff {
            original: head.unwrap_or_default(),
            modified: staged.unwrap_or_default(),
        };
    }
    if staged.is_none() && working_tree.is_none() {
        return ReviewTextDiff {
            original: head.unwrap_or_default(),
            modified: String::new(),
        };
    }
    ReviewTextDiff {
        original: staged.unwrap_or_default(),
        modified: working_tree.unwrap_or_default(),
    }
}

fn select_review_image_diff(
    diff_base: &str,
    section: &str,
    head: Option<String>,
    branch_base: Option<String>,
    staged: Option<String>,
    working_tree: Option<String>,
) -> ReviewImageDiff {
    if is_branch_base(diff_base) {
        return ReviewImageDiff {
            original_base64: branch_base,
            modified_base64: working_tree,
        };
    }
    if is_staged_section(section) {
        return ReviewImageDiff {
            original_base64: head,
            modified_base64: staged,
        };
    }
    ReviewImageDiff {
        original_base64: staged,
        modified_base64: working_tree,
    }
}

#[tauri::command]
pub async fn get_review_text_diff(
    state: State<'_, AppState>,
    file_path: String,
    diff_base: String,
    section: String,
) -> Result<ReviewTextDiff, AppError> {
    let uc = state.code_usecase.clone();
    run_blocking(move || {
        crate::other::telemetry::measure_result(
            crate::other::telemetry::HotPath::ReviewFileOpen,
            || {
                let diff = if is_branch_base(&diff_base) {
                    select_review_text_diff(
                        &diff_base,
                        &section,
                        None,
                        uc.get_file_at_branch_base(&file_path).ok(),
                        None,
                        read_working_tree_text(&file_path),
                    )
                } else if is_staged_section(&section) {
                    select_review_text_diff(
                        &diff_base,
                        &section,
                        uc.get_file_at_ref(&file_path, "HEAD").ok(),
                        None,
                        uc.get_staged_content(&file_path).ok(),
                        None,
                    )
                } else {
                    let staged = uc.get_staged_content(&file_path).ok();
                    let working_tree = read_working_tree_text(&file_path);
                    let head = if staged.is_none() && working_tree.is_none() {
                        uc.get_file_at_ref(&file_path, "HEAD").ok()
                    } else {
                        None
                    };
                    select_review_text_diff(&diff_base, &section, head, None, staged, working_tree)
                };
                Ok(diff)
            },
        )
    })
    .await
}

#[tauri::command]
pub async fn get_review_image_diff(
    state: State<'_, AppState>,
    file_path: String,
    diff_base: String,
    section: String,
) -> Result<ReviewImageDiff, AppError> {
    let uc = state.code_usecase.clone();
    run_blocking(move || {
        crate::other::telemetry::measure_result(
            crate::other::telemetry::HotPath::ReviewFileOpen,
            || {
                let diff = if is_branch_base(&diff_base) {
                    select_review_image_diff(
                        &diff_base,
                        &section,
                        None,
                        uc.get_binary_file_at_branch_base(&file_path).ok(),
                        None,
                        read_working_tree_base64(&file_path),
                    )
                } else if is_staged_section(&section) {
                    select_review_image_diff(
                        &diff_base,
                        &section,
                        uc.get_binary_file_at_ref(&file_path, "HEAD").ok(),
                        None,
                        uc.get_binary_staged_content(&file_path).ok(),
                        None,
                    )
                } else {
                    select_review_image_diff(
                        &diff_base,
                        &section,
                        None,
                        None,
                        uc.get_binary_staged_content(&file_path).ok(),
                        read_working_tree_base64(&file_path),
                    )
                };
                Ok(diff)
            },
        )
    })
    .await
}

#[tauri::command]
pub async fn get_file_at_ref(
    state: State<'_, AppState>,
    file_path: String,
    git_ref: String,
) -> Result<String, AppError> {
    let uc = state.code_usecase.clone();
    run_blocking(move || {
        crate::other::telemetry::measure_result(
            crate::other::telemetry::HotPath::ReviewFileOpen,
            || uc.get_file_at_ref(&file_path, &git_ref),
        )
    })
    .await
}

#[tauri::command]
pub async fn get_staged_content(
    state: State<'_, AppState>,
    file_path: String,
) -> Result<String, AppError> {
    let uc = state.code_usecase.clone();
    run_blocking(move || {
        crate::other::telemetry::measure_result(
            crate::other::telemetry::HotPath::ReviewFileOpen,
            || uc.get_staged_content(&file_path),
        )
    })
    .await
}

#[tauri::command]
pub async fn get_binary_staged_content(
    state: State<'_, AppState>,
    file_path: String,
) -> Result<String, AppError> {
    let uc = state.code_usecase.clone();
    run_blocking(move || {
        crate::other::telemetry::measure_result(
            crate::other::telemetry::HotPath::ReviewFileOpen,
            || uc.get_binary_staged_content(&file_path),
        )
    })
    .await
}

#[tauri::command]
pub async fn get_file_at_branch_base(
    state: State<'_, AppState>,
    file_path: String,
) -> Result<String, AppError> {
    let uc = state.code_usecase.clone();
    run_blocking(move || {
        crate::other::telemetry::measure_result(
            crate::other::telemetry::HotPath::ReviewFileOpen,
            || uc.get_file_at_branch_base(&file_path),
        )
    })
    .await
}

#[tauri::command]
pub async fn get_binary_file_at_branch_base(
    state: State<'_, AppState>,
    file_path: String,
) -> Result<String, AppError> {
    let uc = state.code_usecase.clone();
    run_blocking(move || {
        crate::other::telemetry::measure_result(
            crate::other::telemetry::HotPath::ReviewFileOpen,
            || uc.get_binary_file_at_branch_base(&file_path),
        )
    })
    .await
}

#[tauri::command]
pub async fn get_binary_file_at_ref(
    state: State<'_, AppState>,
    file_path: String,
    git_ref: String,
) -> Result<String, AppError> {
    let uc = state.code_usecase.clone();
    run_blocking(move || {
        crate::other::telemetry::measure_result(
            crate::other::telemetry::HotPath::ReviewFileOpen,
            || uc.get_binary_file_at_ref(&file_path, &git_ref),
        )
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_text_branch_base_uses_base_and_working_tree() {
        let diff = select_review_text_diff(
            "branch-base",
            "changes",
            Some("head".to_string()),
            Some("base".to_string()),
            Some("staged".to_string()),
            Some("working".to_string()),
        );

        assert_eq!(
            diff,
            ReviewTextDiff {
                original: "base".to_string(),
                modified: "working".to_string()
            }
        );
    }

    #[test]
    fn select_text_staged_uses_head_and_staged() {
        let diff = select_review_text_diff(
            "head",
            "staged",
            Some("head".to_string()),
            None,
            Some("staged".to_string()),
            Some("working".to_string()),
        );

        assert_eq!(
            diff,
            ReviewTextDiff {
                original: "head".to_string(),
                modified: "staged".to_string()
            }
        );
    }

    #[test]
    fn select_text_changes_uses_staged_and_working_tree() {
        let diff = select_review_text_diff(
            "head",
            "changes",
            Some("head".to_string()),
            None,
            Some("staged".to_string()),
            Some("working".to_string()),
        );

        assert_eq!(
            diff,
            ReviewTextDiff {
                original: "staged".to_string(),
                modified: "working".to_string()
            }
        );
    }

    #[test]
    fn select_text_added_file_uses_empty_original_and_working_tree() {
        let diff = select_review_text_diff(
            "head",
            "changes",
            Some("head".to_string()),
            None,
            None,
            Some("working".to_string()),
        );

        assert_eq!(
            diff,
            ReviewTextDiff {
                original: String::new(),
                modified: "working".to_string()
            }
        );
    }

    #[test]
    fn select_text_deleted_file_falls_back_to_head() {
        let diff = select_review_text_diff(
            "head",
            "changes",
            Some("head".to_string()),
            None,
            None,
            None,
        );

        assert_eq!(
            diff,
            ReviewTextDiff {
                original: "head".to_string(),
                modified: String::new()
            }
        );
    }

    #[test]
    fn select_image_branch_base_uses_base_and_working_tree() {
        let diff = select_review_image_diff(
            "branch-base",
            "changes",
            Some("HEAD64".to_string()),
            Some("BASE64".to_string()),
            Some("STAGED64".to_string()),
            Some("WORK64".to_string()),
        );

        assert_eq!(
            diff,
            ReviewImageDiff {
                original_base64: Some("BASE64".to_string()),
                modified_base64: Some("WORK64".to_string())
            }
        );
    }

    #[test]
    fn select_image_staged_uses_head_and_staged() {
        let diff = select_review_image_diff(
            "head",
            "staged",
            Some("HEAD64".to_string()),
            None,
            Some("STAGED64".to_string()),
            Some("WORK64".to_string()),
        );

        assert_eq!(
            diff,
            ReviewImageDiff {
                original_base64: Some("HEAD64".to_string()),
                modified_base64: Some("STAGED64".to_string())
            }
        );
    }

    #[test]
    fn select_image_changes_uses_staged_and_working_tree() {
        let diff = select_review_image_diff(
            "head",
            "changes",
            Some("HEAD64".to_string()),
            None,
            Some("STAGED64".to_string()),
            Some("WORK64".to_string()),
        );

        assert_eq!(
            diff,
            ReviewImageDiff {
                original_base64: Some("STAGED64".to_string()),
                modified_base64: Some("WORK64".to_string())
            }
        );
    }

    #[test]
    fn select_image_added_file_keeps_missing_original_null() {
        let diff = select_review_image_diff(
            "head",
            "changes",
            Some("HEAD64".to_string()),
            None,
            None,
            Some("WORK64".to_string()),
        );

        assert_eq!(
            diff,
            ReviewImageDiff {
                original_base64: None,
                modified_base64: Some("WORK64".to_string())
            }
        );
    }

    #[test]
    fn select_image_deleted_file_keeps_missing_modified_null_without_head_fallback() {
        let diff = select_review_image_diff(
            "head",
            "changes",
            Some("HEAD64".to_string()),
            None,
            Some("STAGED64".to_string()),
            None,
        );

        assert_eq!(
            diff,
            ReviewImageDiff {
                original_base64: Some("STAGED64".to_string()),
                modified_base64: None
            }
        );
    }
}
