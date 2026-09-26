use crate::domain::failure::{BusinessFailure, Failure, TechnicalFailureNature};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use super::DirtyCheckpointScheduler;

#[test]
fn test_ターミナル復元点定期保存_追加出力なしでも未保存状態を保存する() {
    let completed = Arc::new((Mutex::new(0usize), Condvar::new()));
    let observed = Arc::clone(&completed);
    let scheduler = spawn_scheduler(
        crate::usecase::work_queue::shared().clone(),
        uuid::Uuid::new_v4().to_string(),
        Duration::from_millis(20),
        Arc::new(move || {
            let (count, changed) = &*observed;
            *count.lock().unwrap() += 1;
            changed.notify_all();
            Ok(())
        }),
    );

    scheduler.mark_dirty();

    let (count, changed) = &*completed;
    let count = changed
        .wait_timeout_while(count.lock().unwrap(), Duration::from_secs(2), |count| {
            *count == 0
        })
        .unwrap()
        .0;
    assert_eq!(*count, 1);
}

#[test]
fn test_ターミナル復元点定期保存_遅い書込中も未保存通知を停止しない() {
    let started = Arc::new((Mutex::new(false), Condvar::new()));
    let release = Arc::new((Mutex::new(false), Condvar::new()));
    let observed_started = Arc::clone(&started);
    let observed_release = Arc::clone(&release);
    let scheduler = spawn_scheduler(
        crate::usecase::work_queue::shared().clone(),
        uuid::Uuid::new_v4().to_string(),
        Duration::ZERO,
        Arc::new(move || {
            let (started, changed) = &*observed_started;
            *started.lock().unwrap() = true;
            changed.notify_all();
            let (released, changed) = &*observed_release;
            let _guard = changed
                .wait_while(released.lock().unwrap(), |released| !*released)
                .unwrap();
            Ok(())
        }),
    );
    scheduler.mark_dirty();
    let (has_started, changed) = &*started;
    let _guard = changed
        .wait_while(has_started.lock().unwrap(), |started| !*started)
        .unwrap();

    let (sent, received) = mpsc::channel();
    let concurrent = scheduler.clone();
    std::thread::spawn(move || {
        concurrent.mark_dirty();
        sent.send(()).unwrap();
    });
    received
        .recv_timeout(Duration::from_secs(1))
        .expect("mark_dirty must not wait for checkpoint I/O");

    let (released, changed) = &*release;
    *released.lock().unwrap() = true;
    changed.notify_all();
}

#[test]
fn test_ターミナル復元点明示保存_復帰前に保留状態を永続化する() {
    let count = Arc::new(Mutex::new(0usize));
    let observed = Arc::clone(&count);
    let scheduler = spawn_scheduler(
        crate::usecase::work_queue::shared().clone(),
        uuid::Uuid::new_v4().to_string(),
        Duration::from_secs(60),
        Arc::new(move || {
            *observed.lock().unwrap() += 1;
            Ok(())
        }),
    );
    for _ in 0..100 {
        scheduler.mark_dirty();
    }

    scheduler.flush().unwrap();

    assert_eq!(*count.lock().unwrap(), 1);
}

#[test]
fn test_ターミナル復元点定期保存_失敗後は追加出力なしで再試行する() {
    let attempts = Arc::new((Mutex::new(0usize), Condvar::new()));
    let observed = Arc::clone(&attempts);
    let scheduler = spawn_scheduler(
        crate::usecase::work_queue::shared().clone(),
        uuid::Uuid::new_v4().to_string(),
        Duration::from_millis(20),
        Arc::new(move || {
            let (attempts, changed) = &*observed;
            let mut attempts = attempts.lock().unwrap();
            *attempts += 1;
            changed.notify_all();
            if *attempts == 1 {
                Err(crate::usecase::work_queue::WorkFailure {
                    kind: Failure::Technical(TechnicalFailureNature::Transient),
                    message: "temporary storage failure".to_string(),
                })
            } else {
                Ok(())
            }
        }),
    );

    scheduler.mark_dirty();

    let (attempts, changed) = &*attempts;
    let attempts = changed
        .wait_timeout_while(
            attempts.lock().unwrap(),
            Duration::from_secs(2),
            |attempts| *attempts < 2,
        )
        .unwrap()
        .0;
    assert_eq!(*attempts, 2);
}

fn spawn_scheduler(
    queue: Arc<crate::usecase::work_queue::WorkQueueUsecase>,
    target: String,
    interval: Duration,
    flush: Arc<dyn Fn() -> Result<(), crate::usecase::work_queue::WorkFailure> + Send + Sync>,
) -> DirtyCheckpointScheduler {
    let background = flush.clone();
    DirtyCheckpointScheduler::spawn(
        queue,
        target,
        interval,
        flush,
        Arc::new(move || {
            let background = background.clone();
            Box::pin(async move { background() })
        }),
    )
}

#[tokio::test(start_paused = true)]
async fn test_ターミナル復元点_停止分類の失敗後は新しい出力だけで保存を再開する() {
    use crate::usecase::work_queue::WorkFailure;
    use std::sync::atomic::{AtomicUsize, Ordering};

    for kind in [
        Failure::Technical(TechnicalFailureNature::Other),
        Failure::Business(BusinessFailure::Other),
        Failure::Technical(TechnicalFailureNature::Cancelled),
        Failure::Technical(TechnicalFailureNature::TimedOut),
    ] {
        // Given
        let queue = crate::usecase::work_queue::work_queue_tests::queue();
        let output = Arc::new(AtomicUsize::new(1));
        let saved = Arc::new(AtomicUsize::new(0));
        let attempts = Arc::new(AtomicUsize::new(0));
        let scheduler = spawn_scheduler(
            queue.clone(),
            "terminal".into(),
            Duration::from_millis(250),
            {
                let output = output.clone();
                let saved = saved.clone();
                let attempts = attempts.clone();
                Arc::new(move || {
                    if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                        Err(WorkFailure {
                            kind,
                            message: "failed checkpoint".into(),
                        })
                    } else {
                        saved.store(output.load(Ordering::SeqCst), Ordering::SeqCst);
                        Ok(())
                    }
                })
            },
        );
        scheduler.mark_dirty();
        tokio::time::sleep(Duration::from_secs(1)).await;
        // When / Then
        assert_eq!(crate::usecase::work_queue::next_attempt(kind), None);
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
        assert_eq!(
            queue.records("terminal").await[0].requires_attention,
            kind != Failure::Technical(TechnicalFailureNature::Cancelled)
        );
        tokio::time::sleep(Duration::from_secs(60)).await;
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
        output.store(2, Ordering::SeqCst);
        scheduler.mark_dirty();
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
        assert_eq!(saved.load(Ordering::SeqCst), 2);
        let records = queue.records("terminal").await;
        assert_eq!(records[0].record.count, 1);
        assert!(!records[0].requires_attention);
    }
}

#[tokio::test(start_paused = true)]
async fn test_ターミナル復元点_保存中の新しい出力は停止分類の失敗でも失わない() {
    use crate::usecase::work_queue::WorkFailure;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // Given
    let queue = crate::usecase::work_queue::work_queue_tests::queue();
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let attempts = Arc::new(AtomicUsize::new(0));
    let scheduler = DirtyCheckpointScheduler::spawn(
        queue,
        "terminal".into(),
        Duration::ZERO,
        Arc::new(|| Ok(())),
        {
            let entered = entered.clone();
            let release = release.clone();
            let attempts = attempts.clone();
            Arc::new(move || {
                let entered = entered.clone();
                let release = release.clone();
                let first = attempts.fetch_add(1, Ordering::SeqCst) == 0;
                Box::pin(async move {
                    if first {
                        entered.notify_one();
                        release.notified().await;
                        Err(WorkFailure {
                            kind: Failure::Technical(TechnicalFailureNature::Other),
                            message: "failed checkpoint".into(),
                        })
                    } else {
                        Ok(())
                    }
                })
            })
        },
    );
    scheduler.mark_dirty();
    entered.notified().await;
    // When
    scheduler.mark_dirty();
    tokio::task::yield_now().await;
    release.notify_one();
    tokio::time::sleep(Duration::from_secs(1)).await;
    // Then
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
}
