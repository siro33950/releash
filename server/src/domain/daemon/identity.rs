#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DaemonIdentity {
    pub(crate) daemon_id: String,
    pub(crate) pid: u32,
    pub(crate) process_started_at: u64,
}
