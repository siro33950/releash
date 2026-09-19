use super::*;
use crate::usecase::cli_install::CliInstallGateway;
#[test]
fn test_cli設置_開発用appで本番リンクを置き換えない() {
    // Given
    let gateway = MacCliInstall;
    // When
    let error = gateway.install().unwrap_err();
    // Then
    assert_eq!(
        error,
        "Install the CLI from a release build of Releash.app."
    );
}
