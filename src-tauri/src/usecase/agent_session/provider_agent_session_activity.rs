use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::domain::agent_session::ProviderAgentTerminalObservationGateway;
use crate::domain::terminal_surface::{TerminalActivity, TERMINAL_ACTIVITY_RUNNING_WINDOW};

/// standalone AgentSession の読み取りモデルに変化があったことを
/// クライアント surface へ通知する port。adaptor 側で実装する。
pub(crate) trait ProviderAgentSessionChangeNotifier: Send + Sync {
    fn provider_agent_session_changed(&self, worktree_path: &str);
}

enum TrackedSurface {
    NotSessionOwned,
    Session(ActivityEntry),
}

struct ActivityEntry {
    worktree_path: String,
    running: bool,
    /// idle→running エッジごとに増える世代。旧世代の watcher を停止させる。
    period: u64,
    last_output_at: tokio::time::Instant,
}

/// terminal surface の出力recencyから activity（running / idle）遷移を検出し、
/// 遷移エッジで変更通知を発火する。分類規則そのものは
/// `TerminalActivity::classify`（domain）に委譲する。
///
/// idle→running は出力到着時に同期検出し、running→idle は
/// 出力毎に基準時刻が更新される debounce watcher（tokio タスク）が検出する。
pub(crate) struct ProviderAgentSessionActivityUsecase {
    terminal: Arc<dyn ProviderAgentTerminalObservationGateway>,
    notifier: Arc<dyn ProviderAgentSessionChangeNotifier>,
    runtime: tokio::runtime::Handle,
    entries: Arc<Mutex<HashMap<String, TrackedSurface>>>,
}

impl ProviderAgentSessionActivityUsecase {
    pub(crate) fn new(
        terminal: Arc<dyn ProviderAgentTerminalObservationGateway>,
        notifier: Arc<dyn ProviderAgentSessionChangeNotifier>,
        runtime: tokio::runtime::Handle,
    ) -> Self {
        Self {
            terminal,
            notifier,
            runtime,
            entries: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// 出力到着の観測。session 所有 surface のみ対象。
    pub(crate) fn observe_output(&self, session_key: &str) {
        let now = tokio::time::Instant::now();
        let edge = {
            let mut entries = lock_entries(&self.entries);
            match entries.get_mut(session_key) {
                Some(TrackedSurface::NotSessionOwned) => return,
                Some(TrackedSurface::Session(entry)) => {
                    entry.last_output_at = now;
                    if entry.running {
                        None
                    } else {
                        entry.running = true;
                        entry.period += 1;
                        Some((entry.worktree_path.clone(), entry.period))
                    }
                }
                None => {
                    let tracked = match self.terminal.session_worktree_path(session_key) {
                        Some(worktree_path) => TrackedSurface::Session(ActivityEntry {
                            worktree_path,
                            running: true,
                            period: 1,
                            last_output_at: now,
                        }),
                        None => TrackedSurface::NotSessionOwned,
                    };
                    let edge = match &tracked {
                        TrackedSurface::Session(entry) => {
                            Some((entry.worktree_path.clone(), entry.period))
                        }
                        TrackedSurface::NotSessionOwned => None,
                    };
                    entries.insert(session_key.to_string(), tracked);
                    edge
                }
            }
        };
        let Some((worktree_path, period)) = edge else {
            return;
        };
        self.notifier.provider_agent_session_changed(&worktree_path);
        self.spawn_idle_watcher(session_key.to_string(), period);
    }

    /// surface の終了観測。追跡エントリを破棄し、running のまま終了した場合は
    /// idle への遷移として通知する。
    pub(crate) fn observe_exit(&self, session_key: &str) {
        let removed = lock_entries(&self.entries).remove(session_key);
        if let Some(TrackedSurface::Session(entry)) = removed {
            if entry.running {
                self.notifier
                    .provider_agent_session_changed(&entry.worktree_path);
            }
        }
    }

    fn spawn_idle_watcher(&self, session_key: String, period: u64) {
        let entries = Arc::clone(&self.entries);
        let notifier = Arc::clone(&self.notifier);
        self.runtime.spawn(async move {
            let mut wait = TERMINAL_ACTIVITY_RUNNING_WINDOW;
            loop {
                tokio::time::sleep(wait).await;
                let transitioned = {
                    let mut tracked = lock_entries(&entries);
                    let Some(TrackedSurface::Session(entry)) = tracked.get_mut(&session_key) else {
                        return;
                    };
                    if entry.period != period || !entry.running {
                        return;
                    }
                    let elapsed = tokio::time::Instant::now() - entry.last_output_at;
                    if TerminalActivity::classify(Some(elapsed)) == TerminalActivity::Running {
                        wait = TERMINAL_ACTIVITY_RUNNING_WINDOW - elapsed;
                        None
                    } else {
                        entry.running = false;
                        Some(entry.worktree_path.clone())
                    }
                };
                if let Some(worktree_path) = transitioned {
                    notifier.provider_agent_session_changed(&worktree_path);
                    return;
                }
            }
        });
    }
}

fn lock_entries(
    entries: &Mutex<HashMap<String, TrackedSurface>>,
) -> std::sync::MutexGuard<'_, HashMap<String, TrackedSurface>> {
    entries
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
#[path = "provider_agent_session_activity_test.rs"]
mod provider_agent_session_activity_tests;
