use super::*;
use std::sync::Arc;

#[tokio::test]
async fn test_実行context_同期処理へ引き継ぎ次の処理へ漏らさない() {
    let token = tokio_util::sync::CancellationToken::new();
    token.cancel();
    let context = OperationContext::new(None, Arc::new(token));
    let result = scope(context, async { spawn_blocking(check).await.unwrap() }).await;
    assert_eq!(result, Err(OperationStopped::Cancelled));
    assert_eq!(spawn_blocking(check).await.unwrap(), Ok(()));
}

#[test]
fn test_実行context_panic時も元のcontextに戻す() {
    let context = with_timeout(Duration::ZERO);
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        sync_scope(context, || panic!("test"))
    }));
    assert_eq!(check(), Ok(()));
}
