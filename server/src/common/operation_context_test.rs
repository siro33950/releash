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

#[tokio::test]
async fn test_受け口_期限を超えても処理の完了を待ってから期限切れを返す() {
    // Given
    let (finish, done) = tokio::sync::oneshot::channel();
    let completed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = completed.clone();
    let mut call = Box::pin(ingress(Some(Instant::now()), async move {
        done.await.unwrap();
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
    }));
    // When
    assert!(futures_util::poll!(&mut call).is_pending());
    finish.send(()).unwrap();
    // Then
    assert_eq!(call.await, Err(OperationStopped::Expired));
    assert!(completed.load(std::sync::atomic::Ordering::SeqCst));
    assert_eq!(ingress(None, async { 42 }).await, Ok(42));
}

#[tokio::test]
async fn test_受け口_dropで同期処理へ取り消しを伝える() {
    let (started, ready) = tokio::sync::oneshot::channel();
    let (stopped, done) = tokio::sync::oneshot::channel();
    let call = tokio::spawn(ingress(None, async {
        spawn_blocking(move || {
            let context = current();
            let _ = started.send(());
            let result = sleep(&context, Duration::from_secs(5));
            let _ = stopped.send(result);
        })
        .await
        .unwrap();
    }));
    ready.await.unwrap();
    call.abort();
    assert!(call.await.unwrap_err().is_cancelled());
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), done)
            .await
            .unwrap()
            .unwrap(),
        Err(OperationStopped::Cancelled)
    );
}

#[tokio::test]
async fn test_実行task_dropで内部taskを中止しpanicを返す() {
    let token = tokio_util::sync::CancellationToken::new();
    let signal = token.clone();
    let (started, ready) = tokio::sync::oneshot::channel();
    let mut task = Box::pin(spawned(async move {
        let _guard = signal.drop_guard();
        let _ = started.send(());
        std::future::pending::<()>().await;
    }));
    assert!(futures_util::poll!(&mut task).is_pending());
    ready.await.unwrap();
    drop(task);
    tokio::time::timeout(Duration::from_secs(1), token.cancelled())
        .await
        .unwrap();
    assert!(spawned(async { panic!("test panic") })
        .await
        .unwrap_err()
        .is_panic());
}

#[test]
fn test_同期境界_停止済みなら呼ばず実行後の停止も伝える() {
    let context = with_timeout(Duration::ZERO);
    assert_eq!(
        sync_scope(context.clone(), || before(|| panic!("must not run"))),
        Err(OperationStopped::Expired)
    );
    assert_eq!(
        sync_scope(context, || poll(|| Some(42))),
        Err(OperationStopped::Expired)
    );
    let token = tokio_util::sync::CancellationToken::new();
    let context = OperationContext::new(None, Arc::new(token.clone()));
    assert_eq!(
        sync_scope(context, || checked(|| {
            token.cancel();
            42
        })),
        Err(OperationStopped::Cancelled)
    );
    let mut attempts = 0;
    assert_eq!(
        poll(|| {
            attempts += 1;
            (attempts == 2).then_some(42)
        }),
        Ok(42)
    );
    assert_eq!(attempts, 2);
}

struct Cancelled;
impl Cancellation for Cancelled {
    fn is_cancelled(&self) -> bool {
        true
    }
}

#[test]
fn test_期限_早い方を採り境界で期限切れになる() {
    let now = Instant::now();
    let early = Deadline::new(now + Duration::from_secs(1));
    let late = Deadline::new(now + Duration::from_secs(2));
    assert_eq!(early.minimum(late), early);
    assert_eq!(late.minimum(early), early);
    assert_eq!(early.remaining(now), Duration::from_secs(1));
    assert!(early.is_expired(now + Duration::from_secs(1)));
    assert_eq!(
        early.remaining(now + Duration::from_secs(2)),
        Duration::ZERO
    );
}

#[test]
fn test_停止理由_期限切れと取り消しを区別する() {
    let now = Instant::now();
    let context = OperationContext::new(None, Arc::new(Cancelled));
    assert_eq!(context.check(now), Err(OperationStopped::Cancelled));
    assert_eq!(
        context.with_deadline(Deadline::new(now)).check(now),
        Err(OperationStopped::Expired)
    );
    assert_eq!(OperationContext::default().check(now), Ok(()));
}

#[tokio::test]
async fn test_接続期限_親より長くせず結果と内部contextを保つ() {
    // Given
    let parent = with_timeout(Duration::ZERO);
    // When / Then
    assert_eq!(
        scope(
            parent,
            timeout(Duration::from_secs(10), async {
                panic!("expired operation ran")
            })
        )
        .await,
        Err(OperationStopped::Expired)
    );
    assert_eq!(
        timeout(Duration::from_secs(1), async { Ok::<_, &str>(42) }).await,
        Ok(Ok(42))
    );
    assert_eq!(
        timeout(Duration::from_secs(1), async { Err::<(), _>("io") }).await,
        Ok(Err("io"))
    );
    assert_eq!(
        timeout(Duration::ZERO, async { 42 }).await,
        Err(OperationStopped::Expired)
    );
    assert!(timeout(Duration::from_secs(1), async {
        current().remaining(Instant::now()).is_some()
    })
    .await
    .unwrap());
    assert_eq!(check(), Ok(()));
}

#[test]
fn test_同期接続期限_親期限と取消を維持し終了後はcontextを復元する() {
    // Given
    let expired = with_timeout(Duration::ZERO);
    let cancelled = OperationContext::new(None, Arc::new(Cancelled));
    // When / Then
    for (parent, expected) in [
        (expired, OperationStopped::Expired),
        (cancelled, OperationStopped::Cancelled),
    ] {
        assert_eq!(
            sync_scope(parent, || timeout_sync(Duration::from_secs(1), check)),
            Err(expected)
        );
        assert_eq!(check(), Ok(()));
    }
    assert_eq!(
        timeout_sync(Duration::ZERO, check),
        Err(OperationStopped::Expired)
    );
    assert_eq!(timeout_sync(Duration::from_secs(1), || 42), 42);
}

#[tokio::test]
async fn test_接続期限_期限切れで資源を破棄する() {
    // Given
    let token = tokio_util::sync::CancellationToken::new();
    let guard = token.clone().drop_guard();
    // When
    let result = timeout(Duration::from_millis(20), async move {
        let _guard = guard;
        std::future::pending::<()>().await
    })
    .await;
    // Then
    assert_eq!(result, Err(OperationStopped::Expired));
    assert!(token.is_cancelled());
}
