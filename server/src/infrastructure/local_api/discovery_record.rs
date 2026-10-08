use serde::Serialize;
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
