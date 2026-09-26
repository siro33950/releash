use crate::adaptor::presenter::{connect::ConnectFailure, error::AppError};
use crate::usecase::terminal_surface::error::UsecaseError;
use serde::Serialize;
#[derive(Clone, Copy)]
pub(crate) enum TerminalCommandErrorCode {
    PtyError,
    InvalidRequest,
}

impl TerminalCommandErrorCode {
    fn code(self) -> &'static str {
        match self {
            Self::PtyError => "PTY_ERROR",
            Self::InvalidRequest => "INVALID_REQUEST",
        }
    }
}

pub(crate) fn invalid_owner_error(
    operation: TerminalCommandOperation,
    internal_cause: String,
) -> TerminalCommandError {
    let code = TerminalCommandErrorCode::InvalidRequest;
    log::warn!(
        "Terminal command failed: operation={} code={} cause={}",
        operation.name(),
        code.code(),
        internal_cause
    );
    TerminalCommandError {
        kind: connectrpc::ErrorCode::InvalidArgument,
        code: code.code().to_string(),
        message: operation.message(code).to_string(),
    }
}

pub(crate) fn invalid_terminal_write_owner_error(internal_cause: String) -> AppError {
    log::warn!(
        "Terminal command failed: operation=write_terminal_surface code=INVALID_REQUEST cause={}",
        internal_cause
    );
    AppError::new("Terminal input could not be sent because the request is invalid.")
        .with_status(connectrpc::ErrorCode::InvalidArgument)
}

pub(crate) fn terminal_write_error(error: UsecaseError) -> AppError {
    log::error!(
        "Terminal command failed: operation=write_terminal_surface code=PTY_ERROR cause={}",
        error
    );
    AppError::new("Terminal input could not be sent. Try again.").with_status(error.connect_code())
}

pub(crate) fn invalid_terminal_resize_owner_error(internal_cause: String) -> AppError {
    log::warn!(
        "Terminal command failed: operation=resize_terminal_surface code=INVALID_REQUEST cause={}",
        internal_cause
    );
    AppError::new("Terminal resize failed because the request is invalid.")
        .with_status(connectrpc::ErrorCode::InvalidArgument)
}

pub(crate) fn terminal_resize_error(error: UsecaseError) -> AppError {
    log::error!(
        "Terminal command failed: operation=resize_terminal_surface code=PTY_ERROR cause={}",
        error
    );
    AppError::new("Terminal resize failed. Try again.").with_status(error.connect_code())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TerminalCommandError {
    #[serde(skip)]
    pub kind: connectrpc::ErrorCode,
    pub code: String,
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TerminalCommandOperation {
    Initialize,
}

impl TerminalCommandOperation {
    fn name(self) -> &'static str {
        match self {
            Self::Initialize => "get_or_spawn_terminal_surface",
        }
    }

    fn message(self, code: TerminalCommandErrorCode) -> &'static str {
        match (self, code) {
            (Self::Initialize, TerminalCommandErrorCode::PtyError) => {
                "Terminal initialization failed. Try again."
            }
            (Self::Initialize, TerminalCommandErrorCode::InvalidRequest) => {
                "Terminal initialization failed because the request is invalid."
            }
        }
    }
}

impl TerminalCommandError {
    pub(crate) fn from_usecase(error: UsecaseError, operation: TerminalCommandOperation) -> Self {
        let kind = error.connect_code();
        let internal_cause = error.to_string();
        let code = match error {
            UsecaseError::Gateway(_)
            | UsecaseError::OwnerConflict
            | UsecaseError::PtySpawn { .. }
            | UsecaseError::OtherSpawnFailure { .. } => TerminalCommandErrorCode::PtyError,
        };
        log::error!(
            "Terminal command failed: operation={} code={} cause={}",
            operation.name(),
            code.code(),
            internal_cause
        );
        Self {
            kind,
            code: code.code().to_string(),
            message: operation.message(code).to_string(),
        }
    }
}
