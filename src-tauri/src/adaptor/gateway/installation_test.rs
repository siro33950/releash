use super::*;
#[test]
fn test_cli観測_ファイルとリンクと不存在を区別する() {
    // Given / When / Then
    assert_eq!(decode_cli_link(None), CliLink::Missing);
    assert_eq!(decode_cli_link(Some(None)), CliLink::Other);
    assert_eq!(
        decode_cli_link(Some(Some("/target".into()))),
        CliLink::Symlink("/target".into())
    );
    assert!(matches!(
        technical(std::io::Error::other("unavailable")),
        InstallationError::Technical(_)
    ));
}
