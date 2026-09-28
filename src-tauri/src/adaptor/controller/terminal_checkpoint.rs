use crate::common::retry::RetryBackoff;
use crate::usecase::failure::FailureKey;
use crate::usecase::failure::WorkFailure;
use crate::usecase::retry::Retrying;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub(crate) const CHECKPOINT_PERSIST_INTERVAL: Duration = Duration::from_millis(250);

pub(crate) type DirtyReceiver = tokio::sync::mpsc::UnboundedReceiver<String>;
pub(crate) type DirtyNotifier = Arc<dyn Fn(&str) + Send + Sync>;

pub(crate) fn dirty_channel() -> (DirtyNotifier, DirtyReceiver) {
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
    (
        Arc::new(move |session_key: &str| {
            let _ = sender.send(session_key.to_string());
        }),
        receiver,
    )
}

#[derive(Default)]
struct Pending {
    dirty: AtomicBool,
    scheduled: AtomicBool,
}

pub(crate) async fn run<F, Fut>(
    retrying: Arc<Retrying>,
    flush: F,
    mut dirty: DirtyReceiver,
    interval: Duration,
) where
    F: Fn(String) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<(), WorkFailure>> + Send,
{
    let flush = Arc::new(flush);
    let sessions: Arc<Mutex<HashMap<String, Arc<Pending>>>> = Default::default();
    while let Some(session_key) = dirty.recv().await {
        let pending = sessions
            .lock()
            .expect("pending checkpoints")
            .entry(session_key.clone())
            .or_default()
            .clone();
        pending.dirty.store(true, Ordering::SeqCst);
        if pending.scheduled.swap(true, Ordering::SeqCst) {
            continue;
        }
        let retrying = retrying.clone();
        let flush = flush.clone();
        let sessions = sessions.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;
                pending.dirty.store(false, Ordering::SeqCst);
                let _ = retrying
                    .restart(
                        FailureKey::new("terminal_checkpoint", &session_key),
                        RetryBackoff::ITEM,
                        |_| flush(session_key.clone()),
                    )
                    .await;
                let mut sessions = sessions.lock().expect("pending checkpoints");
                pending.scheduled.store(false, Ordering::SeqCst);
                if !pending.dirty.load(Ordering::SeqCst) {
                    sessions.remove(&session_key);
                    return;
                }
                if pending.scheduled.swap(true, Ordering::SeqCst) {
                    return;
                }
            }
        });
    }
}

#[cfg(test)]
#[path = "terminal_checkpoint_test.rs"]
mod terminal_checkpoint_tests;
