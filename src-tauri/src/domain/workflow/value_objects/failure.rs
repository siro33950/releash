use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeExecutionFailureKind {
    StartupTimeout,
    StaleRuntimeTimeout,
    ModelRefusal,
    StructuredOutputMismatch,
    ValidationFailure,
    UserAbort,
    InfrastructureCrash,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureDisposition {
    Retryable,
    Partial,
    Terminal,
    UserActionRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeoutKind {
    Startup,
    Stale,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FailureClassification {
    pub kind: NodeExecutionFailureKind,
    pub disposition: FailureDisposition,
    pub timeout_kind: Option<TimeoutKind>,
}

impl FailureClassification {
    pub fn new(kind: NodeExecutionFailureKind) -> Self {
        Self {
            kind,
            disposition: kind.default_disposition(),
            timeout_kind: kind.timeout_kind(),
        }
    }
}

impl NodeExecutionFailureKind {
    pub fn default_disposition(self) -> FailureDisposition {
        match self {
            Self::StartupTimeout | Self::StaleRuntimeTimeout | Self::StructuredOutputMismatch => {
                FailureDisposition::Retryable
            }
            Self::ModelRefusal => FailureDisposition::Partial,
            Self::ValidationFailure | Self::InfrastructureCrash => FailureDisposition::Terminal,
            Self::UserAbort => FailureDisposition::UserActionRequired,
        }
    }

    pub fn timeout_kind(self) -> Option<TimeoutKind> {
        match self {
            Self::StartupTimeout => Some(TimeoutKind::Startup),
            Self::StaleRuntimeTimeout => Some(TimeoutKind::Stale),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::StartupTimeout => "startup_timeout",
            Self::StaleRuntimeTimeout => "stale_runtime_timeout",
            Self::ModelRefusal => "model_refusal",
            Self::StructuredOutputMismatch => "structured_output_mismatch",
            Self::ValidationFailure => "validation_failure",
            Self::UserAbort => "user_abort",
            Self::InfrastructureCrash => "infrastructure_crash",
        }
    }
}

impl FailureDisposition {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Retryable => "retryable",
            Self::Partial => "partial",
            Self::Terminal => "terminal",
            Self::UserActionRequired => "user-action-required",
        }
    }
}

impl TimeoutKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Startup => "startup",
            Self::Stale => "stale",
        }
    }
}

impl fmt::Display for NodeExecutionFailureKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Display for FailureDisposition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Display for TimeoutKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_kinds_have_expected_default_dispositions() {
        let cases = [
            (
                NodeExecutionFailureKind::StartupTimeout,
                FailureDisposition::Retryable,
            ),
            (
                NodeExecutionFailureKind::StaleRuntimeTimeout,
                FailureDisposition::Retryable,
            ),
            (
                NodeExecutionFailureKind::ModelRefusal,
                FailureDisposition::Partial,
            ),
            (
                NodeExecutionFailureKind::StructuredOutputMismatch,
                FailureDisposition::Retryable,
            ),
            (
                NodeExecutionFailureKind::ValidationFailure,
                FailureDisposition::Terminal,
            ),
            (
                NodeExecutionFailureKind::UserAbort,
                FailureDisposition::UserActionRequired,
            ),
            (
                NodeExecutionFailureKind::InfrastructureCrash,
                FailureDisposition::Terminal,
            ),
        ];

        for (kind, disposition) in cases {
            assert_eq!(kind.default_disposition(), disposition);
        }
    }

    #[test]
    fn timeout_kind_is_only_present_for_timeout_failures() {
        assert_eq!(
            NodeExecutionFailureKind::StartupTimeout.timeout_kind(),
            Some(TimeoutKind::Startup)
        );
        assert_eq!(
            NodeExecutionFailureKind::StaleRuntimeTimeout.timeout_kind(),
            Some(TimeoutKind::Stale)
        );
        assert_eq!(NodeExecutionFailureKind::ModelRefusal.timeout_kind(), None);
    }

    #[test]
    fn strings_are_stable_for_events_and_telemetry() {
        assert_eq!(
            NodeExecutionFailureKind::StructuredOutputMismatch.as_str(),
            "structured_output_mismatch"
        );
        assert_eq!(
            FailureDisposition::UserActionRequired.as_str(),
            "user-action-required"
        );
        assert_eq!(TimeoutKind::Startup.as_str(), "startup");
    }
}
