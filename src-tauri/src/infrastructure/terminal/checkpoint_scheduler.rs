use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::Arc;
use std::time::{Duration, Instant};

type CheckpointFlush = Arc<dyn Fn() -> Result<(), String> + Send + Sync>;

enum CheckpointCommand {
    Wake,
    Flush(mpsc::Sender<Result<(), String>>),
}

#[derive(Clone)]
pub(crate) struct DirtyCheckpointScheduler {
    dirty: Arc<AtomicBool>,
    commands: SyncSender<CheckpointCommand>,
}

impl DirtyCheckpointScheduler {
    pub(crate) fn spawn(interval: Duration, flush: CheckpointFlush) -> Self {
        let dirty = Arc::new(AtomicBool::new(false));
        let (commands, receiver) = mpsc::sync_channel(1);
        let worker_dirty = Arc::clone(&dirty);
        std::thread::spawn(move || {
            let mut last_flush = Instant::now();
            loop {
                match receiver.recv() {
                    Ok(CheckpointCommand::Wake) => loop {
                        if !worker_dirty.load(Ordering::Acquire) {
                            break;
                        }
                        let wait = interval.saturating_sub(last_flush.elapsed());
                        match receiver.recv_timeout(wait) {
                            Ok(CheckpointCommand::Wake) => {}
                            Ok(CheckpointCommand::Flush(completed)) => {
                                worker_dirty.store(false, Ordering::Release);
                                let result = flush();
                                last_flush = Instant::now();
                                let _ = completed.send(result);
                            }
                            Err(RecvTimeoutError::Timeout) => {
                                if worker_dirty.swap(false, Ordering::AcqRel) {
                                    if let Err(error) = flush() {
                                        worker_dirty.store(true, Ordering::Release);
                                        log::error!("failed to persist Terminal Surface: {error}");
                                    }
                                    last_flush = Instant::now();
                                }
                            }
                            Err(RecvTimeoutError::Disconnected) => {
                                if worker_dirty.swap(false, Ordering::AcqRel) {
                                    let _ = flush();
                                }
                                return;
                            }
                        }
                    },
                    Ok(CheckpointCommand::Flush(completed)) => {
                        worker_dirty.store(false, Ordering::Release);
                        let result = flush();
                        last_flush = Instant::now();
                        let _ = completed.send(result);
                    }
                    Err(_) => {
                        if worker_dirty.swap(false, Ordering::AcqRel) {
                            let _ = flush();
                        }
                        return;
                    }
                }
            }
        });
        Self { dirty, commands }
    }

    pub(crate) fn mark_dirty(&self) {
        self.dirty.store(true, Ordering::Release);
        match self.commands.try_send(CheckpointCommand::Wake) {
            Ok(()) | Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {}
        }
    }

    pub(crate) fn flush(&self) -> Result<(), String> {
        let (completed, result) = mpsc::channel();
        self.commands
            .send(CheckpointCommand::Flush(completed))
            .map_err(|_| "Terminal Surface checkpoint worker is unavailable".to_string())?;
        result
            .recv()
            .map_err(|_| "Terminal Surface checkpoint worker stopped before flush".to_string())?
    }
}

#[cfg(test)]
#[path = "checkpoint_scheduler_test.rs"]
mod checkpoint_scheduler_tests;
