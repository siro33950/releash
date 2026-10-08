pub(crate) mod config;
pub(crate) mod crash;

use opentelemetry::global;
use opentelemetry::KeyValue;
use opentelemetry_otlp::{
    LogExporter, MetricExporter, SpanExporter, WithExportConfig, WithHttpConfig,
};
use opentelemetry_sdk::logs::SdkLoggerProvider;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::trace::SdkTracerProvider;
use opentelemetry_sdk::Resource;

pub struct TelemetryGuard {
    tracer_provider: SdkTracerProvider,
    meter_provider: SdkMeterProvider,
    logger_provider: SdkLoggerProvider,
}

impl Drop for TelemetryGuard {
    fn drop(&mut self) {
        if let Err(error) = self.tracer_provider.shutdown() {
            log::error!("OTLP tracer provider shutdown failed: {error}");
        }
        if let Err(error) = self.meter_provider.shutdown() {
            log::error!("OTLP meter provider shutdown failed: {error}");
        }
        if let Err(error) = self.logger_provider.shutdown() {
            log::error!("OTLP logger provider shutdown failed: {error}");
        }
    }
}

pub fn init_telemetry(
    crash_reporting: bool,
    performance_telemetry: bool,
) -> Option<TelemetryGuard> {
    let endpoint = config::endpoint();
    let license_key = config::license_key();
    let configured = config::configured(endpoint, license_key);
    let active = config::telemetry_active(
        config::BuildType::current(),
        endpoint,
        license_key,
        performance_telemetry,
    );

    crate::infrastructure::telemetry::metrics::set_performance_configured(configured);
    crate::infrastructure::telemetry::metrics::set_performance_enabled(active);

    if !configured {
        crash::init_crash_reporting(None, crash_reporting, false);
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
            crash::init_crash_reporting(None, crash_reporting, false);
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
            crash::init_crash_reporting(None, crash_reporting, false);
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
            crash::init_crash_reporting(None, crash_reporting, false);
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
    crate::infrastructure::telemetry::metrics::install_metrics();

    let logger_provider = SdkLoggerProvider::builder()
        .with_batch_exporter(log_exporter)
        .with_resource(resource)
        .build();
    crash::init_crash_reporting(Some(logger_provider.clone()), crash_reporting, configured);

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

pub(crate) mod metrics;

#[cfg(test)]
#[path = "mod_test.rs"]
pub(crate) mod mod_tests;

#[cfg(any(test, feature = "test-support"))]
pub(crate) mod test_helpers;
