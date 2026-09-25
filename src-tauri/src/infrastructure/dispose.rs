//! ブロッキングし得る資源破棄を呼び出し元スレッドから隔離するユーティリティ。

/// `value` の drop を使い捨てスレッドで実行する。
///
/// FSEvents watcher の drop は run loop の停止待ちで長時間（最悪無期限に）ブロック
/// し得るため、メインスレッドや async worker 上では実行しない（#1641）。
/// スレッド生成に失敗した場合のみ、その場で drop する。
pub fn dispose_in_background<T: Send + 'static>(thread_name: &str, value: T) {
    let _ = std::thread::Builder::new()
        .name(thread_name.to_string())
        .spawn(move || drop(value));
}

#[cfg(test)]
mod dispose_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_dispose_はdropを別スレッドで実行する() {
        struct Probe {
            tx: std::sync::mpsc::Sender<std::thread::ThreadId>,
        }

        impl Drop for Probe {
            fn drop(&mut self) {
                let _ = self.tx.send(std::thread::current().id());
            }
        }

        let (tx, rx) = std::sync::mpsc::channel();

        dispose_in_background("test-dispose", Probe { tx });

        let drop_thread = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("drop が実行されること");
        assert_ne!(drop_thread, std::thread::current().id());
    }
}
