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
#[path = "config_test.rs"]
mod config_tests;
