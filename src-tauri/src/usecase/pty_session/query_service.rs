use std::collections::HashSet;

use crate::usecase::pty_session::dto::{
    GetPtyBufferedOutputResult, PtySessionAvailability, PtySessionInfo,
};
use crate::usecase::pty_session::error::UsecaseError;
use crate::usecase::pty_session::ports::PtySessionReadGateway;

pub fn list(manager: &(impl PtySessionReadGateway + ?Sized)) -> Vec<PtySessionInfo> {
    manager
        .list_snapshots()
        .into_iter()
        .map(PtySessionInfo::from)
        .collect()
}

pub fn reconcile_unavailable(
    manager: &(impl PtySessionReadGateway + ?Sized),
    referenced_session_keys: &[String],
) -> PtySessionAvailability {
    let live_session_keys: HashSet<String> = manager
        .list_snapshots()
        .into_iter()
        .map(|snapshot| snapshot.session_key)
        .collect();

    PtySessionAvailability {
        unavailable_session_keys: referenced_session_keys
            .iter()
            .filter(|session_key| !live_session_keys.contains(*session_key))
            .cloned()
            .collect(),
    }
}

pub fn get_buffered_output(
    manager: &(impl PtySessionReadGateway + ?Sized),
    session_key: &str,
    worktree_path: &str,
) -> Result<GetPtyBufferedOutputResult, UsecaseError> {
    let found = manager.find_by_session_key(session_key).ok_or_else(|| {
        UsecaseError::Gateway(format!(
            "PTY session {session_key} not found for worktree {worktree_path}"
        ))
    })?;

    if found.snapshot.worktree_path.as_deref() != Some(worktree_path) {
        return Err(UsecaseError::Gateway(format!(
            "PTY session {session_key} not found for worktree {worktree_path}"
        )));
    }

    Ok(GetPtyBufferedOutputResult {
        pty_id: found.snapshot.pty_id,
        session_key: found.snapshot.session_key,
        buffered_output: found.buffered_output,
        buffered_output_sequence: found.buffered_output_sequence,
        is_exited: found.snapshot.exited,
        exit_code: found.snapshot.exit_code,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::domain::pty_session::entities::PtySessionSnapshot;
    use crate::usecase::pty_session::dto::FoundPtySession;

    struct MockGateway {
        snapshots: Vec<PtySessionSnapshot>,
        outputs: HashMap<String, String>,
    }

    impl MockGateway {
        fn new(snapshots: Vec<PtySessionSnapshot>, outputs: HashMap<String, String>) -> Self {
            Self { snapshots, outputs }
        }
    }

    impl PtySessionReadGateway for MockGateway {
        fn find_by_session_key(&self, session_key: &str) -> Option<FoundPtySession> {
            let snapshot = self
                .snapshots
                .iter()
                .find(|snapshot| snapshot.session_key == session_key)?
                .clone();
            Some(FoundPtySession {
                snapshot,
                buffered_output: self.outputs.get(session_key).cloned().unwrap_or_default(),
                buffered_output_sequence: 5,
            })
        }

        fn list_snapshots(&self) -> Vec<PtySessionSnapshot> {
            self.snapshots.clone()
        }
    }

    fn snapshot(pty_id: u64, session_key: &str, exited: bool) -> PtySessionSnapshot {
        PtySessionSnapshot {
            pty_id,
            session_key: session_key.to_string(),
            worktree_path: Some("/repo".to_string()),
            label: None,
            exited,
            exit_code: None,
        }
    }

    #[test]
    fn reconcile_unavailable_returns_only_referenced_keys_missing_from_live_sessions() {
        let gateway = MockGateway::new(
            vec![
                snapshot(1, "live-a", false),
                snapshot(2, "live-b", false),
                snapshot(3, "unreferenced-live", false),
            ],
            HashMap::new(),
        );

        let availability = reconcile_unavailable(
            &gateway,
            &[
                "live-a".to_string(),
                "missing-a".to_string(),
                "live-b".to_string(),
                "missing-b".to_string(),
            ],
        );

        assert_eq!(
            availability.unavailable_session_keys,
            vec!["missing-a".to_string(), "missing-b".to_string()]
        );
    }

    #[test]
    fn reconcile_unavailable_is_empty_when_all_referenced_sessions_are_live() {
        let gateway = MockGateway::new(
            vec![snapshot(1, "live-a", false), snapshot(2, "live-b", false)],
            HashMap::new(),
        );

        let availability =
            reconcile_unavailable(&gateway, &["live-a".to_string(), "live-b".to_string()]);

        assert!(availability.unavailable_session_keys.is_empty());
    }

    #[test]
    fn reconcile_unavailable_is_empty_without_referenced_sessions() {
        let gateway = MockGateway::new(vec![snapshot(1, "live-a", false)], HashMap::new());

        let availability = reconcile_unavailable(&gateway, &[]);

        assert!(availability.unavailable_session_keys.is_empty());
    }

    #[test]
    fn get_buffered_output_returns_snapshot_for_matching_worktree() {
        let gateway = MockGateway::new(
            vec![snapshot(7, "terminal", false)],
            HashMap::from([("terminal".to_string(), "buffered".to_string())]),
        );

        let output = get_buffered_output(&gateway, "terminal", "/repo").unwrap();

        assert_eq!(output.pty_id, 7);
        assert_eq!(output.session_key, "terminal");
        assert_eq!(output.buffered_output, "buffered");
        assert_eq!(output.buffered_output_sequence, 5);
        assert!(!output.is_exited);
        assert_eq!(output.exit_code, None);
    }

    #[test]
    fn get_buffered_output_rejects_other_worktree() {
        let gateway = MockGateway::new(
            vec![snapshot(7, "terminal", false)],
            HashMap::from([("terminal".to_string(), "buffered".to_string())]),
        );

        let result = get_buffered_output(&gateway, "terminal", "/other");

        assert!(matches!(result, Err(UsecaseError::Gateway(_))));
    }
}
