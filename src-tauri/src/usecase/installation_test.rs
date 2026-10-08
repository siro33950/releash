use super::*;
use crate::domain::installation::{BuildKind, CliLink, InstallationLocation};
use parking_lot::Mutex;
pub(crate) struct FakeInstallation {
    pub(crate) failure_stage: Option<&'static str>,
    pub(crate) path: PathBuf,
    pub(crate) read_only: bool,
    pub(crate) build: BuildKind,
    pub(crate) link: Mutex<CliLink>,
    pub(crate) calls: Mutex<Vec<&'static str>>,
    pub(crate) direct_fails: bool,
    pub(crate) admin_fails: bool,
    pub(crate) admin_verifies: bool,
}
impl Default for FakeInstallation {
    fn default() -> Self {
        Self {
            failure_stage: None,
            path: "/Applications/Releash.app/Contents/MacOS/releashd".into(),
            read_only: false,
            build: BuildKind::Release,
            link: Mutex::new(CliLink::Missing),
            calls: Mutex::new(vec![]),
            direct_fails: false,
            admin_fails: false,
            admin_verifies: true,
        }
    }
}
fn failure() -> InstallationError {
    InstallationError::Technical(crate::domain::failure::TechnicalFailure {
        nature: crate::domain::failure::TechnicalFailureNature::Other,
        message: "denied".into(),
    })
}
impl InstallationService for FakeInstallation {
    fn executable(&self) -> Result<PathBuf, InstallationError> {
        if self.failure_stage == Some("executable") {
            return Err(failure());
        }
        Ok(self.path.clone())
    }
    fn location(&self, path: &Path) -> Result<InstallationLocation, InstallationError> {
        self.calls.lock().push("location");
        if self.failure_stage == Some("location") {
            return Err(failure());
        }
        Ok(InstallationLocation::from_path(
            path,
            self.read_only,
            self.build,
            true,
        ))
    }
    fn cli_link(&self, _: &Path) -> Result<CliLink, InstallationError> {
        self.calls.lock().push("observe");
        if self.failure_stage == Some("observe") {
            return Err(failure());
        }
        Ok(self.link.lock().clone())
    }
    fn create_cli_link(&self, target: &Path, _: &Path) -> Result<(), InstallationError> {
        self.calls.lock().push("direct");
        if self.direct_fails {
            return Err(failure());
        }
        *self.link.lock() = CliLink::Symlink(target.into());
        Ok(())
    }
    fn create_cli_link_as_admin(
        &self,
        target: &Path,
        _: &Path,
        _: InstallationError,
    ) -> Result<(), InstallationError> {
        self.calls.lock().push("admin");
        if self.admin_fails {
            return Err(failure());
        }
        if self.admin_verifies {
            *self.link.lock() = CliLink::Symlink(target.into());
        }
        Ok(())
    }
}
#[test]
fn test_cli設置_拒否時にはリンクの観測も変更もしない() {
    // Given
    for fake in [
        FakeInstallation {
            path: "/AppTranslocation/id/releashd".into(),
            ..Default::default()
        },
        FakeInstallation {
            read_only: true,
            ..Default::default()
        },
        FakeInstallation {
            build: BuildKind::Development,
            ..Default::default()
        },
    ] {
        let port = Arc::new(fake);
        let usecase = InstallationUsecase(port.clone());
        // When
        let availability = usecase.cli_installation().unwrap();
        let error = usecase
            .install_cli(Path::new("/usr/local/bin/releash"))
            .unwrap_err();
        // Then
        assert_eq!(error, InstallationError::Location(availability));
        assert_eq!(*port.calls.lock(), ["location", "location"]);
    }
}
#[test]
fn test_cli設置_既存状態と権限に従い設置後の指し先を検証する() {
    // Given
    for (existing, direct_fails, admin_fails, admin_verifies, expected, calls) in [
        (
            CliLink::Missing,
            false,
            false,
            true,
            Ok(CliPlacement::Create),
            vec!["location", "observe", "direct", "observe"],
        ),
        (
            CliLink::Symlink("/old/releash".into()),
            false,
            false,
            true,
            Ok(CliPlacement::Create),
            vec!["location", "observe", "direct", "observe"],
        ),
        (
            CliLink::Symlink("/Applications/Releash.app/Contents/MacOS/releash".into()),
            false,
            false,
            true,
            Ok(CliPlacement::AlreadyInstalled),
            vec!["location", "observe"],
        ),
        (
            CliLink::Other,
            false,
            false,
            true,
            Err(InstallationError::Occupied("/test/link".into())),
            vec!["location", "observe"],
        ),
        (
            CliLink::Missing,
            true,
            false,
            true,
            Ok(CliPlacement::Create),
            vec!["location", "observe", "direct", "admin", "observe"],
        ),
        (
            CliLink::Missing,
            true,
            true,
            true,
            Err(failure()),
            vec!["location", "observe", "direct", "admin"],
        ),
        (
            CliLink::Missing,
            true,
            false,
            false,
            Err(InstallationError::Verification),
            vec!["location", "observe", "direct", "admin", "observe"],
        ),
    ] {
        let port = Arc::new(FakeInstallation {
            link: Mutex::new(existing),
            direct_fails,
            admin_fails,
            admin_verifies,
            ..Default::default()
        });
        let usecase = InstallationUsecase(port.clone());
        // When
        let result = usecase.install_cli(Path::new("/test/link"));
        // Then
        assert_eq!(
            result.map(|(status, path)| {
                assert_eq!(path, Path::new("/test/link"));
                status
            }),
            expected
        );
        assert_eq!(*port.calls.lock(), calls);
    }
}
#[test]
fn test_ログイン項目判定_画面のパスを使いサーバの配置で代用しない() {
    // Given
    let port = Arc::new(FakeInstallation::default());
    let usecase = InstallationUsecase(port);
    // When / Then
    assert_eq!(
        usecase.cli_installation().unwrap(),
        CliInstallation::Allowed
    );
    assert_eq!(
        usecase
            .login_registration(Path::new("/AppTranslocation/gui/releash-desktop"))
            .unwrap(),
        LoginRegistration::Translocated
    );
}

#[test]
fn test_cli設置_観測失敗は変更前に返す() {
    for (stage, calls) in [
        ("executable", vec![]),
        ("location", vec!["location"]),
        ("observe", vec!["location", "observe"]),
    ] {
        // Given
        let port = Arc::new(FakeInstallation {
            failure_stage: Some(stage),
            ..Default::default()
        });
        let usecase = InstallationUsecase(port.clone());
        // When / Then
        assert_eq!(usecase.install_cli(Path::new("/test/link")), Err(failure()));
        assert_eq!(*port.calls.lock(), calls);
    }
}
