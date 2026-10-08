use crate::domain::workflow::{ExecutionParentRef, FanoutSlot};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExecutionParentWire {
    parent_id: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    delegate: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fanout_slot: Option<FanoutSlot>,
}

impl Serialize for ExecutionParentRef {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        ExecutionParentWire {
            parent_id: self.parent_id.clone(),
            delegate: self.is_delegate_child(),
            fanout_slot: self.fanout_slot(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ExecutionParentRef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = ExecutionParentWire::deserialize(deserializer)?;
        match (wire.delegate, wire.fanout_slot) {
            (false, None) => Ok(Self::sequence_child(wire.parent_id)),
            (false, Some(slot)) => Ok(Self::fanout_child(
                wire.parent_id,
                slot.item_index,
                slot.child_index,
            )),
            (true, None) => Ok(Self::delegate_child(wire.parent_id)),
            (true, Some(_)) => Err(serde::de::Error::custom(
                "delegate parent reference cannot have a fanout slot",
            )),
        }
    }
}

#[cfg(test)]
#[path = "execution_parent_wire_test.rs"]
mod execution_parent_wire_tests;
