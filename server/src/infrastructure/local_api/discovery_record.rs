use serde::Serialize;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

pub use crate::infrastructure::client_protocol::wire::LocalApiDiscovery;

fn descriptor() -> prost_reflect::MessageDescriptor {
    crate::infrastructure::client_protocol::descriptor::pool()
        .get_message_by_name("releash.client.v1.LocalApiDiscovery")
        .expect("discovery descriptor")
}

pub(crate) fn decode(bytes: &[u8]) -> Option<LocalApiDiscovery> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let message =
        prost_reflect::DynamicMessage::deserialize(descriptor(), &mut deserializer).ok()?;
    deserializer.end().ok()?;
    message.transcode_to().ok()
}

impl Serialize for LocalApiDiscovery {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let message = prost_reflect::DynamicMessage::decode(
            descriptor(),
            prost::Message::encode_to_vec(self).as_slice(),
        )
        .map_err(serde::ser::Error::custom)?;
        message.serialize_with_options(
            serializer,
            &prost_reflect::SerializeOptions::new().use_proto_field_name(true),
        )
    }
}

pub fn process_start_time(pid: u32) -> Option<u64> {
    let pid = Pid::from_u32(pid);
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing(),
    );
    let start_time = system.process(pid).map(|process| process.start_time());
    if start_time.is_some_and(|start_time| start_time != 0) {
        return start_time;
    }
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
    system
        .process(pid)
        .map(|process| process.start_time())
        .filter(|start_time| *start_time != 0)
}
