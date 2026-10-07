use crate::adaptor::presenter::connect::ConnectFailure;
use axum::http::StatusCode;

use crate::adaptor::controller::api::error::{ApiError, ApiErrorBody};
use crate::domain::workflow::WorkflowError;

impl ApiError {
    fn new(status: StatusCode, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            status,
            body: ApiErrorBody {
                code: code.into(),
                message: message.into(),
            },
        }
    }

    pub(crate) fn invalid_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalid_request", message)
    }

    pub(crate) fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", message)
    }

    pub(crate) fn unauthorized() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "a valid bearer token is required",
        )
    }

    pub(crate) fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", message)
    }
}

impl From<WorkflowError> for ApiError {
    fn from(error: WorkflowError) -> Self {
        let status = error.connect_code().http_status();
        let mut response = match error {
            WorkflowError::Store(_) => Self::new(status, "storage_unavailable", error.to_string()),
            WorkflowError::Validation(message) => {
                Self::new(StatusCode::BAD_REQUEST, "validation_error", message)
            }
            WorkflowError::Conflict(message) => {
                Self::new(StatusCode::CONFLICT, "workflow_conflict", message)
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
            WorkflowError::Technical(error) => {
                Self::new(status, "workflow_error", error.to_string())
            }
            WorkflowError::Editor(error) => Self::new(status, "workflow_error", error.to_string()),
            WorkflowError::External(message) => {
                Self::new(StatusCode::INTERNAL_SERVER_ERROR, "workflow_error", message)
            }
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
        };
        response.status = status;
        response
    }
}

impl ApiError {
    pub(crate) fn provider_lifecycle_unavailable() -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "provider_lifecycle_unavailable",
            "Provider lifecycle service is unavailable",
        )
    }
}

impl From<crate::usecase::provider_lifecycle::ProviderLifecycleIngressUsecaseError> for ApiError {
    fn from(
        error: crate::usecase::provider_lifecycle::ProviderLifecycleIngressUsecaseError,
    ) -> Self {
        use crate::usecase::provider_lifecycle::ProviderLifecycleIngressUsecaseError;
        let status = error.connect_code().http_status();
        let mut response = match error {
            ProviderLifecycleIngressUsecaseError::Technical(failure) => {
                ApiError::new(status, "INTERNAL", failure.message)
            }
            ProviderLifecycleIngressUsecaseError::Payload(error) => {
                ApiError::invalid_request(error.to_string())
            }
            ProviderLifecycleIngressUsecaseError::InvalidInput => {
                ApiError::invalid_request("Provider lifecycle input is invalid")
            }
            ProviderLifecycleIngressUsecaseError::Conflict => ApiError::new(
                StatusCode::CONFLICT,
                "provider_lifecycle_conflict",
                "Provider lifecycle conflicts with current AgentSession ownership",
            ),
            ProviderLifecycleIngressUsecaseError::Store(_)
            | ProviderLifecycleIngressUsecaseError::StorageUnavailable => ApiError::new(
                status,
                "provider_lifecycle_storage_unavailable",
                "Provider lifecycle persistence is unavailable",
            ),
            ProviderLifecycleIngressUsecaseError::Corrupt => {
                ApiError::internal("Provider lifecycle state is corrupt")
            }
        };
        response.status = status;
        response
    }
}

#[cfg(test)]
#[path = "api_error_test.rs"]
mod api_error_tests;

impl From<crate::domain::failure::TechnicalFailure> for ApiError {
    fn from(error: crate::domain::failure::TechnicalFailure) -> Self {
        Self::new(
            error.connect_code().http_status(),
            "technical_failure",
            error.to_string(),
        )
    }
}

impl From<crate::common::concurrency::Rejection> for ApiError {
    fn from(error: crate::common::concurrency::Rejection) -> Self {
        let failure = crate::adaptor::presenter::connect::request_rejected(&error);
        Self::new(failure.http_status(), "request_rejected", error.to_string())
    }
}
