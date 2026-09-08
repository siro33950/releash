use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::domain::workflow::{CompletionRequirement, NodeCompletion};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum CompletionShapeError {
    ExpectedMap,
    Empty,
    UnknownField,
    InvalidRequirement,
}

impl std::fmt::Display for CompletionShapeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::ExpectedMap => "completion must be a map",
            Self::Empty => "completion must contain at least one requirement",
            Self::UnknownField => "completion map only accepts the key 'require'",
            Self::InvalidRequirement => "completion require must be approval",
        })
    }
}

impl std::error::Error for CompletionShapeError {}

pub(super) fn parse_completion(value: &Value) -> Result<NodeCompletion, CompletionShapeError> {
    let map = value.as_object().ok_or(CompletionShapeError::ExpectedMap)?;
    if map.is_empty() {
        return Err(CompletionShapeError::Empty);
    }
    if map.keys().any(|key| key != "require") {
        return Err(CompletionShapeError::UnknownField);
    }
    match map.get("require").and_then(Value::as_str) {
        Some("approval") => Ok(NodeCompletion::require_approval()),
        _ => Err(CompletionShapeError::InvalidRequirement),
    }
}

impl<'de> Deserialize<'de> for NodeCompletion {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        parse_completion(&value).map_err(serde::de::Error::custom)
    }
}

impl Serialize for NodeCompletion {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(usize::from(self.require.is_some())))?;
        if let Some(CompletionRequirement::Approval) = self.require {
            map.serialize_entry("require", "approval")?;
        }
        map.end()
    }
}

#[cfg(test)]
#[path = "completion_wire_test.rs"]
mod completion_wire_tests;
