use std::collections::{HashMap, HashSet, VecDeque};

use crate::domain::pty_session::entities::{PtySession, PtySessionSnapshot};
use crate::domain::pty_session::PtyLifecycleConfig;

pub struct PtySessionRegistry {
    sessions: HashMap<u64, PtySession>,
    activity: HashMap<u64, u64>,
    pinned: HashSet<String>,
    active_pin_tokens: HashMap<String, ActivePin>,
    retired_active_tokens: HashMap<String, ActivePin>,
    retired_active_token_order: VecDeque<String>,
    reserved_evictions: HashSet<u64>,
    reserved_spawn_total: usize,
    reserved_spawn_by_worktree: HashMap<String, usize>,
    next_pty_id: u64,
    config: PtyLifecycleConfig,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ActivePin {
    worktree_path: String,
    session_key: String,
}

const RETIRED_ACTIVE_TOKEN_CAP: usize = 128;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PtySpawnReservation {
    pub worktree_path: Option<String>,
    pub evict_targets: Vec<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PtySpawnReservationError {
    WorktreeCapReached(String),
    TotalCapReached,
}

impl Default for PtySessionRegistry {
    fn default() -> Self {
        Self {
            sessions: HashMap::new(),
            activity: HashMap::new(),
            pinned: HashSet::new(),
            active_pin_tokens: HashMap::new(),
            retired_active_tokens: HashMap::new(),
            retired_active_token_order: VecDeque::new(),
            reserved_evictions: HashSet::new(),
            reserved_spawn_total: 0,
            reserved_spawn_by_worktree: HashMap::new(),
            next_pty_id: 1,
            config: PtyLifecycleConfig::default(),
        }
    }
}

impl PtySessionRegistry {
    pub fn with_config(config: PtyLifecycleConfig) -> Self {
        Self {
            config,
            ..Self::default()
        }
    }

    pub fn config(&self) -> &PtyLifecycleConfig {
        &self.config
    }

    pub fn next_pty_id(&mut self) -> u64 {
        let pty_id = self.next_pty_id;
        self.next_pty_id += 1;
        pty_id
    }

    pub fn insert(&mut self, session: PtySession) -> Option<PtySession> {
        let pty_id = session.pty_id;
        self.activity.entry(pty_id).or_insert(0);
        self.sessions.insert(pty_id, session)
    }

    pub fn remove(&mut self, pty_id: u64) -> Option<PtySession> {
        self.activity.remove(&pty_id);
        self.reserved_evictions.remove(&pty_id);
        let removed = self.sessions.remove(&pty_id)?;
        self.pinned.remove(&removed.session_key);
        self.active_pin_tokens
            .retain(|_, pin| pin.session_key != removed.session_key);
        self.retired_active_tokens
            .retain(|_, pin| pin.session_key != removed.session_key);
        self.compact_retired_active_token_order();
        Some(removed)
    }

    pub fn get(&self, pty_id: u64) -> Option<&PtySession> {
        self.sessions.get(&pty_id)
    }

    pub fn find_by_session_key(&self, session_key: &str) -> Option<&PtySession> {
        self.sessions
            .values()
            .find(|session| session.session_key == session_key)
    }

    pub fn list_snapshots(&self) -> Vec<PtySessionSnapshot> {
        self.sessions.values().map(PtySession::snapshot).collect()
    }

    pub fn select_kill_targets_by_worktree(&self, worktree_path: &str) -> Vec<u64> {
        self.sessions
            .values()
            .filter(|session| session.worktree_path.as_deref() == Some(worktree_path))
            .map(|session| session.pty_id)
            .collect()
    }

    pub fn select_gc_targets(&self, worktree_path: &str, keep_session_keys: &[String]) -> Vec<u64> {
        self.sessions
            .values()
            .filter(|session| {
                session.worktree_path.as_deref() == Some(worktree_path)
                    && !keep_session_keys.contains(&session.session_key)
            })
            .map(|session| session.pty_id)
            .collect()
    }

    pub fn mark_exited(&mut self, pty_id: u64, exit_code: Option<i32>) -> bool {
        let Some(session) = self.sessions.get_mut(&pty_id) else {
            return false;
        };
        session.mark_exited(exit_code);
        true
    }

    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    #[cfg(test)]
    pub fn count_total_alive(&self) -> usize {
        self.sessions
            .values()
            .filter(|session| !session.exited)
            .count()
    }

    #[cfg(test)]
    pub fn count_alive_for_worktree(&self, worktree_path: &str) -> usize {
        self.sessions
            .values()
            .filter(|session| {
                !session.exited && session.worktree_path.as_deref() == Some(worktree_path)
            })
            .count()
    }

    #[cfg(test)]
    pub fn would_exceed_worktree_cap(&self, worktree_path: &str) -> bool {
        self.count_alive_for_worktree(worktree_path) >= self.config.per_worktree_cap
    }

    #[cfg(test)]
    pub fn would_exceed_total_cap(&self) -> bool {
        self.count_total_alive() >= self.config.max_panes_total
    }

    pub fn reserve_spawn_slot(
        &mut self,
        worktree_path: Option<&str>,
        now_ms: u64,
    ) -> Result<PtySpawnReservation, PtySpawnReservationError> {
        let mut evict_targets = Vec::new();

        if let Some(worktree_path) = worktree_path {
            if self.effective_alive_for_worktree(worktree_path)
                + self.reserved_spawn_for_worktree(worktree_path)
                >= self.config.per_worktree_cap
            {
                let Some(target) = self.select_oldest_idle(now_ms, |session| {
                    session.worktree_path.as_deref() == Some(worktree_path)
                        && !self.reserved_evictions.contains(&session.pty_id)
                }) else {
                    return Err(PtySpawnReservationError::WorktreeCapReached(
                        worktree_path.to_string(),
                    ));
                };
                self.reserved_evictions.insert(target);
                evict_targets.push(target);
            }
        }

        if self.effective_total_alive() + self.reserved_spawn_total >= self.config.max_panes_total {
            let Some(target) = self.select_oldest_idle(now_ms, |session| {
                !self.reserved_evictions.contains(&session.pty_id)
            }) else {
                for target in &evict_targets {
                    self.reserved_evictions.remove(target);
                }
                return Err(PtySpawnReservationError::TotalCapReached);
            };
            self.reserved_evictions.insert(target);
            evict_targets.push(target);
        }

        self.reserved_spawn_total += 1;
        if let Some(worktree_path) = worktree_path {
            *self
                .reserved_spawn_by_worktree
                .entry(worktree_path.to_string())
                .or_insert(0) += 1;
        }

        Ok(PtySpawnReservation {
            worktree_path: worktree_path.map(str::to_string),
            evict_targets,
        })
    }

    pub fn complete_spawn_slot(&mut self, reservation: &PtySpawnReservation) {
        self.release_spawn_slot(reservation);
    }

    pub fn rollback_spawn_slot(&mut self, reservation: &PtySpawnReservation) {
        self.release_spawn_slot(reservation);
    }

    pub fn record_activity(&mut self, pty_id: u64, now_ms: u64) -> bool {
        if !self.sessions.contains_key(&pty_id) {
            return false;
        }
        self.activity.insert(pty_id, now_ms);
        true
    }

    pub fn pin_session_key(&mut self, session_key: &str) {
        if self
            .sessions
            .values()
            .any(|session| session.session_key == session_key)
        {
            self.pinned.insert(session_key.to_string());
        }
    }

    pub fn unpin_session_key_if_unused(&mut self, session_key: &str) {
        self.clear_pin_if_unused(session_key);
    }

    pub fn register_active_terminal(
        &mut self,
        worktree_path: &str,
        session_key: &str,
        active_token: &str,
    ) -> bool {
        if self.retired_active_tokens.remove(active_token).is_some() {
            self.retired_active_token_order
                .retain(|token| token != active_token);
            return false;
        }
        if !self.session_key_belongs_to_worktree(worktree_path, session_key) {
            return false;
        }

        let previous = self.active_pin_tokens.insert(
            active_token.to_string(),
            ActivePin {
                worktree_path: worktree_path.to_string(),
                session_key: session_key.to_string(),
            },
        );
        self.pinned.insert(session_key.to_string());

        if let Some(previous) = previous {
            self.clear_pin_if_unused(&previous.session_key);
        }

        true
    }

    pub fn unregister_active_terminal(
        &mut self,
        worktree_path: &str,
        session_key: &str,
        active_token: &str,
    ) -> bool {
        if self.session_key_belongs_to_worktree(worktree_path, session_key) {
            self.retire_active_token(
                active_token,
                ActivePin {
                    worktree_path: worktree_path.to_string(),
                    session_key: session_key.to_string(),
                },
            );
        }
        let removed = self.active_pin_tokens.get(active_token).is_some_and(|pin| {
            pin.worktree_path == worktree_path && pin.session_key == session_key
        });
        if removed {
            self.active_pin_tokens.remove(active_token);
        }
        self.clear_pin_if_unused(session_key);
        removed
    }

    #[cfg(test)]
    pub fn is_pinned(&self, pty_id: u64) -> bool {
        self.sessions
            .get(&pty_id)
            .is_some_and(|session| self.pinned.contains(&session.session_key))
    }

    #[cfg(test)]
    pub fn select_evictable_for_worktree(&self, worktree_path: &str, now_ms: u64) -> Option<u64> {
        self.select_oldest_idle(now_ms, |session| {
            session.worktree_path.as_deref() == Some(worktree_path)
        })
    }

    pub fn select_idle_timed_out(&self, now_ms: u64) -> Vec<u64> {
        self.sessions
            .values()
            .filter(|session| self.is_session_idle_evictable(session, now_ms))
            .filter(|session| !self.reserved_evictions.contains(&session.pty_id))
            .map(|session| session.pty_id)
            .collect()
    }

    #[cfg(test)]
    pub fn is_idle_evictable(&self, pty_id: u64, now_ms: u64) -> bool {
        self.sessions
            .get(&pty_id)
            .is_some_and(|session| self.is_session_idle_evictable(session, now_ms))
    }

    pub fn snapshot_if_idle_evictable(
        &self,
        pty_id: u64,
        now_ms: u64,
    ) -> Option<PtySessionSnapshot> {
        let session = self.sessions.get(&pty_id)?;
        if !self.is_session_idle_evictable(session, now_ms) {
            return None;
        }
        Some(session.snapshot())
    }

    #[cfg(test)]
    pub fn remove_if_idle_evictable(&mut self, pty_id: u64, now_ms: u64) -> Option<PtySession> {
        if !self.is_idle_evictable(pty_id, now_ms) {
            return None;
        }
        self.remove(pty_id)
    }

    fn release_spawn_slot(&mut self, reservation: &PtySpawnReservation) {
        self.reserved_spawn_total = self.reserved_spawn_total.saturating_sub(1);
        if let Some(worktree_path) = &reservation.worktree_path {
            if let Some(count) = self.reserved_spawn_by_worktree.get_mut(worktree_path) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    self.reserved_spawn_by_worktree.remove(worktree_path);
                }
            }
        }
        for target in &reservation.evict_targets {
            self.reserved_evictions.remove(target);
        }
    }

    fn effective_total_alive(&self) -> usize {
        self.sessions
            .values()
            .filter(|session| !session.exited && !self.reserved_evictions.contains(&session.pty_id))
            .count()
    }

    fn effective_alive_for_worktree(&self, worktree_path: &str) -> usize {
        self.sessions
            .values()
            .filter(|session| {
                !session.exited
                    && session.worktree_path.as_deref() == Some(worktree_path)
                    && !self.reserved_evictions.contains(&session.pty_id)
            })
            .count()
    }

    fn reserved_spawn_for_worktree(&self, worktree_path: &str) -> usize {
        self.reserved_spawn_by_worktree
            .get(worktree_path)
            .copied()
            .unwrap_or(0)
    }

    fn select_oldest_idle(
        &self,
        now_ms: u64,
        predicate: impl Fn(&PtySession) -> bool,
    ) -> Option<u64> {
        self.sessions
            .values()
            .filter(|session| predicate(session))
            .filter(|session| self.is_session_idle_evictable(session, now_ms))
            .min_by_key(|session| self.activity.get(&session.pty_id).copied().unwrap_or(0))
            .map(|session| session.pty_id)
    }

    fn is_session_idle_evictable(&self, session: &PtySession, now_ms: u64) -> bool {
        if session.exited || self.pinned.contains(&session.session_key) {
            return false;
        }
        let last_activity = self.activity.get(&session.pty_id).copied().unwrap_or(0);
        now_ms.saturating_sub(last_activity) >= self.idle_timeout_ms()
    }

    fn session_key_belongs_to_worktree(&self, worktree_path: &str, session_key: &str) -> bool {
        self.sessions.values().any(|session| {
            session.session_key == session_key
                && session.worktree_path.as_deref() == Some(worktree_path)
        })
    }

    fn retire_active_token(&mut self, active_token: &str, pin: ActivePin) {
        let token = active_token.to_string();
        if self
            .retired_active_tokens
            .insert(token.clone(), pin)
            .is_none()
        {
            self.retired_active_token_order.push_back(token);
        }
        while self.retired_active_tokens.len() > RETIRED_ACTIVE_TOKEN_CAP {
            let Some(oldest) = self.retired_active_token_order.pop_front() else {
                break;
            };
            self.retired_active_tokens.remove(&oldest);
        }
    }

    fn compact_retired_active_token_order(&mut self) {
        self.retired_active_token_order
            .retain(|token| self.retired_active_tokens.contains_key(token));
    }

    fn clear_pin_if_unused(&mut self, session_key: &str) {
        if self
            .active_pin_tokens
            .values()
            .any(|pin| pin.session_key == session_key)
        {
            return;
        }
        self.pinned.remove(session_key);
    }

    fn idle_timeout_ms(&self) -> u64 {
        self.config
            .idle_timeout
            .as_millis()
            .min(u128::from(u64::MAX)) as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::pty_session::PtyKind;

    fn session(
        pty_id: u64,
        session_key: &str,
        worktree_path: Option<&str>,
        label: Option<&str>,
    ) -> PtySession {
        PtySession::new(
            pty_id,
            session_key.to_string(),
            worktree_path.map(str::to_string),
            label.map(str::to_string),
            PtyKind::Terminal,
        )
    }

    #[test]
    fn allocates_monotonic_pty_ids() {
        let mut registry = PtySessionRegistry::default();

        assert_eq!(registry.next_pty_id(), 1);
        assert_eq!(registry.next_pty_id(), 2);
        assert_eq!(registry.next_pty_id(), 3);
    }

    #[test]
    fn inserts_finds_and_removes_sessions() {
        let mut registry = PtySessionRegistry::default();
        registry.insert(session(10, "key-1", Some("/repo"), Some("dev")));

        let found = registry.find_by_session_key("key-1").unwrap();
        assert_eq!(found.pty_id, 10);
        assert_eq!(found.label.as_deref(), Some("dev"));

        assert!(registry.find_by_session_key("missing").is_none());
        assert_eq!(registry.remove(10).unwrap().session_key, "key-1");
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn lists_snapshots_without_exposing_mutable_sessions() {
        let mut registry = PtySessionRegistry::default();
        registry.insert(session(1, "key-1", Some("/repo"), Some("dev")));
        registry.insert(session(2, "key-2", Some("/repo2"), None));

        let snapshots = registry.list_snapshots();

        assert_eq!(snapshots.len(), 2);
        assert!(snapshots.iter().any(|snapshot| snapshot.pty_id == 1));
        assert!(snapshots.iter().any(|snapshot| snapshot.pty_id == 2));
    }

    #[test]
    fn selects_kill_targets_by_worktree_only() {
        let mut registry = PtySessionRegistry::default();
        registry.insert(session(1, "key-1", Some("/repo"), Some("dev")));
        registry.insert(session(2, "key-2", Some("/repo"), Some("test")));
        registry.insert(session(3, "key-3", Some("/other"), None));
        registry.insert(session(4, "key-4", None, None));

        let mut targets = registry.select_kill_targets_by_worktree("/repo");
        targets.sort_unstable();

        assert_eq!(targets, vec![1, 2]);
    }

    #[test]
    fn selects_gc_targets_except_keep_keys() {
        let mut registry = PtySessionRegistry::default();
        registry.insert(session(1, "key-1", Some("/repo"), Some("dev")));
        registry.insert(session(2, "key-2", Some("/repo"), Some("test")));
        registry.insert(session(3, "key-3", Some("/other"), None));

        let targets = registry.select_gc_targets("/repo", &[String::from("key-1")]);

        assert_eq!(targets, vec![2]);
    }

    #[test]
    fn mark_exited_updates_session_state() {
        let mut registry = PtySessionRegistry::default();
        registry.insert(session(1, "key-1", Some("/repo"), None));

        assert!(registry.mark_exited(1, Some(42)));
        let snapshot = registry.get(1).unwrap().snapshot();
        assert!(snapshot.exited);
        assert_eq!(snapshot.exit_code, Some(42));
        assert!(!registry.mark_exited(999, None));
    }

    fn lifecycle_config() -> PtyLifecycleConfig {
        PtyLifecycleConfig {
            per_worktree_cap: 2,
            max_panes_total: 3,
            idle_timeout: std::time::Duration::from_millis(100),
            output_buffer_cap: 64 * 1024,
            sweep_interval: std::time::Duration::from_secs(60),
        }
    }

    #[test]
    fn cap_checks_count_alive_sessions_only() {
        let mut registry = PtySessionRegistry::with_config(lifecycle_config());
        registry.insert(session(1, "key-1", Some("/repo"), None));
        registry.insert(session(2, "key-2", Some("/repo"), None));
        registry.insert(session(3, "key-3", Some("/other"), None));

        assert!(registry.would_exceed_worktree_cap("/repo"));
        assert!(registry.would_exceed_total_cap());

        registry.mark_exited(2, Some(0));

        assert!(!registry.would_exceed_worktree_cap("/repo"));
        assert!(!registry.would_exceed_total_cap());
    }

    #[test]
    fn select_evictable_for_worktree_uses_oldest_idle_and_excludes_pinned() {
        let mut registry = PtySessionRegistry::with_config(lifecycle_config());
        registry.insert(session(1, "key-1", Some("/repo"), None));
        registry.insert(session(2, "key-2", Some("/repo"), None));
        registry.insert(session(3, "key-3", Some("/repo"), None));
        registry.record_activity(1, 10);
        registry.record_activity(2, 20);
        registry.record_activity(3, 0);
        registry.pin_session_key("key-3");

        assert_eq!(
            registry.select_evictable_for_worktree("/repo", 150),
            Some(1)
        );
    }

    #[test]
    fn select_evictable_for_worktree_returns_none_when_no_idle_candidate() {
        let mut registry = PtySessionRegistry::with_config(lifecycle_config());
        registry.insert(session(1, "key-1", Some("/repo"), None));
        registry.insert(session(2, "key-2", Some("/repo"), None));
        registry.record_activity(1, 80);
        registry.record_activity(2, 90);

        assert_eq!(registry.select_evictable_for_worktree("/repo", 150), None);
    }

    #[test]
    fn select_idle_timed_out_excludes_pinned_and_boundary_before_timeout() {
        let mut registry = PtySessionRegistry::with_config(lifecycle_config());
        registry.insert(session(1, "key-1", Some("/repo"), None));
        registry.insert(session(2, "key-2", Some("/repo"), None));
        registry.insert(session(3, "key-3", Some("/repo"), None));
        registry.record_activity(1, 50);
        registry.record_activity(2, 51);
        registry.record_activity(3, 0);
        registry.pin_session_key("key-3");

        assert!(registry.select_idle_timed_out(149).is_empty());
        assert_eq!(registry.select_idle_timed_out(150), vec![1]);
    }

    #[test]
    fn is_idle_evictable_revalidates_activity_and_pinned_state() {
        let mut registry = PtySessionRegistry::with_config(lifecycle_config());
        registry.insert(session(1, "key-1", Some("/repo"), None));
        registry.record_activity(1, 0);

        assert!(registry.is_idle_evictable(1, 150));

        registry.record_activity(1, 80);
        assert!(!registry.is_idle_evictable(1, 150));

        registry.record_activity(1, 0);
        registry.pin_session_key("key-1");
        assert!(!registry.is_idle_evictable(1, 150));
    }

    #[test]
    fn remove_if_idle_evictable_revalidates_and_removes_atomically() {
        let mut registry = PtySessionRegistry::with_config(lifecycle_config());
        registry.insert(session(1, "key-1", Some("/repo"), None));
        registry.insert(session(2, "key-2", Some("/repo"), None));
        registry.record_activity(1, 0);
        registry.record_activity(2, 80);

        assert!(registry.remove_if_idle_evictable(2, 150).is_none());
        assert!(registry.get(2).is_some());

        let removed = registry.remove_if_idle_evictable(1, 150).unwrap();

        assert_eq!(removed.pty_id, 1);
        assert!(registry.get(1).is_none());
    }

    #[test]
    fn remove_if_idle_evictable_does_not_remove_pinned_session() {
        let mut registry = PtySessionRegistry::with_config(lifecycle_config());
        registry.insert(session(1, "key-1", Some("/repo"), None));
        registry.record_activity(1, 0);
        registry.pin_session_key("key-1");

        assert!(registry.remove_if_idle_evictable(1, 150).is_none());
        assert!(registry.get(1).is_some());
    }

    #[test]
    fn active_terminal_tokens_pin_until_their_own_token_is_removed() {
        let mut registry = PtySessionRegistry::with_config(lifecycle_config());
        registry.insert(session(1, "key-1", Some("/repo"), None));

        assert!(registry.register_active_terminal("/repo", "key-1", "token-a"));
        assert!(registry.register_active_terminal("/repo", "key-1", "token-b"));
        assert!(registry.is_pinned(1));

        assert!(registry.unregister_active_terminal("/repo", "key-1", "token-a"));
        assert!(registry.is_pinned(1));

        assert!(registry.unregister_active_terminal("/repo", "key-1", "token-b"));
        assert!(!registry.is_pinned(1));
    }

    #[test]
    fn stale_register_after_unregister_is_ignored() {
        let mut registry = PtySessionRegistry::with_config(lifecycle_config());
        registry.insert(session(1, "key-1", Some("/repo"), None));
        registry.pin_session_key("key-1");

        assert!(!registry.unregister_active_terminal("/repo", "key-1", "token-a"));
        assert!(!registry.is_pinned(1));
        assert!(!registry.register_active_terminal("/repo", "key-1", "token-a"));
        assert!(!registry.is_pinned(1));
        assert!(registry.retired_active_tokens.is_empty());
    }

    #[test]
    fn stale_unregister_does_not_clear_new_active_token_for_same_session() {
        let mut registry = PtySessionRegistry::with_config(lifecycle_config());
        registry.insert(session(1, "key-1", Some("/repo"), None));

        assert!(registry.register_active_terminal("/repo", "key-1", "new-token"));
        assert!(!registry.unregister_active_terminal("/repo", "key-1", "old-token"));

        assert!(registry.is_pinned(1));
    }

    #[test]
    fn retired_active_tokens_are_bounded_during_mount_churn() {
        let mut registry = PtySessionRegistry::with_config(lifecycle_config());
        registry.insert(session(1, "key-1", Some("/repo"), None));

        for index in 0..(RETIRED_ACTIVE_TOKEN_CAP * 3) {
            let token = format!("token-{index}");
            assert!(registry.register_active_terminal("/repo", "key-1", &token));
            assert!(registry.unregister_active_terminal("/repo", "key-1", &token));
            assert!(registry.retired_active_tokens.len() <= RETIRED_ACTIVE_TOKEN_CAP);
        }

        assert!(registry.retired_active_tokens.len() <= RETIRED_ACTIVE_TOKEN_CAP);
    }

    #[test]
    fn remove_cleans_retired_active_tokens_for_session() {
        let mut registry = PtySessionRegistry::with_config(lifecycle_config());
        registry.insert(session(1, "key-1", Some("/repo"), None));

        assert!(!registry.unregister_active_terminal("/repo", "key-1", "late-token"));
        assert_eq!(registry.retired_active_tokens.len(), 1);

        registry.remove(1);

        assert!(registry.retired_active_tokens.is_empty());
        assert!(registry.retired_active_token_order.is_empty());
    }

    #[test]
    fn remove_cleans_activity_and_pinned_state() {
        let mut registry = PtySessionRegistry::with_config(lifecycle_config());
        registry.insert(session(1, "key-1", Some("/repo"), None));
        registry.record_activity(1, 1);
        registry.pin_session_key("key-1");

        registry.remove(1);
        registry.insert(session(2, "key-2", Some("/repo"), None));
        registry.record_activity(2, 0);

        assert!(!registry.is_pinned(1));
        assert_eq!(registry.select_idle_timed_out(150), vec![2]);
    }

    #[test]
    fn reserve_spawn_slot_reserves_evict_targets_under_one_registry_lock() {
        let mut registry = PtySessionRegistry::with_config(lifecycle_config());
        registry.insert(session(1, "key-1", Some("/repo"), None));
        registry.insert(session(2, "key-2", Some("/repo"), None));
        registry.record_activity(1, 0);
        registry.record_activity(2, 10);

        let first = registry.reserve_spawn_slot(Some("/repo"), 200).unwrap();
        let second = registry.reserve_spawn_slot(Some("/repo"), 200).unwrap();

        assert_eq!(first.evict_targets, vec![1]);
        assert_eq!(second.evict_targets, vec![2]);
    }

    #[test]
    fn reserve_spawn_slot_rolls_back_pending_spawn_and_reserved_eviction() {
        let mut registry = PtySessionRegistry::with_config(lifecycle_config());
        registry.insert(session(1, "key-1", Some("/repo"), None));
        registry.insert(session(2, "key-2", Some("/repo"), None));
        registry.record_activity(1, 0);
        registry.record_activity(2, 10);

        let first = registry.reserve_spawn_slot(Some("/repo"), 200).unwrap();
        registry.rollback_spawn_slot(&first);
        let second = registry.reserve_spawn_slot(Some("/repo"), 200).unwrap();

        assert_eq!(second.evict_targets, vec![1]);
    }

    #[test]
    fn reserve_spawn_slot_returns_cap_error_when_no_idle_target_exists() {
        let mut registry = PtySessionRegistry::with_config(lifecycle_config());
        registry.insert(session(1, "key-1", Some("/repo"), None));
        registry.insert(session(2, "key-2", Some("/repo"), None));
        registry.record_activity(1, 150);
        registry.record_activity(2, 160);

        assert_eq!(
            registry.reserve_spawn_slot(Some("/repo"), 200),
            Err(PtySpawnReservationError::WorktreeCapReached(
                "/repo".to_string()
            ))
        );
    }

    fn total_cap_config(per_worktree_cap: usize, max_panes_total: usize) -> PtyLifecycleConfig {
        PtyLifecycleConfig {
            per_worktree_cap,
            max_panes_total,
            idle_timeout: std::time::Duration::from_millis(100),
            output_buffer_cap: 64 * 1024,
            sweep_interval: std::time::Duration::from_secs(60),
        }
    }

    #[test]
    fn reserve_spawn_slot_evicts_cross_worktree_oldest_idle_at_total_cap() {
        let mut registry = PtySessionRegistry::with_config(total_cap_config(10, 2));
        registry.insert(session(1, "key-1", Some("/repo"), None));
        registry.insert(session(2, "key-2", Some("/other"), None));
        registry.record_activity(1, 50);
        registry.record_activity(2, 0);

        let reservation = registry.reserve_spawn_slot(Some("/repo"), 200).unwrap();

        assert_eq!(reservation.evict_targets, vec![2]);
    }

    #[test]
    fn reserve_spawn_slot_returns_total_cap_when_total_cap_has_no_idle_candidate() {
        let mut registry = PtySessionRegistry::with_config(total_cap_config(10, 2));
        registry.insert(session(1, "key-1", Some("/repo"), None));
        registry.insert(session(2, "key-2", Some("/other"), None));
        registry.record_activity(1, 150);
        registry.record_activity(2, 160);

        assert_eq!(
            registry.reserve_spawn_slot(Some("/third"), 200),
            Err(PtySpawnReservationError::TotalCapReached)
        );
    }

    #[test]
    fn reserve_spawn_slot_rolls_back_worktree_eviction_when_total_cap_fails() {
        let mut registry = PtySessionRegistry::with_config(total_cap_config(2, 1));
        registry.insert(session(1, "key-1", Some("/repo"), None));
        registry.insert(session(2, "key-2", Some("/repo"), None));
        registry.record_activity(1, 0);
        registry.record_activity(2, 150);

        assert_eq!(
            registry.reserve_spawn_slot(Some("/repo"), 200),
            Err(PtySpawnReservationError::TotalCapReached)
        );

        assert!(registry.reserved_evictions.is_empty());
        assert!(registry.get(1).is_some());
        assert!(registry.get(2).is_some());
    }
}
