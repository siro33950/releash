use crate::wire::ServerInfo;
use serde::{Deserialize, Serialize};
use std::path::Path;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LocalApiDiscovery {
    pub port: u16,
    pub token: String,
    pub instance_id: String,
    pub pid: u32,
    pub process_started_at: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessStartTimeLookup {
    pub process_list_available: bool,
    pub start_time: Option<u64>,
}

pub fn process_start_time(pid: u32) -> Option<u64> {
    lookup_process_start_time(pid)
        .start_time
        .filter(|start_time| *start_time != 0)
}

pub fn lookup_process_start_time(pid: u32) -> ProcessStartTimeLookup {
    let pid = Pid::from_u32(pid);
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing(),
    );
    let start_time = system.process(pid).map(|process| process.start_time());
    if start_time.is_some_and(|start_time| start_time != 0) {
        return ProcessStartTimeLookup {
            process_list_available: true,
            start_time,
        };
    }

    system.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
    ProcessStartTimeLookup {
        process_list_available: !system.processes().is_empty(),
        start_time: system.process(pid).map(|process| process.start_time()),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DiscoveryReadError {
    #[error("client discovery is unreadable: {0}")]
    Read(#[source] std::io::Error),
    #[error("client discovery is invalid: {0}")]
    Decode(#[source] serde_json::Error),
}

pub fn read_optional(data_dir: &Path) -> Result<Option<LocalApiDiscovery>, DiscoveryReadError> {
    let bytes = match std::fs::read(data_dir.join("client-api.json")) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(DiscoveryReadError::Read(error)),
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(DiscoveryReadError::Decode)
}

pub fn read(data_dir: &Path) -> Result<LocalApiDiscovery, connectrpc::ConnectError> {
    read_optional(data_dir)
        .map_err(|error| unavailable(error.to_string()))?
        .ok_or_else(|| unavailable("client discovery is unavailable"))
}

impl LocalApiDiscovery {
    pub fn verify_process(
        &self,
        lookup: impl FnOnce(u32) -> ProcessStartTimeLookup,
    ) -> Result<(), connectrpc::ConnectError> {
        if self.port == 0
            || self.token.trim().is_empty()
            || self.instance_id.trim().is_empty()
            || self.pid == 0
            || self.process_started_at == 0
        {
            return Err(unavailable("client discovery is invalid"));
        }
        let process = lookup(self.pid);
        if !process.process_list_available {
            return Err(unavailable("process information is unavailable"));
        }
        if process.start_time != Some(self.process_started_at) {
            return Err(unavailable("client discovery is stale"));
        }
        Ok(())
    }

    pub fn verify_server(&self, info: &ServerInfo) -> Result<(), connectrpc::ConnectError> {
        if self.instance_id != info.daemon_id
            || self.pid != info.pid
            || self.process_started_at != info.process_started_at
        {
            return Err(unavailable(
                "client discovery does not match server identity",
            ));
        }
        Ok(())
    }
}

fn unavailable(message: impl Into<String>) -> connectrpc::ConnectError {
    connectrpc::ConnectError::new(connectrpc::ErrorCode::Unavailable, message)
}

#[cfg(test)]
#[path = "discovery_test.rs"]
mod discovery_tests;
