use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{atomic::AtomicU64, atomic::Ordering, LazyLock},
};

type SubscriptionTask = (u64, tauri::async_runtime::JoinHandle<()>);
static TASKS: LazyLock<parking_lot::Mutex<HashMap<String, SubscriptionTask>>> =
    LazyLock::new(|| parking_lot::Mutex::new(HashMap::new()));
static NEXT_TASK_ID: AtomicU64 = AtomicU64::new(0);

pub(crate) fn start(id: String, task: Pin<Box<dyn Future<Output = ()> + Send>>) {
    let task_id = NEXT_TASK_ID.fetch_add(1, Ordering::Relaxed);
    let mut tasks = TASKS.lock();
    let task_key = id.clone();
    let handle = tauri::async_runtime::spawn(async move {
        task.await;
        let mut tasks = TASKS.lock();
        if tasks.get(&task_key).is_some_and(|(id, _)| *id == task_id) {
            tasks.remove(&task_key);
        }
    });
    if let Some((_, previous)) = tasks.insert(id, (task_id, handle)) {
        previous.abort();
    }
}

pub(crate) fn stop(id: &str) {
    if let Some((_, handle)) = TASKS.lock().remove(id) {
        handle.abort();
    }
}

pub(crate) fn send<T: serde::Serialize + Clone>(
    channel: &tauri::ipc::Channel<T>,
    value: T,
) -> bool {
    channel.send(value).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    struct DropSignal(Arc<AtomicUsize>);

    impl Drop for DropSignal {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn test_購読停止は状態変化を待たずに繰り返しタスクを終了する() {
        // Given
        let dropped = Arc::new(AtomicUsize::new(0));
        // When
        for index in 0..3 {
            let signal = DropSignal(dropped.clone());
            start(
                format!("desktop_channel_test_{index}"),
                Box::pin(async move {
                    let _signal = signal;
                    std::future::pending::<()>().await;
                }),
            );
            tokio::task::yield_now().await;
            stop(&format!("desktop_channel_test_{index}"));
            for _ in 0..10 {
                if dropped.load(Ordering::SeqCst) == index + 1 {
                    break;
                }
                tokio::task::yield_now().await;
            }
            // Then
            assert_eq!(dropped.load(Ordering::SeqCst), index + 1);
            assert!(!TASKS
                .lock()
                .contains_key(&format!("desktop_channel_test_{index}")));
        }
    }

    #[test]
    fn test_同期コマンド文脈で開始した購読は自然終了後に登録を解放する() {
        // Given
        let (sender, receiver) = std::sync::mpsc::channel();
        let id = "desktop_channel_sync_test".to_string();
        // When
        start(
            id.clone(),
            Box::pin(async move {
                sender.send(()).unwrap();
            }),
        );
        // Then
        receiver
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        for _ in 0..100 {
            if !TASKS.lock().contains_key(&id) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(!TASKS.lock().contains_key(&id));
    }
}
