use super::*;
use crate::cli::common::{cli_error_exit_code, cli_error_stderr};

#[test]
fn test_local_api_status_errorのcli分類を維持する() {
    // Given
    let invalid_statuses = [400, 409, 422];

    // When
    let not_found = api_error(404, Some("missing"));
    let invalid = invalid_statuses.map(|status| api_error(status, Some("invalid")));
    let unauthorized = api_error(401, Some("unauthorized"));

    // Then
    assert!(matches!(not_found, CliError::NotFound(_)));
    assert!(invalid
        .into_iter()
        .all(|error| matches!(error, CliError::InvalidInput(_))));
    assert!(matches!(unauthorized, CliError::Other(_)));
}

#[test]
fn test_local_api_discovery_process情報参照不能をcatch_allで終了コード1にする() {
    // Given / When
    let error = discovery_error(LocalApiClientError::ProcessInformationUnavailable);

    // Then
    assert_eq!(
        error,
        CliError::Other(
            "プロセス情報を参照できないため、local API の接続先を確認できませんでした".to_string()
        )
    );
    assert_eq!(cli_error_exit_code(&error), 1);
    assert_eq!(
        cli_error_stderr(&error),
        "error: プロセス情報を参照できないため、local API の接続先を確認できませんでした"
    );
}
