use super::client as wire;
use crate::domain::installation::{
    CliInstallation, CliPlacement, InstallationError, LoginRegistration,
};
pub(crate) fn cli_installation(
    result: Result<CliInstallation, String>,
) -> wire::CliInstallationResult {
    use wire::CliInstallationStatus as W;
    let result = match result {
        Ok(result) => result,
        Err(reason) => {
            return wire::CliInstallationResult {
                status: W::Undetermined as i32,
                reason,
            }
        }
    };
    wire::CliInstallationResult {
        status: match result {
            CliInstallation::Allowed => W::Allowed,
            CliInstallation::Translocated => W::Translocated,
            CliInstallation::ReadOnly => W::ReadOnly,
            CliInstallation::Development => W::Development,
        } as i32,
        reason: result.reason().into(),
    }
}
pub(crate) fn login_registration(result: LoginRegistration) -> wire::LoginRegistrationResult {
    use wire::LoginRegistrationStatus as W;
    wire::LoginRegistrationResult {
        status: match result {
            LoginRegistration::Allowed => W::Allowed,
            LoginRegistration::Translocated => W::Translocated,
            LoginRegistration::ReadOnly => W::ReadOnly,
        } as i32,
        reason: result.reason().into(),
    }
}
pub(crate) fn installed(result: (CliPlacement, std::path::PathBuf)) -> wire::InstallCliResponse {
    wire::InstallCliResponse {
        status: match result.0 {
            CliPlacement::Create => wire::CliInstallStatus::Installed,
            CliPlacement::AlreadyInstalled => wire::CliInstallStatus::AlreadyInstalled,
        } as i32,
        path: result.1.to_string_lossy().into_owned(),
    }
}
impl super::connect::ConnectFailure for InstallationError {
    fn connect_code(&self) -> connectrpc::ErrorCode {
        match self {
            Self::Location(_) | Self::Occupied(_) => connectrpc::ErrorCode::FailedPrecondition,
            Self::Verification | Self::CreationFailed { .. } => connectrpc::ErrorCode::Internal,
            Self::Technical(error) => super::connect::ConnectFailure::connect_code(error),
        }
    }
}

pub(crate) fn failure(error: InstallationError) -> wire::CommandFailure {
    let reason = error.to_string();
    super::error::AppError::from_failure(error)
        .with_cause(Some(reason))
        .into()
}
