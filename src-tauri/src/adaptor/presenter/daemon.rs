pub(crate) fn server_info(info: crate::domain::daemon::DaemonInfo) -> super::client::ServerInfo {
    use super::client::ServingStatus as W;
    use crate::domain::daemon::ServingStatus as S;
    super::client::ServerInfo {
        release: info.release,
        daemon_id: info.identity.daemon_id,
        pid: info.identity.pid,
        process_started_at: info.identity.process_started_at,
        protocol: info.protocol,
        capabilities: info.capabilities.into_iter().collect(),
        serving_status: match info.serving_status {
            S::Starting => W::Starting,
            S::Serving => W::Serving,
            S::Stopping => W::Stopping,
            S::Stopped => W::Stopped,
            S::Failed(_) => W::Failed,
        } as i32,
    }
}
