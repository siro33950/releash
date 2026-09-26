use super::*;
use crate::domain::failure::{Failure, TechnicalFailureNature};

#[test]
fn test_バックグラウンド実行の失敗_分類と文面を保持する() {
    // Given
    let error = std::io::Error::new(
        std::io::ErrorKind::UnexpectedEof,
        "background worker exited before replying",
    );
    let message = error.to_string();
    // When
    let failure = failure(error);
    // Then
    assert_eq!(
        failure.kind,
        Failure::Technical(TechnicalFailureNature::Other)
    );
    assert_eq!(failure.message, message);
}
