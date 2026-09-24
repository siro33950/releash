//! review read model の Tauri コマンド。

use super::run_blocking;
use crate::adaptor::controller::state::AppState;
use crate::adaptor::protocol::code::{
    ReviewFileViewInput, ReviewGroupActionInput, ReviewSnapshotInput, ReviewTargetInput,
};
use crate::other::AppError;
use crate::usecase::code_dto::{ReviewFileViewDto, ReviewSnapshotDto};
use crate::usecase::review_usecase::{ReviewTarget, ReviewViewport};

pub(crate) async fn get_review_snapshot_shared(
    state: &AppState,
    input: ReviewSnapshotInput,
) -> Result<ReviewSnapshotDto, AppError> {
    let review = state.review_usecase.clone();
    run_blocking(move || review.get_review_snapshot(&input.worktree_path, &input.base)).await
}

pub(crate) async fn get_review_file_view_shared(
    state: &AppState,
    input: ReviewFileViewInput,
) -> Result<ReviewFileViewDto, AppError> {
    let review = state.review_usecase.clone();
    run_blocking(move || {
        review.get_review_file_view(
            &input.worktree_path,
            match input.target {
                ReviewTargetInput::FileId(value) => ReviewTarget::FileId(value),
                ReviewTargetInput::Path(value) => ReviewTarget::Path(value),
            },
            &input.section,
            &input.base,
            input.viewport.map(|viewport| ReviewViewport {
                start_line: viewport.start_line,
                end_line: viewport.end_line,
            }),
            input.snapshot_version,
        )
    })
    .await
}

pub(crate) async fn git_stage_review_group_shared(
    state: &AppState,
    input: ReviewGroupActionInput,
) -> Result<(), AppError> {
    let review = state.review_usecase.clone();
    run_blocking(move || {
        review.git_stage_review_group(
            &input.worktree_path,
            &input.path,
            &input.section,
            &input.base,
            &input.group_id,
        )
    })
    .await
}

pub(crate) async fn git_unstage_review_group_shared(
    state: &AppState,
    input: ReviewGroupActionInput,
) -> Result<(), AppError> {
    let review = state.review_usecase.clone();
    run_blocking(move || {
        review.git_unstage_review_group(
            &input.worktree_path,
            &input.path,
            &input.section,
            &input.base,
            &input.group_id,
        )
    })
    .await
}

pub(crate) async fn get_review_blob_shared(
    state: &AppState,
    reference: String,
) -> Result<String, AppError> {
    use base64::Engine;
    let blob = parse_blob_reference(&reference)?;
    let review = state.review_usecase.clone();
    let mime = crate::usecase::review_usecase::review_blob_mime_for_path(&blob.path);
    let bytes = run_blocking(move || {
        review.read_review_blob_bytes(
            &blob.worktree,
            &blob.path,
            blob.side,
            &blob.section,
            &blob.base,
            blob.version,
        )
    })
    .await?;
    Ok(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

struct BlobReference {
    worktree: String,
    path: String,
    side: crate::domain::code::ReviewBlobSide,
    section: String,
    base: String,
    version: u64,
}

fn parse_blob_reference(reference: &str) -> Result<BlobReference, AppError> {
    let invalid = || {
        AppError::coded(
            "INVALID_REQUEST",
            "Invalid review blob reference",
            crate::domain::failure::FailureKind::InvalidInput,
        )
    };
    let query = reference.strip_prefix("blob?").ok_or_else(invalid)?;
    let params: std::collections::HashMap<_, _> = url::form_urlencoded::parse(query.as_bytes())
        .into_owned()
        .collect();
    let required = |key| {
        params
            .get(key)
            .filter(|value| !value.is_empty())
            .cloned()
            .ok_or_else(invalid)
    };
    Ok(BlobReference {
        worktree: required("worktree")?,
        path: required("path")?,
        side: match required("side")?.as_str() {
            "original" => crate::domain::code::ReviewBlobSide::Original,
            "modified" => crate::domain::code::ReviewBlobSide::Modified,
            _ => return Err(invalid()),
        },
        section: required("section")?,
        base: required("base")?,
        version: required("version")?.parse().map_err(|_| invalid())?,
    })
}

#[cfg(test)]
#[path = "review_test.rs"]
mod review_tests;
