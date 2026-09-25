use parking_lot::Mutex;
#[derive(Default)]
pub(crate) struct OutputPause {
    paused: std::sync::atomic::AtomicBool,
    workers: Mutex<Vec<std::thread::Thread>>,
}
impl OutputPause {
    pub(crate) fn set(&self, paused: bool) {
        self.paused
            .store(paused, std::sync::atomic::Ordering::Release);
        if !paused {
            for worker in self.workers.lock().iter() {
                worker.unpark();
            }
        }
    }
    pub(crate) fn wait(&self) {
        let current = std::thread::current();
        {
            let mut workers = self.workers.lock();
            if !workers.iter().any(|worker| worker.id() == current.id()) {
                workers.push(current);
            }
        }
        while self.paused.load(std::sync::atomic::Ordering::Acquire) {
            std::thread::park();
        }
    }
}
