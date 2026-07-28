use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

use crate::domain::workflow::WorkflowError;

#[derive(Debug, serde::Serialize)]
pub(super) struct ApiErrorBody {
    pub(super) code: String,
    pub(super) message: String,
}

#[derive(Debug)]
pub(super) struct ApiError {
    status: StatusCode,
    body: ApiErrorBody,
}

impl ApiError {
    pub(super) fn new(
        status: StatusCode,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            status,
            body: ApiErrorBody {
                code: code.into(),
                message: message.into(),
            },
        }
    }

    pub(super) fn invalid_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalid_request", message)
    }

    pub(super) fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", message)
    }

    pub(super) fn unauthorized() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "a valid bearer token is required",
        )
    }

    pub(super) fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", message)
    }
}

impl From<WorkflowError> for ApiError {
    fn from(error: WorkflowError) -> Self {
        match error {
            WorkflowError::Validation(message) => {
                Self::new(StatusCode::BAD_REQUEST, "validation_error", message)
            }
            WorkflowError::InvalidState(message) => {
                Self::new(StatusCode::CONFLICT, "invalid_state", message)
            }
            WorkflowError::NotFound(message) => {
                Self::new(StatusCode::NOT_FOUND, "not_found", message)
            }
            WorkflowError::UnauthorizedApprovalTarget(message) => Self::new(
                StatusCode::FORBIDDEN,
                "unauthorized_approval_target",
                message,
            ),
            WorkflowError::External(message) => {
                Self::new(StatusCode::INTERNAL_SERVER_ERROR, "workflow_error", message)
            }
            WorkflowError::StorageUnavailable { message, .. } => Self::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "storage_unavailable",
                message,
            ),
            WorkflowError::CorruptStoredState(message) => Self::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "corrupt_stored_state",
                message,
            ),
            WorkflowError::IncompatibleStoredEvent(message) => Self::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "incompatible_stored_event",
                message,
            ),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(self.body)).into_response()
    }
}
