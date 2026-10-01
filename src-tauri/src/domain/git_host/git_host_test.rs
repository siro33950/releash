use super::*;

#[test]
fn test_取得結果_失敗しても最後の成功値を保持する() {
    // Given
    let mut result = CachedResult::default();
    result.record(Ok(vec![1]));
    let error = GitHostError::External("offline".into());
    // When
    result.record(Err(error.clone()));
    // Then
    assert_eq!(result.value, Some(vec![1]));
    assert_eq!(result.error, Some(error));
}
#[test]
fn test_取得結果_成功すると失敗を解除して値を更新する() {
    // Given
    let mut result = CachedResult::default();
    result.record(Ok(vec![1]));
    result.record(Err(GitHostError::External("offline".into())));
    // When
    result.record(Ok(vec![2]));
    // Then
    assert_eq!(result.value, Some(vec![2]));
    assert_eq!(result.error, None);
}
