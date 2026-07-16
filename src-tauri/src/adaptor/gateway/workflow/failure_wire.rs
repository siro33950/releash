use serde::de::{Error as DeError, Unexpected};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::domain::workflow::{FailureDisposition, NodeExecutionFailureKind, TimeoutKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SubmissionViolation {
    MissingSubmitOutput,
    InvalidSubmitOutput,
}

pub(crate) fn submission_violation_reason(violation: SubmissionViolation) -> &'static str {
    match violation {
        SubmissionViolation::MissingSubmitOutput => "missing_submit_output",
        SubmissionViolation::InvalidSubmitOutput => "invalid_submit_output",
    }
}

impl Serialize for NodeExecutionFailureKind {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for NodeExecutionFailureKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        node_execution_failure_kind_from_str(&value).ok_or_else(|| {
            D::Error::invalid_value(Unexpected::Str(&value), &"a node execution failure kind")
        })
    }
}

impl Serialize for FailureDisposition {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for FailureDisposition {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        failure_disposition_from_str(&value).ok_or_else(|| {
            D::Error::invalid_value(Unexpected::Str(&value), &"a failure disposition")
        })
    }
}

impl Serialize for TimeoutKind {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for TimeoutKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        timeout_kind_from_str(&value)
            .ok_or_else(|| D::Error::invalid_value(Unexpected::Str(&value), &"a timeout kind"))
    }
}

fn node_execution_failure_kind_from_str(value: &str) -> Option<NodeExecutionFailureKind> {
    match value {
        "startup_timeout" => Some(NodeExecutionFailureKind::StartupTimeout),
        "stale_runtime_timeout" => Some(NodeExecutionFailureKind::StaleRuntimeTimeout),
        "model_refusal" => Some(NodeExecutionFailureKind::ModelRefusal),
        "structured_output_mismatch" => Some(NodeExecutionFailureKind::StructuredOutputMismatch),
        "validation_failure" => Some(NodeExecutionFailureKind::ValidationFailure),
        "user_abort" => Some(NodeExecutionFailureKind::UserAbort),
        "infrastructure_crash" => Some(NodeExecutionFailureKind::InfrastructureCrash),
        _ => None,
    }
}

fn failure_disposition_from_str(value: &str) -> Option<FailureDisposition> {
    match value {
        "retryable" => Some(FailureDisposition::Retryable),
        "partial" => Some(FailureDisposition::Partial),
        "terminal" => Some(FailureDisposition::Terminal),
        "user-action-required" => Some(FailureDisposition::UserActionRequired),
        _ => None,
    }
}

fn timeout_kind_from_str(value: &str) -> Option<TimeoutKind> {
    match value {
        "startup" => Some(TimeoutKind::Startup),
        "stale" => Some(TimeoutKind::Stale),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_kind_wire_serde_uses_stable_strings() {
        let json = serde_json::to_string(&NodeExecutionFailureKind::ModelRefusal).unwrap();
        assert_eq!(json, "\"model_refusal\"");
        let back: NodeExecutionFailureKind = serde_json::from_str(&json).unwrap();
        assert_eq!(back, NodeExecutionFailureKind::ModelRefusal);
    }

    #[test]
    fn failure_disposition_wire_serde_uses_stable_strings() {
        let json = serde_json::to_string(&FailureDisposition::UserActionRequired).unwrap();
        assert_eq!(json, "\"user-action-required\"");
        let back: FailureDisposition = serde_json::from_str(&json).unwrap();
        assert_eq!(back, FailureDisposition::UserActionRequired);
    }
}
