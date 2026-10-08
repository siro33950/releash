use crate::domain::failure::TechnicalFailure;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BuildKind {
    Development,
    Release,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct InstallationLocation {
    translocated: bool,
    read_only: bool,
    build: BuildKind,
}
impl InstallationLocation {
    pub fn from_path(path: &Path, read_only: bool, build: BuildKind, macos: bool) -> Self {
        Self {
            translocated: macos
                && path
                    .components()
                    .any(|part| part.as_os_str() == "AppTranslocation"),
            read_only,
            build,
        }
    }
    pub fn login_registration(self) -> LoginRegistration {
        if self.translocated {
            LoginRegistration::Translocated
        } else if self.read_only {
            LoginRegistration::ReadOnly
        } else {
            LoginRegistration::Allowed
        }
    }
    pub fn cli_installation(self) -> CliInstallation {
        match self.login_registration() {
            LoginRegistration::Translocated => CliInstallation::Translocated,
            LoginRegistration::ReadOnly => CliInstallation::ReadOnly,
            LoginRegistration::Allowed if self.build == BuildKind::Development => {
                CliInstallation::Development
            }
            LoginRegistration::Allowed => CliInstallation::Allowed,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LoginRegistration {
    Allowed,
    Translocated,
    ReadOnly,
}
impl LoginRegistration {
    pub fn reason(self) -> &'static str {
        match self {
            Self::Allowed => "",
            Self::Translocated => "Move Releash.app to Applications before enabling Launch at login.",
            Self::ReadOnly => "Releash.app is on a read-only volume. Move it to Applications before enabling Launch at login.",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CliInstallation {
    Allowed,
    Translocated,
    ReadOnly,
    Development,
}
impl CliInstallation {
    pub fn reason(self) -> &'static str {
        match self {
            Self::Allowed => "",
            Self::Translocated => "Move Releash.app to Applications before installing the CLI (AppTranslocation).",
            Self::ReadOnly => "Releash.app is on a read-only volume. Move it to Applications before installing the CLI.",
            Self::Development => "Install the CLI from a release build of Releash.app.",
        }
    }
    pub fn ensure_allowed(self) -> Result<(), InstallationError> {
        if self == Self::Allowed {
            Ok(())
        } else {
            Err(InstallationError::Location(self))
        }
    }
}
pub(crate) struct CliPaths {
    pub target: PathBuf,
    pub link: PathBuf,
}
impl CliPaths {
    pub fn from_executable(executable: &Path) -> Self {
        Self {
            target: executable.with_file_name("releash"),
            link: "/usr/local/bin/releash".into(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CliLink {
    Missing,
    Symlink(PathBuf),
    Other,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CliPlacement {
    AlreadyInstalled,
    Create,
}
impl CliLink {
    pub fn placement(&self, target: &Path, link: &Path) -> Result<CliPlacement, InstallationError> {
        match self {
            Self::Symlink(existing) if existing == target => Ok(CliPlacement::AlreadyInstalled),
            Self::Other => Err(InstallationError::Occupied(link.to_path_buf())),
            _ => Ok(CliPlacement::Create),
        }
    }
    pub fn verify(&self, target: &Path) -> Result<(), InstallationError> {
        match self {
            Self::Symlink(existing) if existing == target => Ok(()),
            _ => Err(InstallationError::Verification),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum InstallationError {
    Location(CliInstallation),
    Occupied(PathBuf),
    Verification,
    CreationFailed {
        direct: Box<InstallationError>,
        administrator: Box<InstallationError>,
    },
    Technical(TechnicalFailure),
}
impl std::fmt::Display for InstallationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Location(result) => f.write_str(result.reason()),
            Self::Occupied(path) => write!(
                f,
                "refusing to overwrite non-symlink CLI path: {}",
                path.display()
            ),
            Self::Verification => {
                f.write_str("CLI installation did not create the expected symlink")
            }
            Self::CreationFailed {
                direct,
                administrator,
            } => write!(
                f,
                "direct install failed ({direct}); administrator install failed ({administrator})"
            ),
            Self::Technical(error) => error.fmt(f),
        }
    }
}
pub(crate) trait InstallationService: Send + Sync {
    fn executable(&self) -> Result<PathBuf, InstallationError>;
    fn location(&self, executable: &Path) -> Result<InstallationLocation, InstallationError>;
    fn cli_link(&self, link: &Path) -> Result<CliLink, InstallationError>;
    fn create_cli_link(&self, target: &Path, link: &Path) -> Result<(), InstallationError>;
    fn create_cli_link_as_admin(&self, target: &Path, link: &Path)
        -> Result<(), InstallationError>;
}
#[cfg(test)]
#[path = "mod_test.rs"]
mod mod_tests;
