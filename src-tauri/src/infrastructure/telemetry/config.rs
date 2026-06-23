use std::collections::HashMap;

pub(crate) const OTLP_ENDPOINT: &str = env!("OTLP_ENDPOINT");
pub(crate) const NEW_RELIC_LICENSE_KEY: &str = env!("NEW_RELIC_LICENSE_KEY");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BuildType {
    Dev,
    Release,
}

impl BuildType {
    pub(crate) fn current() -> Self {
        if cfg!(debug_assertions) {
            Self::Dev
        } else {
            Self::Release
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Dev => "dev",
            Self::Release => "release",
        }
    }
}

pub(crate) fn endpoint() -> &'static str {
    OTLP_ENDPOINT
}

pub(crate) fn license_key() -> &'static str {
    NEW_RELIC_LICENSE_KEY
}

pub(crate) fn configured(endpoint: &str, license_key: &str) -> bool {
    !endpoint.trim().is_empty() && !license_key.trim().is_empty()
}

pub(crate) fn telemetry_active(
    build_type: BuildType,
    endpoint: &str,
    license_key: &str,
    performance_enabled: bool,
) -> bool {
    if !configured(endpoint, license_key) {
        return false;
    }
    match build_type {
        BuildType::Dev => true,
        BuildType::Release => performance_enabled,
    }
}

pub(crate) fn otlp_headers(license_key: &str) -> HashMap<String, String> {
    HashMap::from([("api-key".to_string(), license_key.to_string())])
}

pub(crate) fn signal_endpoint(endpoint: &str, signal: &str) -> String {
    let endpoint = endpoint.trim().trim_end_matches('/');
    for existing_signal in ["traces", "metrics", "logs"] {
        let suffix = format!("/v1/{existing_signal}");
        if let Some(prefix) = endpoint.strip_suffix(&suffix) {
            return format!("{prefix}/v1/{signal}");
        }
    }
    format!("{endpoint}/v1/{signal}")
}

#[cfg(test)]
mod tests {
    use super::*;

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
