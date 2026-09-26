use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

#[derive(Debug, serde::Serialize)]
pub(crate) struct ApiErrorBody {
    pub(crate) code: String,
    pub(crate) message: String,
}

#[derive(Debug)]
pub(crate) struct ApiError {
    pub(crate) status: StatusCode,
    pub(crate) body: ApiErrorBody,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(self.body)).into_response()
    }
}

#[cfg(test)]
#[path = "error_test.rs"]
mod error_tests;
