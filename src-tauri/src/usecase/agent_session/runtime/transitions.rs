use std::collections::{HashMap, HashSet};
#[cfg(test)]
use std::sync::OnceLock;
use std::sync::{Arc, Mutex as StdMutex};

use tokio::sync::{watch, Mutex, OwnedMutexGuard};

#[cfg(test)]
use crate::domain::agent_session::entities::{
    InterruptReason as DomainInterruptReason, TurnResult,
};
use crate::usecase::agent_session::event_log::AgentSessionEvent;
use crate::usecase::agent_session::status::TurnPhase;

use super::session_state::{RuntimeSessionPhase, RuntimeSessionState};
use super::usecase::{
    acquire_session_control_after_recovery, append_user_session_events_blocking,
    emit_session_state_change, ensure_backend_recovery_operation_allowed, required_backend_id,
    start_next_queued_turn, AgentRuntimeError, AgentSessionRuntimeUsecase, StateChange,
};
#[cfg(test)]
use super::usecase::{
    append_session_events_blocking, complete_turn, run_runtime_event_post_actions,
    turn_completion_post_actions, RuntimeContext,
};

#[cfg(test)]
pub(super) const INTERRUPT_FORCE_FINALIZE_DELAY: std::time::Duration =
    std::time::Duration::from_secs(10);

type SessionLock = Arc<Mutex<()>>;

#[derive(Clone, Default)]
pub(super) struct SessionLockMap {
    locks: Arc<Mutex<HashMap<String, SessionLock>>>,
    pending_prune: Arc<StdMutex<HashSet<String>>>,
}

impl SessionLockMap {
    pub(super) async fn acquire(&self, session_id: &str) -> SessionLockGuard {
        let lock = {
            let mut locks = self.locks.lock().await;
            let pending_prune = {
                let mut pending = self
                    .pending_prune
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                std::mem::take(&mut *pending)
            };
            let mut still_referenced = HashSet::new();
            for pending_session_id in pending_prune {
                if locks
                    .get(&pending_session_id)
                    .is_some_and(|lock| Arc::strong_count(lock) == 1 && lock.try_lock().is_ok())
                {
                    locks.remove(&pending_session_id);
                } else if locks.contains_key(&pending_session_id) {
                    still_referenced.insert(pending_session_id);
                }
            }
            if !still_referenced.is_empty() {
                self.pending_prune
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .extend(still_referenced);
            }
            locks
                .entry(session_id.to_string())
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };
        let guard = lock.lock_owned().await;
        SessionLockGuard {
            session_id: session_id.to_string(),
            guard: Some(guard),
            locks: self.clone(),
        }
    }
}

pub(super) struct SessionLockGuard {
    session_id: String,
    guard: Option<OwnedMutexGuard<()>>,
    locks: SessionLockMap,
}

impl Drop for SessionLockGuard {
    fn drop(&mut self) {
        self.guard.take();
        self.locks
            .pending_prune
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(self.session_id.clone());
    }
}

/// Owns the single per-session serialization boundary for durable Stop, resume,
/// and terminal transitions.
#[derive(Clone, Default)]
pub(super) struct SessionTransitionCoordinator {
    locks: SessionLockMap,
}

impl SessionTransitionCoordinator {
    pub(super) async fn acquire(&self, session_id: &str) -> SessionLockGuard {
        self.locks.acquire(session_id).await
    }
}

struct CommandLockEntry {
    lock: SessionLock,
    invalidated: watch::Sender<bool>,
}

type PendingCommandLockPrune = (String, Arc<CommandLockEntry>);

impl CommandLockEntry {
    fn new() -> Self {
        let (invalidated, _) = watch::channel(false);
        Self {
            lock: Arc::new(Mutex::new(())),
            invalidated,
        }
    }
}

/// Serializes commands while a provider call is healthy. A forced terminal
/// transition rotates the entry so waiters can continue without waiting for a
/// provider future that may never return. The old holder remains isolated on
/// the invalidated entry and its generation checks discard any late result.
#[derive(Clone, Default)]
pub(super) struct SessionCommandLocks {
    entries: Arc<Mutex<HashMap<String, Arc<CommandLockEntry>>>>,
    pending_prune: Arc<StdMutex<Vec<PendingCommandLockPrune>>>,
}

impl SessionCommandLocks {
    pub(super) async fn acquire(&self, session_id: &str) -> SessionCommandLockGuard {
        #[cfg(test)]
        let test_owner_reservation = TestSessionRuntimeLockOwnerReservation::reserve(session_id);
        loop {
            let entry = {
                let mut entries = self.entries.lock().await;
                let pending_prune = {
                    let mut pending = self
                        .pending_prune
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    std::mem::take(&mut *pending)
                };
                let mut current_pending = HashMap::new();
                for (pending_session_id, pending_entry) in pending_prune {
                    let is_current = entries
                        .get(&pending_session_id)
                        .is_some_and(|current| Arc::ptr_eq(current, &pending_entry));
                    if is_current {
                        current_pending
                            .entry(pending_session_id)
                            .or_insert(pending_entry);
                    }
                }
                let mut still_referenced = Vec::new();
                for (pending_session_id, pending_entry) in current_pending {
                    if Arc::strong_count(&pending_entry) == 2
                        && pending_entry.lock.try_lock().is_ok()
                    {
                        entries.remove(&pending_session_id);
                    } else {
                        still_referenced.push((pending_session_id, pending_entry));
                    }
                }
                if !still_referenced.is_empty() {
                    self.pending_prune
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .extend(still_referenced);
                }
                entries
                    .entry(session_id.to_string())
                    .or_insert_with(|| Arc::new(CommandLockEntry::new()))
                    .clone()
            };
            let mut invalidated = entry.invalidated.subscribe();
            if *invalidated.borrow() {
                continue;
            }
            let lock = Arc::clone(&entry.lock);
            let guard = tokio::select! {
                guard = lock.lock_owned() => guard,
                _ = invalidated.changed() => continue,
            };
            let is_current = {
                let entries = self.entries.lock().await;
                entries
                    .get(session_id)
                    .is_some_and(|current| Arc::ptr_eq(current, &entry))
            };
            if !is_current || *invalidated.borrow() {
                drop(guard);
                continue;
            }
            return SessionCommandLockGuard {
                session_id: session_id.to_string(),
                guard: Some(guard),
                entry,
                locks: self.clone(),
                #[cfg(test)]
                test_owner_reservation,
            };
        }
    }

    #[cfg(test)]
    pub(super) async fn invalidate(&self, session_id: &str) {
        let previous = {
            let mut entries = self.entries.lock().await;
            entries.insert(session_id.to_string(), Arc::new(CommandLockEntry::new()))
        };
        if let Some(previous) = previous {
            previous.invalidated.send_replace(true);
        }
    }

    #[cfg(test)]
    pub(super) fn is_held_for_test(&self, session_id: &str) -> bool {
        let Ok(entries) = self.entries.try_lock() else {
            return true;
        };
        entries
            .get(session_id)
            .is_some_and(|entry| entry.lock.try_lock().is_err())
    }

    #[cfg(test)]
    pub(super) async fn contains_for_test(&self, session_id: &str) -> bool {
        self.entries.lock().await.contains_key(session_id)
    }

    #[cfg(test)]
    pub(super) async fn len_for_test(&self) -> usize {
        self.entries.lock().await.len()
    }
}

pub struct SessionCommandLockGuard {
    session_id: String,
    guard: Option<OwnedMutexGuard<()>>,
    entry: Arc<CommandLockEntry>,
    locks: SessionCommandLocks,
    #[cfg(test)]
    test_owner_reservation: TestSessionRuntimeLockOwnerReservation,
}

#[cfg(test)]
impl SessionCommandLockGuard {
    pub(crate) fn adopt_for_current_test_flow(&mut self) {
        self.test_owner_reservation.adopt_for_current_flow();
    }
}

impl Drop for SessionCommandLockGuard {
    fn drop(&mut self) {
        self.guard.take();
        self.locks
            .pending_prune
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push((self.session_id.clone(), Arc::clone(&self.entry)));
    }
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum TestSessionRuntimeLockOwner {
    Task(tokio::task::Id),
    Thread(std::thread::ThreadId),
}

#[cfg(test)]
impl TestSessionRuntimeLockOwner {
    fn current() -> Self {
        tokio::task::try_id()
            .map(Self::Task)
            .unwrap_or_else(|| Self::Thread(std::thread::current().id()))
    }
}

#[cfg(test)]
fn held_session_locks() -> &'static StdMutex<HashMap<TestSessionRuntimeLockOwner, String>> {
    static HELD_SESSION_LOCKS: OnceLock<StdMutex<HashMap<TestSessionRuntimeLockOwner, String>>> =
        OnceLock::new();
    HELD_SESSION_LOCKS.get_or_init(|| StdMutex::new(HashMap::new()))
}

#[cfg(test)]
struct TestSessionRuntimeLockOwnerReservation {
    owner: TestSessionRuntimeLockOwner,
    session_id: String,
}

#[cfg(test)]
impl TestSessionRuntimeLockOwnerReservation {
    fn reserve(session_id: &str) -> Self {
        let owner = TestSessionRuntimeLockOwner::current();
        let mut held = held_session_locks()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert!(
            !held.contains_key(&owner),
            "session runtime lock re-entry is forbidden: owner={owner:?}, held={held:?}, requested={session_id}"
        );
        held.insert(owner.clone(), session_id.to_string());
        Self {
            owner,
            session_id: session_id.to_string(),
        }
    }

    fn adopt_for_current_flow(&mut self) {
        let current_owner = TestSessionRuntimeLockOwner::current();
        if current_owner == self.owner {
            return;
        }
        let mut held = held_session_locks()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert_eq!(
            held.get(&self.owner),
            Some(&self.session_id),
            "transferred session runtime lock must retain its acquiring test owner"
        );
        assert!(
            !held.contains_key(&current_owner),
            "session runtime lock transfer target must not already hold a lock: owner={current_owner:?}, held={held:?}"
        );
        held.remove(&self.owner);
        held.insert(current_owner.clone(), self.session_id.clone());
        self.owner = current_owner;
    }
}

#[cfg(test)]
impl Drop for TestSessionRuntimeLockOwnerReservation {
    fn drop(&mut self) {
        let mut held = held_session_locks()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert_eq!(
            held.get(&self.owner),
            Some(&self.session_id),
            "session runtime lock must be released by its acquiring test flow"
        );
        held.remove(&self.owner);
    }
}

impl AgentSessionRuntimeUsecase {
    /// Fence the accepted turn in process memory, then execute only the
    /// provider-side interrupt effect. Durable Stop admission owns all
    /// acceptance/terminal mutations and calls this after its CAS commit.
    ///
    /// The fence must be installed before provider I/O: a late
    /// `BackendSessionCleared` from the old runtime must not reopen and submit
    /// the accepted turn after Stop owns it.
    pub(crate) async fn interrupt_provider_effect_after_stop_acceptance(
        &self,
        session_id: &str,
        turn_id: u64,
    ) -> Result<(), AgentRuntimeError> {
        let durable_queue_paused_at = self
            .ctx
            .session_store
            .load_queue_paused_at(&self.ctx.data_dir, session_id);
        let projected_queue_paused_at = durable_queue_paused_at.as_ref().ok().copied().flatten();
        let runtime = {
            let _runtime_event_guard = self.ctx.runtime_event_locks.acquire(session_id).await;
            let _transition_guard = self.ctx.transitions.acquire(session_id).await;
            let mut sessions = self.ctx.sessions.lock().await;
            let Some(state) = sessions.get_mut(session_id) else {
                return Ok(());
            };
            if state.phase == RuntimeSessionPhase::Idle || state.current_turn_id != Some(turn_id) {
                return Ok(());
            }
            state.queue_paused = true;
            state.queue_paused_at = projected_queue_paused_at
                .or(state.queue_paused_at)
                .or_else(|| Some(crate::usecase::agent_session::session::now_timestamp()));
            state.interrupt_requested_generation = Some(state.generation);
            state.runtime.clone()
        };
        durable_queue_paused_at.map_err(AgentRuntimeError::Other)?;
        if let Some(runtime) = runtime {
            runtime.interrupt().await?;
        }
        Ok(())
    }

    /// Converge the process-local runtime only after the durable Stop Timeout
    /// terminal has won. The runtime epoch fences the detached provider event
    /// pump, while the exact active turn check prevents a delayed cleanup from
    /// closing a newer turn.
    pub(crate) async fn seal_stop_timeout_terminal(&self, session_id: &str, turn_id: u64) {
        let runtime = {
            let _runtime_event_guard = self.ctx.runtime_event_locks.acquire(session_id).await;
            let _transition_guard = self.ctx.transitions.acquire(session_id).await;
            let mut sessions = self.ctx.sessions.lock().await;
            let Some(state) = sessions.get_mut(session_id) else {
                return;
            };
            if state.phase == RuntimeSessionPhase::Idle || state.current_turn_id != Some(turn_id) {
                return;
            }

            let runtime = state.runtime.take();
            state.bump_runtime_epoch();
            state.rollback_started_turn();
            state.terminal_turn_id = Some(turn_id);
            state.interrupt_requested_generation = None;
            runtime
        };
        if let Some(runtime) = runtime {
            runtime.close().await;
        }
    }

    pub async fn interrupt(&self, session_id: &str) -> Result<(), AgentRuntimeError> {
        let durable_driver = self
            .ctx
            .durable_stop_driver
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        if let Some(driver) = durable_driver {
            let meta = self
                .ctx
                .session_store
                .get_session_meta(&self.ctx.data_dir, session_id)
                .map_err(AgentRuntimeError::Other)?
                .ok_or_else(|| {
                    AgentRuntimeError::Other(format!("Session not found: {session_id}"))
                })?;
            let turn_id = {
                let sessions = self.ctx.sessions.lock().await;
                let Some(state) = sessions.get(session_id) else {
                    return Ok(());
                };
                if state.phase == RuntimeSessionPhase::Idle {
                    return Ok(());
                }
                state.current_turn_id.ok_or_else(|| {
                    AgentRuntimeError::Other(format!(
                        "Cannot durably stop a session without an active turn id: {session_id}"
                    ))
                })?
            };
            return driver
                .stop(session_id, turn_id, meta.state_revision)
                .await
                .map_err(AgentRuntimeError::Other);
        }
        #[cfg(not(test))]
        return Err(AgentRuntimeError::Other(
            "Durable Stop driver is not configured".to_string(),
        ));

        #[cfg(test)]
        {
            let transition_guard = self.ctx.transitions.acquire(session_id).await;
            let (runtime, generation, repeated, pause_accepted_at, turn_id) = {
                let mut sessions = self.ctx.sessions.lock().await;
                let Some(state) = sessions.get_mut(session_id) else {
                    return Ok(());
                };
                if state.phase == RuntimeSessionPhase::Idle {
                    return Ok(());
                }
                let generation = state.generation;
                let repeated =
                    state.interrupt_requested_generation == Some(generation) && state.queue_paused;
                let pause_accepted_at =
                    (!repeated).then(crate::usecase::agent_session::session::now_timestamp);
                (
                    state.runtime.clone(),
                    generation,
                    repeated,
                    pause_accepted_at,
                    state.current_turn_id,
                )
            };

            if repeated {
                drop(transition_guard);
                force_finalize_interrupted_turn(&self.ctx, session_id, generation).await;
            } else {
                let at = pause_accepted_at.expect("new interrupt acceptance timestamp");
                let turn_id = turn_id.ok_or_else(|| {
                AgentRuntimeError::Other(format!(
                    "Cannot durably accept interrupt without a current turn id for session {session_id}"
                ))
            })?;
                append_session_events_blocking(
                    &self.ctx,
                    session_id,
                    vec![
                        AgentSessionEvent::TurnInterruptRequested { turn_id, at },
                        AgentSessionEvent::QueuePaused { at },
                    ],
                )
                .await
                .map_err(AgentRuntimeError::Other)?;
                let (phase, pending_permission, permission_revision) = {
                    let mut sessions = self.ctx.sessions.lock().await;
                    let state = sessions.get_mut(session_id).ok_or_else(|| {
                    AgentRuntimeError::Other(format!(
                        "Runtime state disappeared while accepting interrupt for session {session_id}"
                    ))
                })?;
                    if state.phase == RuntimeSessionPhase::Idle || state.generation != generation {
                        return Err(AgentRuntimeError::Other(format!(
                            "Turn changed while accepting interrupt for session {session_id}"
                        )));
                    }
                    state.queue_paused = true;
                    state.queue_paused_at = Some(at);
                    state.interrupt_requested_generation = Some(generation);
                    (
                        TurnPhase::from(state.phase),
                        state.pending_permission_request.clone(),
                        state.pending_permission_state_revision,
                    )
                };
                spawn_interrupt_watchdog_task(
                    &self.ctx,
                    session_id.to_string(),
                    generation,
                    INTERRUPT_FORCE_FINALIZE_DELAY,
                );
                emit_session_state_change(
                    &self.ctx.session_store,
                    &self.ctx.notifier,
                    &self.ctx.status_center,
                    &self.ctx.status_notifier,
                    &self.ctx.data_dir,
                    session_id,
                    StateChange {
                        turn_phase: phase,
                        queue_paused: Some(true),
                        pending_permission_request: pending_permission,
                        pending_permission_state_revision: Some(permission_revision),
                        exit_code: None,
                        completed_at: None,
                        interrupted: false,
                        session_state: None,
                    },
                );
                drop(transition_guard);
            }

            if !repeated {
                if let Some(runtime) = runtime {
                    if let Err(error) = runtime.interrupt().await {
                        log::warn!("agent backend interrupt failed for {session_id}: {error}");
                    }
                }
            }
            Ok(())
        }
    }

    pub async fn resume_queue(&self, session_id: &str) -> Result<(), AgentRuntimeError> {
        let _admission_guard = self.ctx.shutdown_admission.admit()?;
        let _session_guard = acquire_session_control_after_recovery(&self.ctx, session_id).await;
        self.ctx
            .session_store
            .ensure_no_unresolved_recovery(session_id)
            .await
            .map_err(|failure| {
                AgentRuntimeError::Other(format!(
                    "unresolved recovery {} blocks queue resume: {failure}",
                    failure.correlation_id
                ))
            })?;
        ensure_backend_recovery_operation_allowed(&self.ctx, session_id)?;
        let transition_guard = self.ctx.transitions.acquire(session_id).await;
        let session = self
            .ctx
            .session_store
            .get_session_shell(&self.ctx.data_dir, session_id)
            .map_err(AgentRuntimeError::Other)?
            .ok_or_else(|| AgentRuntimeError::Other(format!("Session not found: {session_id}")))?;
        let backend_id = required_backend_id(&session)?;
        let durable_paused_at = self
            .ctx
            .session_store
            .load_queue_paused_at(&self.ctx.data_dir, session_id)
            .map_err(AgentRuntimeError::Other)?;
        let paused_at = {
            let mut sessions = self.ctx.sessions.lock().await;
            let state = sessions.entry(session_id.to_string()).or_insert_with(|| {
                RuntimeSessionState::with_queue_pause(backend_id, durable_paused_at)
            });
            if !state.queue_paused {
                return Ok(());
            }
            state.queue_paused_at.ok_or_else(|| {
                AgentRuntimeError::Other(format!(
                    "Paused queue for session {session_id} is missing its durable revision"
                ))
            })?
        };
        let resumed_at = crate::usecase::agent_session::session::now_timestamp();
        append_user_session_events_blocking(
            &self.ctx,
            session_id,
            vec![AgentSessionEvent::QueueResumed {
                expected_paused_at: paused_at,
                at: resumed_at,
            }],
        )
        .await
        .map_err(AgentRuntimeError::Other)?;
        let (phase, pending_permission, permission_revision, should_start) = {
            let mut sessions = self.ctx.sessions.lock().await;
            let Some(state) = sessions.get_mut(session_id) else {
                return Ok(());
            };
            if !state.queue_paused || state.queue_paused_at != Some(paused_at) {
                return Ok(());
            }
            state.queue_paused = false;
            state.queue_paused_at = None;
            (
                TurnPhase::from(state.phase),
                state.pending_permission_request.clone(),
                state.pending_permission_state_revision,
                state.phase == RuntimeSessionPhase::Idle && !state.pending_queue.is_empty(),
            )
        };
        drop(transition_guard);
        emit_session_state_change(
            &self.ctx.session_store,
            &self.ctx.notifier,
            &self.ctx.status_center,
            &self.ctx.status_notifier,
            &self.ctx.data_dir,
            session_id,
            StateChange {
                turn_phase: phase,
                queue_paused: Some(false),
                pending_permission_request: pending_permission,
                pending_permission_state_revision: Some(permission_revision),
                exit_code: None,
                completed_at: None,
                interrupted: false,
                session_state: None,
            },
        );
        if should_start {
            start_next_queued_turn(&self.ctx, session_id).await;
        }
        Ok(())
    }
}

#[cfg(test)]
pub(super) fn spawn_interrupt_watchdog_task(
    ctx: &RuntimeContext,
    session_id: String,
    generation: u64,
    delay: std::time::Duration,
) {
    let ctx = ctx.clone();
    let spawner = Arc::clone(&ctx.spawner);
    spawner.spawn(Box::pin(async move {
        tokio::time::sleep(delay).await;
        force_finalize_interrupted_turn(&ctx, &session_id, generation).await;
    }));
}

#[cfg(test)]
pub(super) async fn force_finalize_interrupted_turn(
    ctx: &RuntimeContext,
    session_id: &str,
    generation: u64,
) {
    let (runtime, workflow_notification) = {
        let _runtime_event_guard = ctx.runtime_event_locks.acquire(session_id).await;
        let runtime = {
            let mut sessions = ctx.sessions.lock().await;
            let Some(state) = sessions.get_mut(session_id) else {
                return;
            };
            if state.phase == RuntimeSessionPhase::Idle || state.generation != generation {
                return;
            }
            let runtime = state.runtime.take();
            state.bump_runtime_epoch();
            runtime
        };
        let workflow_notification = match complete_turn(
            ctx,
            session_id,
            Some(generation),
            TurnResult::Interrupted {
                reason: DomainInterruptReason::Timeout,
                error: None,
            },
        )
        .await
        {
            Ok(notification) => notification,
            Err(error) => {
                log::warn!("failed to force-finalize interrupted turn for {session_id}: {error}");
                None
            }
        };
        (runtime, workflow_notification)
    };
    ctx.session_locks.invalidate(session_id).await;
    let mut actions = turn_completion_post_actions(ctx, session_id, workflow_notification).await;
    actions.close_runtime(runtime);
    run_runtime_event_post_actions(ctx, session_id, actions).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn invalidation_releases_a_waiter_without_dropping_the_stale_holder() {
        let locks = SessionCommandLocks::default();
        let stale_holder = locks.acquire("session").await;
        let waiting_locks = locks.clone();
        let waiter = tokio::spawn(async move { waiting_locks.acquire("session").await });
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished());

        locks.invalidate("session").await;

        let replacement = tokio::time::timeout(std::time::Duration::from_millis(100), waiter)
            .await
            .expect("waiter should move to the replacement command lock")
            .unwrap();
        drop(replacement);
        drop(stale_holder);
    }

    #[tokio::test]
    async fn transition_coordinator_serializes_the_same_session_only() {
        let coordinator = SessionTransitionCoordinator::default();
        let first = coordinator.acquire("first").await;
        let same_session = {
            let coordinator = coordinator.clone();
            tokio::spawn(async move { coordinator.acquire("first").await })
        };
        let other_session = {
            let coordinator = coordinator.clone();
            tokio::spawn(async move { coordinator.acquire("second").await })
        };

        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), async {
                other_session.await.unwrap()
            })
            .await
            .is_ok()
        );
        assert!(!same_session.is_finished());
        drop(first);
        tokio::time::timeout(std::time::Duration::from_millis(100), same_session)
            .await
            .expect("same-session transition should continue after release")
            .unwrap();
    }
}
