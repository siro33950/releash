//! Review image / binary 用の custom URI scheme handler。

use std::collections::HashMap;

use tauri::http::{header, Response, StatusCode};
use tauri::Manager;

use crate::adaptor::controller::state::AppState;
use crate::domain::code::{CodeError, ReviewBlobSide};
use crate::usecase::code_error::CodeUsecaseError;
use crate::usecase::review_usecase::review_blob_mime_for_path;

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReviewBlobRequest {
    worktree_path: String,
    path: String,
    side: ReviewBlobSide,
    section: String,
    base: String,
    version: u64,
}

pub(crate) fn register_review_blob_protocol(
    builder: tauri::Builder<tauri::Wry>,
) -> tauri::Builder<tauri::Wry> {
    builder.register_asynchronous_uri_scheme_protocol("review-blob", |ctx, request, responder| {
        let Some(state) = ctx.app_handle().try_state::<AppState>() else {
            responder.respond(text_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "app state not ready",
            ));
            return;
        };
        let review = state.review_usecase.clone();
        let request = parse_review_blob_request(request.uri().to_string());

        tauri::async_runtime::spawn(async move {
            let response = match request {
                Ok(blob) => {
                    let result = tauri::async_runtime::spawn_blocking(move || {
                        review
                            .read_review_blob_bytes(
                                &blob.worktree_path,
                                &blob.path,
                                blob.side,
                                &blob.section,
                                &blob.base,
                                blob.version,
                            )
                            .map(|bytes| (blob.path, bytes))
                    })
                    .await;
                    review_blob_response_from_result(result)
                }
                Err(message) => text_response(StatusCode::NOT_FOUND, &message),
            };
            responder.respond(response);
        });
    })
}

fn review_blob_response_from_result(
    result: Result<Result<(String, Vec<u8>), CodeUsecaseError>, tauri::Error>,
) -> Response<Vec<u8>> {
    match result {
        Ok(Ok((path, bytes))) => binary_response(&path, bytes),
        Ok(Err(error)) => text_response(review_blob_error_status(&error), &error.to_string()),
        Err(error) => text_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("review blob task join error: {error}"),
        ),
    }
}

fn parse_review_blob_request(uri: String) -> Result<ReviewBlobRequest, String> {
    let url = url::Url::parse(&uri).map_err(|e| e.to_string())?;
    let query = url.query_pairs().into_owned().collect::<HashMap<_, _>>();
    let required = |key: &str| {
        query
            .get(key)
            .cloned()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("missing review blob query: {key}"))
    };
    let side = match required("side")?.as_str() {
        "original" => ReviewBlobSide::Original,
        "modified" => ReviewBlobSide::Modified,
        other => return Err(format!("invalid review blob side: {other}")),
    };
    let version = required("version")?
        .parse::<u64>()
        .map_err(|e| format!("invalid review blob version: {e}"))?;
    Ok(ReviewBlobRequest {
        worktree_path: required("worktree")?,
        path: required("path")?,
        side,
        section: required("section")?,
        base: required("base")?,
        version,
    })
}

fn review_blob_error_status(error: &CodeUsecaseError) -> StatusCode {
    match error {
        CodeUsecaseError::Code(CodeError::StaleReviewBlobVersion { .. }) => StatusCode::CONFLICT,
        _ => StatusCode::NOT_FOUND,
    }
}

fn binary_response(path: &str, bytes: Vec<u8>) -> Response<Vec<u8>> {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, review_blob_mime_for_path(path))
        .header(header::CACHE_CONTROL, "no-store")
        .body(bytes)
        .unwrap()
}

fn text_response(status: StatusCode, message: &str) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-store")
        .body(message.as_bytes().to_vec())
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_review_blob_url() {
        let request = parse_review_blob_request(
            "review-blob://localhost/blob?worktree=/repo&path=src%2Fimage.png&side=modified&section=changes&base=head&version=7".to_string(),
        )
        .unwrap();

        assert_eq!(request.worktree_path, "/repo");
        assert_eq!(request.path, "src/image.png");
        assert_eq!(request.side, ReviewBlobSide::Modified);
        assert_eq!(request.section, "changes");
        assert_eq!(request.base, "head");
        assert_eq!(request.version, 7);
    }

    #[test]
    fn rejects_missing_required_query() {
        let error = parse_review_blob_request(
            "review-blob://localhost/blob?worktree=/repo&path=a.png&side=modified&section=changes"
                .to_string(),
        )
        .unwrap_err();

        assert!(error.contains("base") || error.contains("version"));
    }

    #[test]
    fn rejects_stale_review_blob_version() {
        let error = CodeUsecaseError::from(CodeError::StaleReviewBlobVersion {
            requested: 7,
            current: 8,
        });

        assert!(error.to_string().contains("stale review blob version"));
        assert_eq!(review_blob_error_status(&error), StatusCode::CONFLICT);
    }

    #[test]
    fn maps_not_found_review_blob_error_to_not_found() {
        let error = CodeUsecaseError::from(CodeError::Rule(
            "review blob not found: src/missing.png".to_string(),
        ));

        assert_eq!(review_blob_error_status(&error), StatusCode::NOT_FOUND);
    }

    #[test]
    fn maps_async_read_result_to_binary_response() {
        let response =
            review_blob_response_from_result(Ok(Ok(("src/image.png".to_string(), vec![1, 2, 3]))));

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "image/png"
        );
        assert!(response
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .is_none());
        assert_eq!(response.body(), &vec![1, 2, 3]);
    }

    #[test]
    fn maps_async_read_usecase_stale_error_to_protocol_conflict_response() {
        let usecase =
            crate::usecase::review_usecase::tests_support::review_usecase_with_snapshot_version(8);
        let result = usecase
            .read_review_blob_bytes(
                "/repo",
                "image.png",
                ReviewBlobSide::Modified,
                "changes",
                "head",
                7,
            )
            .map(|bytes| ("image.png".to_string(), bytes));

        let response = review_blob_response_from_result(Ok(result));

        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/plain; charset=utf-8"
        );
        assert!(response
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .is_none());
        assert_eq!(
            String::from_utf8(response.body().clone()).unwrap(),
            "stale review blob version: requested 7, current 8"
        );
    }
}
