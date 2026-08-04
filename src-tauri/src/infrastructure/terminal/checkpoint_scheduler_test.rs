use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use super::DirtyCheckpointScheduler;

#[test]
fn test_ターミナル復元点定期保存_追加出力なしでも未保存状態を保存する() {
    let completed = Arc::new((Mutex::new(0usize), Condvar::new()));
    let observed = Arc::clone(&completed);
    let scheduler = DirtyCheckpointScheduler::spawn(
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
    let scheduler = DirtyCheckpointScheduler::spawn(
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
    let scheduler = DirtyCheckpointScheduler::spawn(
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
    let scheduler = DirtyCheckpointScheduler::spawn(
        Duration::from_millis(20),
        Arc::new(move || {
            let (attempts, changed) = &*observed;
            let mut attempts = attempts.lock().unwrap();
            *attempts += 1;
            changed.notify_all();
            if *attempts == 1 {
                Err("temporary storage failure".to_string())
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
