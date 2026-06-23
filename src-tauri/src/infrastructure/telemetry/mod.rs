pub(crate) mod config;
pub(crate) mod crash;

use crate::adaptor::gateway::app_config::ReleashConfig;
use opentelemetry::global;
use opentelemetry::KeyValue;
use opentelemetry_otlp::{
    LogExporter, MetricExporter, SpanExporter, WithExportConfig, WithHttpConfig,
};
use opentelemetry_sdk::logs::SdkLoggerProvider;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::trace::SdkTracerProvider;
use opentelemetry_sdk::Resource;

pub(crate) struct TelemetryGuard {
    tracer_provider: SdkTracerProvider,
    meter_provider: SdkMeterProvider,
    logger_provider: SdkLoggerProvider,
}

impl Drop for TelemetryGuard {
    fn drop(&mut self) {
        let _ = self.tracer_provider.shutdown();
        let _ = self.meter_provider.shutdown();
        let _ = self.logger_provider.shutdown();
    }
}

pub(crate) fn init_telemetry(config: &ReleashConfig) -> Option<TelemetryGuard> {
    let endpoint = config::endpoint();
    let license_key = config::license_key();
    let configured = config::configured(endpoint, license_key);
    let active = config::telemetry_active(
        config::BuildType::current(),
        endpoint,
        license_key,
        config.telemetry.performance_telemetry,
    );

    crate::other::telemetry::set_performance_configured(configured);
    crate::other::telemetry::set_performance_enabled(active);

    if !configured {
        crash::init_crash_reporting(None, config.telemetry.crash_reporting, false);
        return None;
    }

    let resource = build_resource(config::BuildType::current());
    let headers = config::otlp_headers(license_key);

    let span_exporter = match SpanExporter::builder()
        .with_http()
        .with_endpoint(config::signal_endpoint(endpoint, "traces"))
        .with_headers(headers.clone())
        .build()
    {
        Ok(exporter) => exporter,
        Err(error) => {
            log::warn!("Failed to build OTLP span exporter: {error}");
            crash::init_crash_reporting(None, config.telemetry.crash_reporting, false);
            return None;
        }
    };
    let metric_exporter = match MetricExporter::builder()
        .with_http()
        .with_endpoint(config::signal_endpoint(endpoint, "metrics"))
        .with_headers(headers.clone())
        .build()
    {
        Ok(exporter) => exporter,
        Err(error) => {
            log::warn!("Failed to build OTLP metric exporter: {error}");
            crash::init_crash_reporting(None, config.telemetry.crash_reporting, false);
            return None;
        }
    };
    let log_exporter = match LogExporter::builder()
        .with_http()
        .with_endpoint(config::signal_endpoint(endpoint, "logs"))
        .with_headers(headers)
        .build()
    {
        Ok(exporter) => exporter,
        Err(error) => {
            log::warn!("Failed to build OTLP log exporter: {error}");
            crash::init_crash_reporting(None, config.telemetry.crash_reporting, false);
            return None;
        }
    };

    let tracer_provider = SdkTracerProvider::builder()
        .with_batch_exporter(span_exporter)
        .with_resource(resource.clone())
        .build();
    global::set_tracer_provider(tracer_provider.clone());

    let meter_provider = SdkMeterProvider::builder()
        .with_periodic_exporter(metric_exporter)
        .with_resource(resource.clone())
        .build();
    global::set_meter_provider(meter_provider.clone());
    crate::other::telemetry::install_metrics();

    let logger_provider = SdkLoggerProvider::builder()
        .with_batch_exporter(log_exporter)
        .with_resource(resource)
        .build();
    crash::init_crash_reporting(
        Some(logger_provider.clone()),
        config.telemetry.crash_reporting,
        configured,
    );

    Some(TelemetryGuard {
        tracer_provider,
        meter_provider,
        logger_provider,
    })
}

pub(crate) fn build_resource(build_type: config::BuildType) -> Resource {
    Resource::builder_empty()
        .with_attributes([
            KeyValue::new("service.version", env!("CARGO_PKG_VERSION")),
            KeyValue::new("os.type", std::env::consts::OS),
            KeyValue::new("releash.build_type", build_type.as_str()),
            KeyValue::new("service.name", "releash"),
        ])
        .build()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use opentelemetry::Value;

    use super::*;

    #[test]
    fn build_resource_has_exact_attribute_set() {
        let resource = build_resource(config::BuildType::Release);
        let attrs = resource
            .iter()
            .map(|(key, value)| (key.as_str().to_string(), value.clone()))
            .collect::<BTreeMap<_, _>>();

        assert_eq!(
            attrs.keys().map(String::as_str).collect::<Vec<_>>(),
            [
                "os.type",
                "releash.build_type",
                "service.name",
                "service.version"
            ]
        );
        assert_eq!(
            attrs.get("releash.build_type"),
            Some(&Value::String("release".into()))
        );
        assert_eq!(
            attrs.get("service.name"),
            Some(&Value::String("releash".into()))
        );
    }

    #[test]
    fn build_resource_uses_build_type_value() {
        let resource = build_resource(config::BuildType::Dev);
        let attrs = resource
            .iter()
            .map(|(key, value)| (key.as_str().to_string(), value.clone()))
            .collect::<BTreeMap<_, _>>();

        assert_eq!(
            attrs.get("releash.build_type"),
            Some(&Value::String("dev".into()))
        );
        assert_eq!(attrs.len(), 4);
    }
}
