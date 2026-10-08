use crate::domain::installation::{
    CliInstallation, CliPaths, CliPlacement, InstallationError, InstallationService,
    LoginRegistration,
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

pub(crate) struct InstallationUsecase(pub Arc<dyn InstallationService>);
impl InstallationUsecase {
    pub fn cli_installation(&self) -> Result<CliInstallation, InstallationError> {
        Ok(self.0.location(&self.0.executable()?)?.cli_installation())
    }
    pub fn login_registration(
        &self,
        executable: &Path,
    ) -> Result<LoginRegistration, InstallationError> {
        Ok(self.0.location(executable)?.login_registration())
    }
    pub fn install_cli(&self) -> Result<(CliPlacement, PathBuf), InstallationError> {
        let executable = self.0.executable()?;
        self.0
            .location(&executable)?
            .cli_installation()
            .ensure_allowed()?;
        let CliPaths { target, link } = CliPaths::from_executable(&executable);
        let placement = self.0.cli_link(&link)?.placement(&target, &link)?;
        if placement == CliPlacement::Create {
            if let Err(direct_error) = self.0.create_cli_link(&target, &link) {
                self.0
                    .create_cli_link_as_admin(&target, &link)
                    .map_err(|administrator| InstallationError::CreationFailed {
                        direct: Box::new(direct_error),
                        administrator: Box::new(administrator),
                    })?;
            }
            self.0.cli_link(&link)?.verify(&target)?;
        }
        Ok((placement, link))
    }
}
#[cfg(test)]
#[path = "installation_test.rs"]
pub(crate) mod installation_tests;
