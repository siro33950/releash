pub(crate) mod tests {
    use super::super::*;

    #[test]
    fn telemetry_is_noop_without_endpoint_or_key() {
        assert!(!telemetry_active(BuildType::Release, "", "key", true));
        assert!(!telemetry_active(BuildType::Release, "endpoint", "", true));
    }

    #[test]
    fn dev_sends_when_configured_even_if_user_setting_is_false() {
        assert!(telemetry_active(BuildType::Dev, "endpoint", "key", false));
    }

    #[test]
    fn release_respects_user_opt_out() {
        assert!(telemetry_active(
            BuildType::Release,
            "endpoint",
            "key",
            true
        ));
        assert!(!telemetry_active(
            BuildType::Release,
            "endpoint",
            "key",
            false
        ));
    }

    #[test]
    fn signal_endpoint_appends_signal_path_to_base_endpoint() {
        assert_eq!(
            signal_endpoint("https://otlp.nr-data.net:4318", "metrics"),
            "https://otlp.nr-data.net:4318/v1/metrics"
        );
    }

    #[test]
    fn signal_endpoint_replaces_existing_signal_path() {
        assert_eq!(
            signal_endpoint("https://collector.example/v1/traces", "logs"),
            "https://collector.example/v1/logs"
        );
    }
}
