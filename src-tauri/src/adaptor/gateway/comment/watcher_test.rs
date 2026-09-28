use super::*;
use crate::usecase::comment::ReviewCommentsWatchUsecase;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::TempDir;

fn spawn(
    retrying: Arc<crate::usecase::retry::Retrying>,
    gateway: ReviewCommentsWatchGateway,
    dir: &std::path::Path,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(crate::adaptor::controller::review_comments_watch::run(
        retrying,
        Arc::new(ReviewCommentsWatchUsecase::new(Arc::new(gateway))),
        dir.to_string_lossy().into_owned(),
        Box::pin(crate::infrastructure::timer::ticks(Duration::from_millis(
            100,
        ))),
    ))
}

/// Rule: `review-comments/` 配下の `*.events.json` 変更を検知すると
/// `review-comments-changed` イベントが payload `"*"` で発火する。
#[tokio::test]
async fn emits_review_comments_changed_when_events_json_is_written() {
    let data_dir = TempDir::new().unwrap();
    let dir = data_dir.path().join("review-comments");
    let received: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let _watch = spawn(
        crate::usecase::retry::test_retrying(),
        ReviewCommentsWatchGateway::new(
            dir.clone(),
            Arc::new({
                let received = received.clone();
                move || received.lock().unwrap().push("*".into())
            }),
        ),
        &dir,
    );
    // watcher の watch 開始が反映されるまで少し待つ
    tokio::time::sleep(Duration::from_millis(100)).await;

    let target = dir.join("dummy.events.json");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(4);
    let mut attempt = 0usize;
    loop {
        if !received.lock().unwrap().is_empty() {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "watcher did not emit review-comments-changed within deadline"
        );
        std::fs::write(&target, format!("[{attempt}]")).unwrap();
        attempt += 1;
        tokio::time::sleep(Duration::from_millis(700)).await;
    }

    let payloads = received.lock().unwrap().clone();
    assert!(
        payloads.iter().any(|p| p == "*"),
        "expected wildcard payload, got {payloads:?}"
    );
}

/// Rule: `*.events.json` 以外のファイル変更（例: `.lock` ファイル）は
/// emit 対象にしない。
#[tokio::test]
async fn ignores_non_events_json_changes() {
    let data_dir = TempDir::new().unwrap();
    let dir = data_dir.path().join("review-comments");
    let received: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let _watch = spawn(
        crate::usecase::retry::test_retrying(),
        ReviewCommentsWatchGateway::new(
            dir.clone(),
            Arc::new({
                let received = received.clone();
                move || received.lock().unwrap().push("*".into())
            }),
        ),
        &dir,
    );
    tokio::time::sleep(Duration::from_millis(100)).await;

    std::fs::write(dir.join("dummy.events.lock"), b"lock").unwrap();

    // debounce 後にも emit されないことを確認する。
    tokio::time::sleep(Duration::from_millis(900)).await;
    let payloads = received.lock().unwrap().clone();
    assert!(
        payloads.is_empty(),
        "expected no emit for non-events.json change, got {payloads:?}"
    );
}

#[tokio::test]
async fn test_監視開始の一時失敗_再試行後に変更を検知する() {
    // Given
    use std::sync::atomic::{AtomicUsize, Ordering};
    let directory = TempDir::new().unwrap();
    let dir = directory.path().join("comments");
    let starts = Arc::new(AtomicUsize::new(0));
    let notifications = Arc::new(AtomicUsize::new(0));
    let retrying = crate::usecase::retry::test_retrying();
    // When
    let _watch = spawn(
        retrying.clone(),
        ReviewCommentsWatchGateway::with_start(
            dir.clone(),
            Arc::new({
                let notifications = notifications.clone();
                move || {
                    notifications.fetch_add(1, Ordering::SeqCst);
                }
            }),
            Arc::new({
                let starts = starts.clone();
                move |dir, notify| {
                    let starts = starts.clone();
                    Box::pin(async move {
                        if starts.fetch_add(1, Ordering::SeqCst) == 0 {
                            return Err(background_io::failure(std::io::Error::from(
                                std::io::ErrorKind::WouldBlock,
                            )));
                        }
                        start()(dir, notify).await
                    })
                }
            }),
        ),
        &dir,
    );
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while notifications.load(Ordering::SeqCst) == 0 {
        assert!(
            tokio::time::Instant::now() < deadline,
            "watch did not recover"
        );
        if dir.exists() {
            std::fs::write(dir.join("recovered.events.json"), b"[]").unwrap();
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    // Then
    assert_eq!(starts.load(Ordering::SeqCst), 2);
    let records = retrying.records(&dir.to_string_lossy());
    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0].record.kind,
        crate::usecase::failure::Failure::Technical(
            crate::domain::failure::TechnicalFailureNature::Transient
        )
    );
    assert_eq!(records[0].record.count, 1);
    assert!(!records[0].requires_attention);
}

#[tokio::test]
async fn test_監視開始の期限切れ_処理を終了して資源を返してから同じ対象を再開できる() {
    // Given
    let directory = TempDir::new().unwrap();
    let gateway = Arc::new(ReviewCommentsWatchGateway::new(
        directory.path().join("comments"),
        Arc::new(|| {}),
    ));
    // When / Then
    let expired = gateway.clone();
    super::super::super::shared::background_worker::background_worker_tests::assert_expired_releases(
        async move { expired.ensure_started().await.map_err(WorkFailure::from) },
    )
    .await;
    gateway.ensure_started().await.unwrap();
    gateway.poll().await.unwrap();
}

#[tokio::test]
async fn test_監視pollの期限切れ_監視資源を回収して同じ対象を再開できる() {
    // Given
    let directory = TempDir::new().unwrap();
    let gateway = Arc::new(ReviewCommentsWatchGateway::new(
        directory.path().join("comments"),
        Arc::new(|| {}),
    ));
    gateway.ensure_started().await.unwrap();
    // When / Then
    let expired = gateway.clone();
    super::super::super::shared::background_worker::background_worker_tests::assert_expired_releases(
        async move { expired.poll().await.map_err(WorkFailure::from) },
    )
    .await;
    gateway.ensure_started().await.unwrap();
    gateway.poll().await.unwrap();
}
