#[cfg(test)]
use crate::infrastructure::client_protocol::wire::ServerInfo;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

pub use crate::infrastructure::client_protocol::wire::LocalApiDiscovery;

impl Serialize for LocalApiDiscovery {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let descriptor = crate::infrastructure::client_protocol::descriptor::pool()
            .get_message_by_name("releash.client.v1.LocalApiDiscovery")
            .expect("discovery descriptor");
        let message = prost_reflect::DynamicMessage::decode(
            descriptor,
            prost::Message::encode_to_vec(self).as_slice(),
        )
        .map_err(serde::ser::Error::custom)?;
        message.serialize_with_options(
            serializer,
            &prost_reflect::SerializeOptions::new().use_proto_field_name(true),
        )
    }
}

impl<'de> Deserialize<'de> for LocalApiDiscovery {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let descriptor = crate::infrastructure::client_protocol::descriptor::pool()
            .get_message_by_name("releash.client.v1.LocalApiDiscovery")
            .expect("discovery descriptor");
        let json = serde_json::Value::deserialize(deserializer)?;
        for (field, json_name) in [
            ("port", "port"),
            ("token", "token"),
            ("pid", "pid"),
            ("process_started_at", "processStartedAt"),
        ] {
            if json
                .get(field)
                .or_else(|| json.get(json_name))
                .is_none_or(serde_json::Value::is_null)
            {
                return Err(serde::de::Error::missing_field(field));
            }
        }
        let has_daemon = json.get("daemon_id").is_some() || json.get("daemonId").is_some();
        let identity = if has_daemon {
            json.get("daemon_id").or_else(|| json.get("daemonId"))
        } else {
            json.get("instance_id").or_else(|| json.get("instanceId"))
        };
        if identity.is_none_or(serde_json::Value::is_null) {
            return Err(serde::de::Error::missing_field("daemon_id"));
        }
        let daemon = descriptor.get_field_by_name("daemon_id").unwrap();
        let mut message = prost_reflect::DynamicMessage::deserialize(descriptor, json)
            .map_err(serde::de::Error::custom)?;
        if !has_daemon {
            let legacy = message
                .get_field_by_name("instance_id")
                .unwrap()
                .into_owned();
            message.set_field(&daemon, legacy);
        }
        message.clear_field_by_name("instance_id");
        message.transcode_to().map_err(serde::de::Error::custom)
    }
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

pub fn discovery_file(data_dir: &Path) -> PathBuf {
    data_dir.join("client-api.json")
}

pub fn read_optional(data_dir: &Path) -> Result<Option<LocalApiDiscovery>, DiscoveryReadError> {
    let bytes = match std::fs::read(discovery_file(data_dir)) {
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
        if (self.port == 0 || self.port > u16::MAX as u32)
            || self.token.trim().is_empty()
            || self.daemon_id.trim().is_empty()
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

    #[cfg(test)]
    pub fn verify_server(&self, info: &ServerInfo) -> Result<(), connectrpc::ConnectError> {
        if self.daemon_id != info.daemon_id
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
#[path = "discovery_record_test.rs"]
mod discovery_record_tests;
