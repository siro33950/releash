use super::*;

#[test]
fn test_計測の包み_有効時だけ成功と失敗を記録し結果を保持する() {
    for enabled in [false, true] {
        for result in [Ok(42), Err("failure")] {
            let mut observed = None;
            assert_eq!(
                measure_result(
                    enabled,
                    "test",
                    "test",
                    || result,
                    |success, _| observed = Some(success)
                ),
                result
            );
            assert_eq!(observed, enabled.then_some(result.is_ok()));
            assert_eq!(
                observe_result(|| result, |value, _| assert_eq!(*value, result)),
                result
            );
        }
    }
}

#[tokio::test]
async fn test_非同期計測_完了結果を一度だけ観測する() {
    let mut count = 0;
    let result: Result<(), &str> = observe_result_async(async { Err("failure") }, |result, _| {
        assert_eq!(*result, Err("failure"));
        count += 1;
    })
    .await;
    assert_eq!(result, Err("failure"));
    assert_eq!(count, 1);
}
