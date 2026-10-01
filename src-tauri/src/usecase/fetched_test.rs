use super::*;

#[test]
fn test_取得の状態_値も失敗もないと取得中を返す() {
    // Given
    let fetched = Fetched::<Vec<u8>>::default();
    // When
    let state = fetched.state(true);
    // Then
    assert_eq!(state, FetchState::Loading);
}
#[test]
fn test_取得の状態_初回の失敗を項目なしと区別する() {
    // Given
    let mut fetched = Fetched::<Vec<u8>>::default();
    // When
    fetched.record(Err(failure("boom")));
    // Then
    assert_eq!(fetched.state(true), FetchState::InitialFailed);
}
#[test]
fn test_取得の状態_空の成功を項目なしとして返す() {
    // Given
    let mut fetched = Fetched::<Vec<u8>>::default();
    // When
    fetched.record(Ok(Vec::new()));
    // Then
    assert_eq!(fetched.state(true), FetchState::Empty);
    assert_eq!(fetched.error, None);
}
#[test]
fn test_取得の状態_値がある成功を取得済みとして返す() {
    // Given
    let mut fetched = Fetched::default();
    // When
    fetched.record(Ok(vec![1]));
    // Then
    assert_eq!(fetched.state(false), FetchState::Ready);
}
#[test]
fn test_取得の状態_失敗しても最後に取れた値を残す() {
    // Given
    let mut fetched = Fetched::ready(vec![1]);
    // When
    fetched.record(Err(failure("boom")));
    // Then
    assert_eq!(fetched.value, Some(vec![1]));
    assert_eq!(fetched.state(false), FetchState::RefreshFailed);
}
#[test]
fn test_取得の状態_成功で失敗を消し値を更新する() {
    // Given
    let mut fetched = Fetched::ready(vec![1]);
    fetched.record(Err(failure("boom")));
    // When
    fetched.record(Ok(vec![2]));
    // Then
    assert_eq!(fetched, Fetched::ready(vec![2]));
}
fn failure(message: &str) -> crate::domain::failure::WorkFailure {
    crate::domain::failure::WorkFailure {
        kind: crate::domain::failure::Failure::Technical(
            crate::domain::failure::TechnicalFailureNature::Other,
        ),
        message: message.into(),
    }
}
