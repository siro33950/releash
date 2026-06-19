use crate::usecase::pty_session::dto::PtySessionInfo;
use crate::usecase::pty_session::ports::PtySessionGateway;

pub fn list(manager: &impl PtySessionGateway) -> Vec<PtySessionInfo> {
    manager
        .list_snapshots()
        .into_iter()
        .map(PtySessionInfo::from)
        .collect()
}
