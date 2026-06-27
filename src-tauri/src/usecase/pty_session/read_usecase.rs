use std::sync::Arc;

use crate::usecase::pty_session::dto::{
    GetPtyBufferedOutputResult, PtySessionAvailability, PtySessionInfo,
};
use crate::usecase::pty_session::error::UsecaseError;
use crate::usecase::pty_session::ports::PtySessionReadGateway;
use crate::usecase::pty_session::query_service;

#[derive(Clone)]
pub(crate) struct PtySessionReadUsecase {
    reader: Arc<dyn PtySessionReadGateway + Send + Sync>,
}

impl PtySessionReadUsecase {
    pub(crate) fn new(reader: Arc<dyn PtySessionReadGateway + Send + Sync>) -> Self {
        Self { reader }
    }

    pub(crate) fn list(&self) -> Vec<PtySessionInfo> {
        query_service::list(self.reader.as_ref())
    }

    pub(crate) fn reconcile_unavailable(
        &self,
        referenced_session_keys: &[String],
    ) -> PtySessionAvailability {
        query_service::reconcile_unavailable(self.reader.as_ref(), referenced_session_keys)
    }

    pub(crate) fn get_buffered_output(
        &self,
        session_key: &str,
        worktree_path: &str,
    ) -> Result<GetPtyBufferedOutputResult, UsecaseError> {
        query_service::get_buffered_output(self.reader.as_ref(), session_key, worktree_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::pty_session::entities::PtySessionSnapshot;
    use crate::domain::pty_session::PtyKind;
    use crate::usecase::pty_session::dto::FoundPtySession;

    struct MockGateway {
        snapshots: Vec<PtySessionSnapshot>,
        buffered_output: String,
        buffered_output_sequence: u64,
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
                buffered_output: self.buffered_output.clone(),
                buffered_output_sequence: self.buffered_output_sequence,
            })
        }

        fn list_snapshots(&self) -> Vec<PtySessionSnapshot> {
            self.snapshots.clone()
        }
    }

    fn snapshot(pty_id: u64, session_key: &str) -> PtySessionSnapshot {
        PtySessionSnapshot {
            pty_id,
            session_key: session_key.to_string(),
            worktree_path: Some("/repo".to_string()),
            label: None,
            kind: PtyKind::Terminal,
            exited: false,
            exit_code: None,
        }
    }

    #[test]
    fn list_uses_injected_read_gateway() {
        let usecase = PtySessionReadUsecase::new(Arc::new(MockGateway {
            snapshots: vec![snapshot(1, "live")],
            buffered_output: String::new(),
            buffered_output_sequence: 0,
        }));

        let sessions = usecase.list();

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].pty_id, 1);
        assert_eq!(sessions[0].session_key, "live");
    }

    #[test]
    fn reconcile_unavailable_uses_injected_read_gateway() {
        let usecase = PtySessionReadUsecase::new(Arc::new(MockGateway {
            snapshots: vec![snapshot(1, "live")],
            buffered_output: String::new(),
            buffered_output_sequence: 0,
        }));

        let availability =
            usecase.reconcile_unavailable(&["live".to_string(), "missing".to_string()]);

        assert_eq!(
            availability.unavailable_session_keys,
            vec!["missing".to_string()]
        );
    }

    #[test]
    fn get_buffered_output_uses_injected_read_gateway() {
        let usecase = PtySessionReadUsecase::new(Arc::new(MockGateway {
            snapshots: vec![snapshot(7, "live")],
            buffered_output: "buffered".to_string(),
            buffered_output_sequence: 9,
        }));

        let output = usecase.get_buffered_output("live", "/repo").unwrap();

        assert_eq!(output.pty_id, 7);
        assert_eq!(output.session_key, "live");
        assert_eq!(output.buffered_output, "buffered");
        assert_eq!(output.buffered_output_sequence, 9);
    }

    #[test]
    fn get_buffered_output_rejects_sessions_from_other_worktrees() {
        let usecase = PtySessionReadUsecase::new(Arc::new(MockGateway {
            snapshots: vec![snapshot(7, "live")],
            buffered_output: "buffered".to_string(),
            buffered_output_sequence: 9,
        }));

        let result = usecase.get_buffered_output("live", "/other");

        assert!(matches!(result, Err(UsecaseError::Gateway(_))));
    }
}
