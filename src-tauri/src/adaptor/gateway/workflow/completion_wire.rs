use crate::domain::workflow::value_objects::{InputSourceRef, InputsMap, InputsMapSeed};
use serde::de::{DeserializeSeed, MapAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::domain::workflow::{CompletionRequirement, NodeCompletion, SessionDelegate};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum CompletionShapeError {
    ExpectedMap,
    Empty,
    UnknownField,
    InvalidRequirement,
    InvalidDelegate(String),
}

impl std::fmt::Display for CompletionShapeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::ExpectedMap => "completion must be a map",
            Self::Empty => "completion must contain at least one requirement",
            Self::UnknownField => "completion map contains an unsupported key",
            Self::InvalidRequirement => "completion require must be approval",
            Self::InvalidDelegate(message) => message,
        })
    }
}

impl std::error::Error for CompletionShapeError {}

pub(super) fn parse_completion(value: &Value) -> Result<NodeCompletion, CompletionShapeError> {
    let map = value.as_object().ok_or(CompletionShapeError::ExpectedMap)?;
    let require = parse_require(map)?;
    let delegate = map.get("delegate").map(parse_delegate).transpose()?;
    Ok(NodeCompletion { require, delegate })
}

fn parse_require(
    map: &serde_json::Map<String, Value>,
) -> Result<Option<CompletionRequirement>, CompletionShapeError> {
    if map.is_empty() {
        return Err(CompletionShapeError::Empty);
    }
    if map
        .keys()
        .any(|key| !matches!(key.as_str(), "require" | "delegate"))
    {
        return Err(CompletionShapeError::UnknownField);
    }
    let require = match map.get("require") {
        Some(value) if value.as_str() == Some("approval") => Some(CompletionRequirement::Approval),
        None => None,
        _ => return Err(CompletionShapeError::InvalidRequirement),
    };
    Ok(require)
}

fn parse_delegate(value: &Value) -> Result<SessionDelegate, CompletionShapeError> {
    let invalid = |message: &str| CompletionShapeError::InvalidDelegate(message.to_string());
    let map = value
        .as_object()
        .ok_or_else(|| invalid("completion delegate must be a map"))?;
    let inputs = map
        .get("inputs")
        .map(|value| InputsMapSeed.deserialize(value))
        .transpose()
        .map_err(|error| CompletionShapeError::InvalidDelegate(error.to_string()))?
        .unwrap_or_default();
    parse_delegate_fields(map, inputs)
}

fn parse_delegate_fields(
    map: &serde_json::Map<String, Value>,
    inputs: Vec<(String, InputSourceRef)>,
) -> Result<SessionDelegate, CompletionShapeError> {
    let invalid = |message: &str| CompletionShapeError::InvalidDelegate(message.to_string());
    if map
        .keys()
        .any(|key| !matches!(key.as_str(), "child" | "inputs" | "when" | "max_iterations"))
    {
        return Err(invalid(
            "completion delegate only accepts child, inputs, when, and max_iterations",
        ));
    }
    let child = map
        .get("child")
        .and_then(Value::as_str)
        .filter(|child| !child.is_empty())
        .ok_or_else(|| invalid("completion delegate requires a child node name"))?
        .to_string();
    let when = super::predicate_wire::parse_predicate(
        map.get("when")
            .ok_or_else(|| invalid("completion delegate requires when"))?,
    )
    .map_err(|error| CompletionShapeError::InvalidDelegate(error.to_string()))?;
    let max_iterations = map
        .get("max_iterations")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| {
            invalid("completion delegate max_iterations must be an unsigned 32-bit integer")
        })?;
    Ok(SessionDelegate {
        child,
        inputs,
        when,
        max_iterations,
    })
}

impl<'de> Deserialize<'de> for NodeCompletion {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct CompletionVisitor;
        impl<'de> Visitor<'de> for CompletionVisitor {
            type Value = NodeCompletion;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a completion map")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Self::Value, A::Error> {
                let mut fields = serde_json::Map::new();
                let mut delegate = None;
                while let Some(key) = access.next_key::<String>()? {
                    let value = if key == "delegate" {
                        delegate = Some(access.next_value::<SessionDelegate>()?);
                        Value::Null
                    } else {
                        access.next_value()?
                    };
                    fields.insert(key, value);
                }
                let require = parse_require(&fields).map_err(serde::de::Error::custom)?;
                Ok(NodeCompletion { require, delegate })
            }
        }
        deserializer.deserialize_map(CompletionVisitor)
    }
}

impl<'de> Deserialize<'de> for SessionDelegate {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct DelegateVisitor;
        impl<'de> Visitor<'de> for DelegateVisitor {
            type Value = SessionDelegate;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a delegate map")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Self::Value, A::Error> {
                let mut fields = serde_json::Map::new();
                let mut inputs: Vec<(String, InputSourceRef)> = Vec::new();
                while let Some(key) = access.next_key::<String>()? {
                    if key == "inputs" {
                        inputs = access.next_value_seed(InputsMapSeed)?;
                    } else {
                        fields.insert(key, access.next_value()?);
                    }
                }
                parse_delegate_fields(&fields, inputs).map_err(serde::de::Error::custom)
            }
        }
        deserializer.deserialize_map(DelegateVisitor)
    }
}

impl Serialize for NodeCompletion {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if self.is_empty() {
            return Err(serde::ser::Error::custom(CompletionShapeError::Empty));
        }
        let mut map = serializer.serialize_map(None)?;
        if self.requires_approval() {
            map.serialize_entry("require", "approval")?;
        }
        if let Some(delegate) = &self.delegate {
            map.serialize_entry("delegate", delegate)?;
        }
        map.end()
    }
}

impl Serialize for SessionDelegate {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("child", &self.child)?;
        if !self.inputs.is_empty() {
            map.serialize_entry("inputs", &InputsMap(&self.inputs))?;
        }
        map.serialize_entry("when", &self.when)?;
        map.serialize_entry("max_iterations", &self.max_iterations)?;
        map.end()
    }
}

#[cfg(test)]
#[path = "completion_wire_test.rs"]
mod completion_wire_tests;
