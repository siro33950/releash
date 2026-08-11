//! code 責務の Tauri コマンド（薄い入口）。
//!
//! 引数の受け渡しと型変換のみを行い、ビジネスロジックは usecase / query service に
//! 委ねる。git2 等のブロッキング呼び出しを非同期境界へ載せるため、各コマンドは
//! `run_blocking` でユースケースを呼ぶ（移行前 `git/commands.rs` の `blocking` と等価）。

pub(crate) mod diff;
pub(crate) mod file_content;
pub(crate) mod hunk;
pub(crate) mod language;
pub(crate) mod markdown;
pub(crate) mod review;
pub(crate) mod review_blob;
pub(crate) mod staging;

use crate::domain::code::CodeError;
use crate::other::AppError;
use crate::usecase::code_error::CodeUsecaseError;

pub(super) const COMMAND_NAMES: &[&str] = &[
    "get_file_at_ref",
    "get_staged_content",
    "get_binary_staged_content",
    "get_file_at_branch_base",
    "get_binary_file_at_branch_base",
    "get_binary_file_at_ref",
    "get_review_snapshot",
    "get_review_file_view",
    "git_stage_review_group",
    "git_unstage_review_group",
    "get_branch_diff_summary",
    "build_diff_file_tree",
    "get_head_diff_file_tree_snapshot",
    "get_file_navigation",
    "compute_hidden_ranges",
    "compute_hidden_ranges_from_content",
    "compute_visible_markdown_blocks",
    "compute_markdown_diff_ranges",
    "compute_markdown_split_rows",
    "compute_markdown_inline_chunks",
    "get_language_from_path",
    "get_relative_path",
    "git_stage",
    "git_unstage",
];

const STALE_REVIEW_GROUP_TARGET_ERROR_CODE: &str = "STALE_REVIEW_GROUP_TARGET";

pub(crate) fn register(router: &mut super::CommandRouter) {
    router.register_domain(COMMAND_NAMES, Box::new(invoke_handler()));
}

pub(crate) fn invoke_handler(
) -> impl Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        file_content::get_file_at_ref,
        file_content::get_staged_content,
        file_content::get_binary_staged_content,
        file_content::get_file_at_branch_base,
        file_content::get_binary_file_at_branch_base,
        file_content::get_binary_file_at_ref,
        review::get_review_snapshot,
        review::get_review_file_view,
        review::git_stage_review_group,
        review::git_unstage_review_group,
        diff::get_branch_diff_summary,
        diff::build_diff_file_tree,
        diff::get_head_diff_file_tree_snapshot,
        diff::get_file_navigation,
        hunk::compute_hidden_ranges,
        hunk::compute_hidden_ranges_from_content,
        hunk::compute_visible_markdown_blocks,
        markdown::compute_markdown_diff_ranges,
        markdown::compute_markdown_split_rows,
        markdown::compute_markdown_inline_chunks,
        language::get_language_from_path,
        diff::get_relative_path,
        staging::git_stage,
        staging::git_unstage,
    ]
}

/// ユースケースエラー → アプリエラーの集約変換（adaptor 層が担う）。
/// `#[error(transparent)]` な `CodeUsecaseError` の `Display` を保持するため、
/// 通常エラーの serialize 表現は移行前（`GitError` のプレーン文字列）と等価に保たれる。
/// frontend が回復判断を必要とする stale review group だけ機械可読 code を付ける。
impl From<CodeUsecaseError> for AppError {
    fn from(e: CodeUsecaseError) -> Self {
        let message = e.to_string();
        match &e {
            CodeUsecaseError::Code(CodeError::StaleReviewGroupTarget { .. }) => {
                AppError::coded(STALE_REVIEW_GROUP_TARGET_ERROR_CODE, message)
            }
            _ => AppError::Internal(message),
        }
    }
}

/// ユースケース呼び出しを `spawn_blocking` 上で実行し、結果を `AppError` に集約する
/// 共通ヘルパー。join 失敗時のメッセージは移行前と等価に保つ。
pub(super) async fn run_blocking<T, F>(f: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, CodeUsecaseError> + Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| AppError::new(format!("task join error: {e}")))?
        .map_err(AppError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::code::CodeError;

    #[test]
    fn code由来のdisplayが変換チェーンを通じて保持される() {
        // CodeError → CodeUsecaseError → AppError の変換でメッセージ文字列が保持され、
        // serialize 表現が移行前（GitError のプレーン文字列）と等価であることをガードする。
        let usecase_err = CodeUsecaseError::Code(CodeError::Rule("file not staged".to_string()));
        let app_err = AppError::from(usecase_err);
        assert_eq!(app_err.to_string(), "file not staged");
        assert_eq!(
            serde_json::to_string(&app_err).unwrap(),
            "\"file not staged\""
        );
    }

    #[test]
    fn external由来のdisplayが変換チェーンを通じて保持される() {
        let usecase_err = CodeUsecaseError::Code(CodeError::External("git2 boom".to_string()));
        let app_err = AppError::from(usecase_err);
        assert_eq!(app_err.to_string(), "git2 boom");
        assert_eq!(serde_json::to_string(&app_err).unwrap(), "\"git2 boom\"");
    }

    #[test]
    fn stale_review_group_targetは機械可読codeを持つerrorとして返す() {
        let usecase_err = CodeUsecaseError::Code(CodeError::StaleReviewGroupTarget {
            group_id: "g:old:0".to_string(),
        });
        let app_err = AppError::from(usecase_err);

        assert_eq!(app_err.to_string(), "review group target stale: g:old:0");
        assert_eq!(
            serde_json::to_value(&app_err).unwrap(),
            serde_json::json!({
                "code": STALE_REVIEW_GROUP_TARGET_ERROR_CODE,
                "message": "review group target stale: g:old:0"
            })
        );
    }
}
