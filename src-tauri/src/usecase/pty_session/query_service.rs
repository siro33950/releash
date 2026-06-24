use std::sync::Arc;

use crate::domain::pty_session::PtyKind;
use crate::usecase::pty_session::dto::{
    GetPtyBufferedOutputResult, PtyReplayOutput, PtySessionInfo,
};
use crate::usecase::pty_session::error::UsecaseError;
use crate::usecase::pty_session::ports::PtySessionReadGateway;

pub fn list(manager: &impl PtySessionReadGateway) -> Vec<PtySessionInfo> {
    manager
        .list_snapshots()
        .into_iter()
        .map(PtySessionInfo::from)
        .collect()
}

pub fn get_buffered_output(
    manager: &impl PtySessionReadGateway,
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

pub(crate) trait PtySessionReplayReader: Send + Sync {
    fn replay_outputs(&self) -> Vec<PtyReplayOutput>;
}

pub(crate) struct PtySessionReplayQueryService<G> {
    gateway: Arc<G>,
}

impl<G> PtySessionReplayQueryService<G> {
    pub(crate) fn new(gateway: Arc<G>) -> Self {
        Self { gateway }
    }
}

impl<G> PtySessionReplayReader for PtySessionReplayQueryService<G>
where
    G: PtySessionReadGateway + Send + Sync + 'static,
{
    fn replay_outputs(&self) -> Vec<PtyReplayOutput> {
        replay_outputs(self.gateway.as_ref())
    }
}

pub fn replay_outputs(manager: &impl PtySessionReadGateway) -> Vec<PtyReplayOutput> {
    manager
        .list_snapshots()
        .into_iter()
        .filter(|snapshot| !snapshot.exited && snapshot.kind == PtyKind::Terminal)
        .filter_map(|snapshot| {
            let found = manager.find_by_session_key(&snapshot.session_key)?;
            if found.buffered_output.is_empty() {
                return None;
            }
            Some(PtyReplayOutput {
                pty_id: snapshot.pty_id,
                data: found.buffered_output,
                sequence: found.buffered_output_sequence,
            })
        })
        .collect()
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

    fn snapshot(pty_id: u64, session_key: &str, kind: PtyKind, exited: bool) -> PtySessionSnapshot {
        PtySessionSnapshot {
            pty_id,
            session_key: session_key.to_string(),
            worktree_path: Some("/repo".to_string()),
            label: None,
            kind,
            exited,
            exit_code: None,
        }
    }

    #[test]
    fn replay_outputs_include_only_alive_terminal_sessions_with_buffered_output() {
        let gateway = MockGateway::new(
            vec![
                snapshot(1, "terminal", PtyKind::Terminal, false),
                snapshot(2, "empty", PtyKind::Terminal, false),
                snapshot(3, "oneshot", PtyKind::OneShot, false),
                snapshot(4, "exited", PtyKind::Terminal, true),
            ],
            HashMap::from([
                ("terminal".to_string(), "buffered".to_string()),
                ("empty".to_string(), String::new()),
                ("oneshot".to_string(), "ignored".to_string()),
                ("exited".to_string(), "ignored".to_string()),
            ]),
        );

        assert_eq!(
            replay_outputs(&gateway),
            vec![PtyReplayOutput {
                pty_id: 1,
                data: "buffered".to_string(),
                sequence: 5,
            }]
        );
    }

    #[test]
    fn get_buffered_output_returns_snapshot_for_matching_worktree() {
        let gateway = MockGateway::new(
            vec![snapshot(7, "terminal", PtyKind::Terminal, false)],
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
            vec![snapshot(7, "terminal", PtyKind::Terminal, false)],
            HashMap::from([("terminal".to_string(), "buffered".to_string())]),
        );

        let result = get_buffered_output(&gateway, "terminal", "/other");

        assert!(matches!(result, Err(UsecaseError::Gateway(_))));
    }
}
