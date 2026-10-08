use crate::domain::installation::{
    BuildKind, CliLink, InstallationError, InstallationLocation, InstallationService,
};
use crate::infrastructure::platform::installation;
use std::path::{Path, PathBuf};
pub(crate) struct LocalInstallationService;
fn technical(error: impl Into<crate::domain::failure::TechnicalFailure>) -> InstallationError {
    InstallationError::Technical(error.into())
}
impl InstallationService for LocalInstallationService {
    fn executable(&self) -> Result<PathBuf, InstallationError> {
        std::env::current_exe().map_err(technical)
    }
    fn location(&self, executable: &Path) -> Result<InstallationLocation, InstallationError> {
        let read_only = installation::read_only(executable).map_err(technical)?;
        let build = if cfg!(debug_assertions) {
            BuildKind::Development
        } else {
            BuildKind::Release
        };
        Ok(InstallationLocation::from_path(
            executable,
            read_only,
            build,
            cfg!(target_os = "macos"),
        ))
    }
    fn cli_link(&self, link: &Path) -> Result<CliLink, InstallationError> {
        Ok(decode_cli_link(
            installation::cli_link(link).map_err(technical)?,
        ))
    }

    fn create_cli_link(&self, target: &Path, link: &Path) -> Result<(), InstallationError> {
        installation::create_cli_link(target, link).map_err(technical)
    }
    fn create_cli_link_as_admin(
        &self,
        target: &Path,
        link: &Path,
    ) -> Result<(), InstallationError> {
        #[cfg(target_os = "macos")]
        {
            installation::create_cli_link_as_admin(target, link).map_err(|message| {
                technical(crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::Other,
                    message,
                })
            })
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (target, link);
            Err(technical(crate::domain::failure::TechnicalFailure {
                nature: crate::domain::failure::TechnicalFailureNature::Other,
                message: "Administrator CLI installation requires macOS.".into(),
            }))
        }
    }
}

fn decode_cli_link(value: installation::CliLink) -> CliLink {
    match value {
        installation::CliLink::Missing => CliLink::Missing,
        installation::CliLink::Symlink(target) => CliLink::Symlink(target),
        installation::CliLink::Other => CliLink::Other,
    }
}

#[cfg(test)]
#[path = "installation_test.rs"]
mod installation_tests;
