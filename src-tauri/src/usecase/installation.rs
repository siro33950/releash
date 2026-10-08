use crate::domain::installation::{
    CliInstallation, CliPlacement, InstallationError, InstallationService, LoginRegistration,
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
    pub fn install_cli(&self, link: &Path) -> Result<(CliPlacement, PathBuf), InstallationError> {
        let executable = self.0.executable()?;
        self.0
            .location(&executable)?
            .cli_installation()
            .ensure_allowed()?;
        let target = executable.with_file_name("releash");
        let placement = self.0.cli_link(link)?.placement(&target, link)?;
        if placement == CliPlacement::Create {
            if let Err(direct_error) = self.0.create_cli_link(&target, link) {
                self.0
                    .create_cli_link_as_admin(&target, link, direct_error)?;
            }
            self.0.cli_link(link)?.verify(&target)?;
        }
        Ok((placement, link.to_path_buf()))
    }
}
#[cfg(test)]
#[path = "installation_test.rs"]
pub(crate) mod installation_tests;
