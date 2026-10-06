use crate::infrastructure::telemetry::crash::init_crash_reporting;
use opentelemetry_sdk::logs::{InMemoryLogExporter, SdkLoggerProvider};
use std::sync::Mutex;

pub static TEST_LOCK: Mutex<()> = Mutex::new(());

pub fn install_test_exporter(
    enabled: bool,
    configured: bool,
) -> (SdkLoggerProvider, InMemoryLogExporter) {
    let exporter = InMemoryLogExporter::default();
    let provider = SdkLoggerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    init_crash_reporting(Some(provider.clone()), enabled, configured);
    (provider, exporter)
}
