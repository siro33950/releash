use super::*;
#[test]
fn test_配置規則_同じ場所からcliとログイン項目の可否を決める() {
    // Given
    for build in [BuildKind::Development, BuildKind::Release] {
        for (path, read_only, login, cli) in [
            (
                "/AppTranslocation/id/Releash.app/releashd",
                false,
                LoginRegistration::Translocated,
                CliInstallation::Translocated,
            ),
            (
                "/AppTranslocation/id/Releash.app/releashd",
                true,
                LoginRegistration::Translocated,
                CliInstallation::Translocated,
            ),
            (
                "/Volumes/Releash/releashd",
                true,
                LoginRegistration::ReadOnly,
                CliInstallation::ReadOnly,
            ),
            (
                "/Applications/Releash.app/releashd",
                false,
                LoginRegistration::Allowed,
                if build == BuildKind::Development {
                    CliInstallation::Development
                } else {
                    CliInstallation::Allowed
                },
            ),
        ] {
            // When
            let location = InstallationLocation::from_path(Path::new(path), read_only, build, true);
            // Then
            assert_eq!(location.login_registration(), login);
            assert_eq!(location.cli_installation(), cli);
            assert_eq!(
                cli.ensure_allowed().is_ok(),
                cli == CliInstallation::Allowed
            );
            if cli != CliInstallation::Allowed {
                assert!(!cli.reason().is_empty());
            }
            if login != LoginRegistration::Allowed {
                assert_eq!(login.reason(), match login {
                    LoginRegistration::Translocated => "Move Releash.app to Applications before enabling Launch at login.",
                    LoginRegistration::ReadOnly => "Releash.app is on a read-only volume. Move it to Applications before enabling Launch at login.",
                    LoginRegistration::Allowed => unreachable!(),
                });
            }
        }
    }
}
#[test]
fn test_配置規則_パス要素だけを判定しmacos以外ではtranslocationを扱わない() {
    // Given / When / Then
    for (path, macos) in [
        ("/AppTranslocation-copy/releashd", true),
        ("/AppTranslocation/id/releashd", false),
    ] {
        assert_eq!(
            InstallationLocation::from_path(Path::new(path), false, BuildKind::Release, macos)
                .cli_installation(),
            CliInstallation::Allowed
        );
    }
}
#[test]
fn test_cli配置_既存ファイルを保護して同じリンクだけを設置済みとする() {
    // Given
    let target = Path::new("/Applications/Releash.app/releash");
    let link = Path::new("/usr/local/bin/releash");
    // When / Then
    assert_eq!(
        CliLink::Symlink(target.into()).placement(target, link),
        Ok(CliPlacement::AlreadyInstalled)
    );
    assert_eq!(
        CliLink::Missing.placement(target, link),
        Ok(CliPlacement::Create)
    );
    assert_eq!(
        CliLink::Symlink("/old/releash".into()).placement(target, link),
        Ok(CliPlacement::Create)
    );
    assert!(matches!(
        CliLink::Other.placement(target, link),
        Err(InstallationError::Occupied(_))
    ));
    for existing in [
        CliLink::Missing,
        CliLink::Other,
        CliLink::Symlink("/old/releash".into()),
    ] {
        assert_eq!(
            existing.verify(target),
            Err(InstallationError::Verification)
        );
    }
    assert!(CliLink::Symlink(target.into()).verify(target).is_ok());
}
