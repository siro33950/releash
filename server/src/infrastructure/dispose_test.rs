mod dispose_tests {
    use super::super::*;
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
