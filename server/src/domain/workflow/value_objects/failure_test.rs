pub(crate) mod tests {
    use super::super::*;

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
