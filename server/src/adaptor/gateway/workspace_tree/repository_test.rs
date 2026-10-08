use super::*;

#[test]
fn test_workspace復元失敗_本番の変換でqueryとcodecの分類を保持する() {
    use crate::adaptor::gateway::local_event_store::{
        reader::storage_unavailable, test_helpers::ReadFailure,
    };
    use crate::adaptor::presenter::connect::classified_error;
    // Given
    for (failure, expected) in ReadFailure::cases() {
        let error = match failure {
            ReadFailure::Query(error) => error,
            ReadFailure::Sqlite(code) => storage_unavailable(&rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(code),
                None,
            )),
        };
        // When / Then
        assert_eq!(
            classified_error(fold_query_error(fact_log::FactReadError::Query(error))).code,
            expected
        );
    }
    assert_eq!(
        classified_error(fold_query_error(fact_log::FactReadError::Corrupt(
            "invalid stored value".into()
        )))
        .code,
        connectrpc::ErrorCode::DataLoss
    );
}
