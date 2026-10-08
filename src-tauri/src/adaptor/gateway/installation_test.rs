use super::*;
#[test]
fn test_cli観測_ファイルとリンクと不存在を区別する() {
    // Given / When / Then
    assert_eq!(
        decode_cli_link(installation::CliLink::Missing),
        CliLink::Missing
    );
    assert_eq!(
        decode_cli_link(installation::CliLink::Other),
        CliLink::Other
    );
    assert_eq!(
        decode_cli_link(installation::CliLink::Symlink("/target".into())),
        CliLink::Symlink("/target".into())
    );
    assert!(matches!(
        technical(std::io::Error::other("unavailable")),
        InstallationError::Technical(_)
    ));
}

#[cfg(not(target_os = "macos"))]
#[test]
fn test_管理者設置_macos以外では管理者実行の失敗を返す() {
    // Given / When
    let error = LocalInstallationService
        .create_cli_link_as_admin(Path::new("/target"), Path::new("/link"))
        .unwrap_err();
    // Then
    assert_eq!(
        error.to_string(),
        "Administrator CLI installation requires macOS."
    );
}
