use super::*;
use crate::domain::failure::TechnicalFailureNature;
use crate::usecase::failure::{BusinessFailure, Failure};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Flushes {
    calls: AtomicUsize,
    failures: std::sync::Mutex<Vec<WorkFailure>>,
    release: tokio::sync::Notify,
    entered: tokio::sync::Notify,
    block_first: bool,
}

fn spawn(
    flushes: Arc<Flushes>,
) -> (
    Arc<crate::adaptor::gateway::failure_records::FailureRecordStore>,
    DirtyNotifier,
    tokio::task::JoinHandle<()>,
) {
    let (retrying, store) = crate::test_support::retry::test_retrying_with_store();
    let (dirty, receiver) = dirty_channel();
    let task = tokio::spawn(run(
        retrying.clone(),
        move |_session| {
            let flushes = flushes.clone();
            async move {
                let call = flushes.calls.fetch_add(1, Ordering::SeqCst);
                if call == 0 && flushes.block_first {
                    flushes.entered.notify_one();
                    flushes.release.notified().await;
                }
                match flushes.failures.lock().unwrap().pop() {
                    Some(failure) => Err(failure),
                    None => Ok(()),
                }
            }
        },
        receiver,
        crate::infrastructure::timer::delays(Duration::from_millis(250)),
    ));
    (store, dirty, task)
}

fn flushes(failures: Vec<WorkFailure>, block_first: bool) -> Arc<Flushes> {
    Arc::new(Flushes {
        calls: AtomicUsize::new(0),
        failures: std::sync::Mutex::new(failures),
        release: tokio::sync::Notify::new(),
        entered: tokio::sync::Notify::new(),
        block_first,
    })
}

#[tokio::test(start_paused = true)]
async fn test_ターミナル保存_間隔の後に一度だけ保存し失敗はやり直す() {
    // Given
    let flushes = flushes(
        vec![WorkFailure {
            kind: Failure::Technical(TechnicalFailureNature::Transient),
            message: "temporary storage failure".into(),
        }],
        false,
    );
    let (store, dirty, _task) = spawn(flushes.clone());
    // When
    for _ in 0..100 {
        dirty("terminal");
    }
    tokio::time::sleep(Duration::from_millis(249)).await;
    assert_eq!(flushes.calls.load(Ordering::SeqCst), 0);
    tokio::time::sleep(Duration::from_secs(2)).await;
    // Then
    assert_eq!(flushes.calls.load(Ordering::SeqCst), 2);
    let records = store.records("terminal");
    assert_eq!(records.len(), 1);
    assert!(!records[0].record.active);
}

#[tokio::test(start_paused = true)]
async fn test_ターミナル保存_停止分類の失敗後は新しい出力だけで保存を再開する() {
    for kind in [
        Failure::Technical(TechnicalFailureNature::Other),
        Failure::Business(BusinessFailure::Other),
        Failure::Technical(TechnicalFailureNature::Cancelled),
        Failure::Technical(TechnicalFailureNature::TimedOut),
    ] {
        // Given
        let flushes = flushes(
            vec![WorkFailure {
                kind,
                message: "failed checkpoint".into(),
            }],
            false,
        );
        let (store, dirty, _task) = spawn(flushes.clone());
        dirty("terminal");
        tokio::time::sleep(Duration::from_secs(1)).await;
        // When / Then
        assert_eq!(flushes.calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            store.records("terminal")[0].requires_attention,
            kind != Failure::Technical(TechnicalFailureNature::Cancelled)
        );
        tokio::time::sleep(Duration::from_secs(60)).await;
        assert_eq!(flushes.calls.load(Ordering::SeqCst), 1);
        dirty("terminal");
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert_eq!(flushes.calls.load(Ordering::SeqCst), 2);
        let records = store.records("terminal");
        assert_eq!(records[0].record.count, 1);
        assert!(!records[0].requires_attention);
    }
}

#[tokio::test(start_paused = true)]
async fn test_ターミナル保存_保存中の新しい出力は停止分類の失敗でも失わない() {
    // Given
    let flushes = flushes(
        vec![WorkFailure {
            kind: Failure::Technical(TechnicalFailureNature::Other),
            message: "failed checkpoint".into(),
        }],
        true,
    );
    let (_retrying, dirty, _task) = spawn(flushes.clone());
    dirty("terminal");
    flushes.entered.notified().await;
    // When
    dirty("terminal");
    tokio::task::yield_now().await;
    flushes.release.notify_one();
    tokio::time::sleep(Duration::from_secs(1)).await;
    // Then
    assert_eq!(flushes.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test(start_paused = true)]
async fn test_ターミナル保存_対象ごとに保存し終わった対象を覚え続けない() {
    // Given
    let flushes = flushes(Vec::new(), false);
    let (_retrying, dirty, _task) = spawn(flushes.clone());
    // When
    dirty("first");
    dirty("second");
    tokio::time::sleep(Duration::from_secs(1)).await;
    // Then
    assert_eq!(flushes.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn test_ターミナル保存_偽の遅延で待機中のdirtyを次の保存へ引き継ぐ() {
    // Given
    let (dirty, receiver) = dirty_channel();
    let elapsed = Arc::new(tokio::sync::Notify::new());
    let armed = Arc::new(tokio::sync::Notify::new());
    let (flushed, mut flushes) = tokio::sync::mpsc::unbounded_channel();
    let task = tokio::spawn(run(
        crate::usecase::retry::test_retrying(),
        move |session| {
            let flushed = flushed.clone();
            async move {
                flushed.send(session).unwrap();
                Ok(())
            }
        },
        receiver,
        Arc::new({
            let elapsed = elapsed.clone();
            let armed = armed.clone();
            move || {
                armed.notify_one();
                let elapsed = elapsed.clone();
                Box::pin(futures_util::stream::once(async move {
                    elapsed.notified().await;
                }))
            }
        }),
    ));
    dirty("terminal");
    armed.notified().await;
    assert!(flushes.try_recv().is_err());
    // When
    dirty("terminal");
    tokio::task::yield_now().await;
    elapsed.notify_one();
    assert_eq!(flushes.recv().await.as_deref(), Some("terminal"));
    armed.notified().await;
    assert!(flushes.try_recv().is_err());
    elapsed.notify_one();
    // Then
    assert_eq!(flushes.recv().await.as_deref(), Some("terminal"));
    task.abort();
}

#[tokio::test]
async fn test_ターミナル保存_時刻streamが終わったら保存せず次のdirtyを受け付ける() {
    // Given
    let (dirty, receiver) = dirty_channel();
    let (armed, mut requests) = tokio::sync::mpsc::unbounded_channel();
    let calls = Arc::new(AtomicUsize::new(0));
    let task = tokio::spawn(run(
        crate::usecase::retry::test_retrying(),
        {
            let calls = calls.clone();
            move |_| {
                calls.fetch_add(1, Ordering::SeqCst);
                async { Ok(()) }
            }
        },
        receiver,
        Arc::new(move || {
            armed.send(()).unwrap();
            Box::pin(futures_util::stream::empty())
        }),
    ));
    // When
    dirty("terminal");
    requests.recv().await.unwrap();
    dirty("terminal");
    requests.recv().await.unwrap();
    // Then
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    task.abort();
}
