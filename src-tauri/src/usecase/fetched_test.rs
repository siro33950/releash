use super::*;

#[test]
fn test_取得の状態_初回の取得中と初回の失敗と項目なしを区別する() {
    // Given
    let mut fetched = Fetched::<Vec<u8>>::default();

    // When / Then: まだ取れていない
    assert_eq!(fetched.state(true), FetchState::Loading);
    fetched.record(Err("boom".into()));
    assert_eq!(fetched.state(true), FetchState::InitialFailed);

    // When / Then: 取れた
    fetched.record(Ok(Vec::new()));
    assert_eq!(fetched.state(true), FetchState::Empty);
    assert_eq!(fetched.error, None);
    fetched.record(Ok(vec![1]));
    assert_eq!(fetched.state(false), FetchState::Ready);
}

#[test]
fn test_取得の状態_失敗しても最後に取れた値を残し成功で失敗を消す() {
    // Given
    let mut fetched = Fetched::ready(vec![1]);

    // When
    fetched.record(Err("boom".into()));

    // Then
    assert_eq!(fetched.value, Some(vec![1]));
    assert_eq!(fetched.state(false), FetchState::RefreshFailed);

    // When
    fetched.record(Ok(vec![2]));

    // Then
    assert_eq!(fetched, Fetched::ready(vec![2]));
}
