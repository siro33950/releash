use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::LazyLock;
use std::time::Duration;

use tokio::sync::{Mutex, OwnedMutexGuard};

pub(crate) struct RuntimeCoordinator {
    spawn_locks: Mutex<HashSet<String>>,
    closing_counts: Mutex<HashMap<String, usize>>,
    pending_turns: Mutex<HashSet<String>>,
    session_locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl RuntimeCoordinator {
    fn new() -> Self {
        Self {
            spawn_locks: Mutex::new(HashSet::new()),
            closing_counts: Mutex::new(HashMap::new()),
            pending_turns: Mutex::new(HashSet::new()),
            session_locks: Mutex::new(HashMap::new()),
        }
    }
}

static RUNTIME_COORDINATOR: LazyLock<RuntimeCoordinator> = LazyLock::new(RuntimeCoordinator::new);

pub(crate) struct SessionRuntimeLockGuard {
    chat_session_id: String,
    guard: Option<OwnedMutexGuard<()>>,
}

impl Drop for SessionRuntimeLockGuard {
    fn drop(&mut self) {
        self.guard.take();
        let chat_session_id = self.chat_session_id.clone();
        tokio::spawn(async move {
            prune_session_runtime_lock(&chat_session_id).await;
        });
    }
}

pub(crate) async fn acquire_session_runtime_lock(chat_session_id: &str) -> SessionRuntimeLockGuard {
    let lock = {
        let mut locks = RUNTIME_COORDINATOR.session_locks.lock().await;
        locks
            .entry(chat_session_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    };
    let guard = lock.lock_owned().await;
    SessionRuntimeLockGuard {
        chat_session_id: chat_session_id.to_string(),
        guard: Some(guard),
    }
}

pub(crate) async fn prune_session_runtime_lock(chat_session_id: &str) {
    let mut locks = RUNTIME_COORDINATOR.session_locks.lock().await;
    if locks
        .get(chat_session_id)
        .is_some_and(|lock| Arc::strong_count(lock) == 1)
    {
        locks.remove(chat_session_id);
    }
}

pub(crate) async fn mark_session_closing(chat_session_id: &str) {
    let mut closing = RUNTIME_COORDINATOR.closing_counts.lock().await;
    *closing.entry(chat_session_id.to_string()).or_insert(0) += 1;
}

pub(crate) async fn clear_session_closing(chat_session_id: &str) {
    let mut closing = RUNTIME_COORDINATOR.closing_counts.lock().await;
    match closing.get_mut(chat_session_id) {
        Some(count) if *count > 1 => *count -= 1,
        Some(_) => {
            closing.remove(chat_session_id);
        }
        None => {}
    }
}

pub(crate) async fn wait_until_session_close_finished(chat_session_id: &str) {
    loop {
        if !RUNTIME_COORDINATOR
            .closing_counts
            .lock()
            .await
            .contains_key(chat_session_id)
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// 指定 session_id が現在 close 中（mark_session_closing 後 clear_session_closing 前）か返す。
/// engine 層の retry / spawn 判断で「ユーザーが閉じた直後の bridge を再起動しない」ガードに使う。
pub(crate) async fn is_session_closing(chat_session_id: &str) -> bool {
    RUNTIME_COORDINATOR
        .closing_counts
        .lock()
        .await
        .contains_key(chat_session_id)
}

pub(crate) async fn mark_pending_turn_starting(chat_session_id: &str) {
    RUNTIME_COORDINATOR
        .pending_turns
        .lock()
        .await
        .insert(chat_session_id.to_string());
}

pub(crate) async fn clear_pending_turn_starting(chat_session_id: &str) {
    RUNTIME_COORDINATOR
        .pending_turns
        .lock()
        .await
        .remove(chat_session_id);
}

pub(crate) async fn is_pending_turn_starting(chat_session_id: &str) -> bool {
    RUNTIME_COORDINATOR
        .pending_turns
        .lock()
        .await
        .contains(chat_session_id)
}

pub(crate) struct SpawnSessionGuard {
    chat_session_id: String,
}

impl Drop for SpawnSessionGuard {
    fn drop(&mut self) {
        let chat_session_id = self.chat_session_id.clone();
        tokio::spawn(async move {
            RUNTIME_COORDINATOR
                .spawn_locks
                .lock()
                .await
                .remove(&chat_session_id);
        });
    }
}

pub(crate) async fn acquire_spawn_session_guard(chat_session_id: &str) -> SpawnSessionGuard {
    loop {
        {
            let mut spawning = RUNTIME_COORDINATOR.spawn_locks.lock().await;
            if spawning.insert(chat_session_id.to_string()) {
                return SpawnSessionGuard {
                    chat_session_id: chat_session_id.to_string(),
                };
            }
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[cfg(test)]
pub(crate) async fn session_runtime_lock_exists(chat_session_id: &str) -> bool {
    RUNTIME_COORDINATOR
        .session_locks
        .lock()
        .await
        .contains_key(chat_session_id)
}
