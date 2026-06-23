use std::collections::HashMap;

use crate::domain::pty_session::entities::{PtySession, PtySessionSnapshot};

pub struct PtySessionRegistry {
    sessions: HashMap<u64, PtySession>,
    next_pty_id: u64,
}

impl Default for PtySessionRegistry {
    fn default() -> Self {
        Self {
            sessions: HashMap::new(),
            next_pty_id: 1,
        }
    }
}

impl PtySessionRegistry {
    pub fn next_pty_id(&mut self) -> u64 {
        let pty_id = self.next_pty_id;
        self.next_pty_id += 1;
        pty_id
    }

    pub fn insert(&mut self, session: PtySession) -> Option<PtySession> {
        self.sessions.insert(session.pty_id, session)
    }

    pub fn remove(&mut self, pty_id: u64) -> Option<PtySession> {
        self.sessions.remove(&pty_id)
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
}
