pub(crate) mod tests {
    use std::collections::BTreeMap;

    use opentelemetry::Value;

    use super::super::*;

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

mod telemetry_tests {
    use super::super::*;
    use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};
    use opentelemetry_sdk::metrics::{
        data::ResourceMetrics, exporter::PushMetricExporter, Temporality,
    };
    use std::sync::{mpsc, Mutex};
    use std::time::Duration;

    struct BlockingMetricExporter(Mutex<mpsc::Receiver<()>>);

    impl PushMetricExporter for BlockingMetricExporter {
        async fn export(&self, _metrics: &ResourceMetrics) -> OTelSdkResult {
            Ok(())
        }

        fn force_flush(&self) -> OTelSdkResult {
            unreachable!("shutdown does not force flush")
        }

        fn shutdown_with_timeout(&self, _timeout: Duration) -> OTelSdkResult {
            self.0.lock().unwrap().recv().unwrap();
            Ok(())
        }

        fn temporality(&self) -> Temporality {
            Temporality::Cumulative
        }
    }

    #[test]
    fn test_telemetry終了_meterのタイムアウトをログに残してloggerを停止する() {
        // Given
        crate::test_support::install_capturing_logger();
        let (release, receiver) = mpsc::channel();
        let logger_provider = SdkLoggerProvider::builder().build();
        let telemetry = TelemetryGuard {
            tracer_provider: SdkTracerProvider::builder().build(),
            meter_provider: SdkMeterProvider::builder()
                .with_periodic_exporter(BlockingMetricExporter(Mutex::new(receiver)))
                .build(),
            logger_provider: logger_provider.clone(),
        };

        // When
        drop(telemetry);
        release.send(()).unwrap();

        // Then
        assert!(crate::test_support::captured_error_messages()
            .iter()
            .any(
                |message| message.starts_with("OTLP meter provider shutdown failed:")
                    && message.contains("Timeout(5s)")
            ));
        assert!(matches!(
            logger_provider.shutdown(),
            Err(OTelSdkError::AlreadyShutdown)
        ));
    }

    #[test]
    fn test_telemetry終了_各providerの失敗をログに残して最後まで続行する() {
        // Given
        crate::test_support::install_capturing_logger();
        let telemetry = TelemetryGuard {
            tracer_provider: SdkTracerProvider::builder().build(),
            meter_provider: SdkMeterProvider::builder().build(),
            logger_provider: SdkLoggerProvider::builder().build(),
        };
        telemetry.tracer_provider.shutdown().unwrap();
        telemetry.meter_provider.shutdown().unwrap();
        telemetry.logger_provider.shutdown().unwrap();

        // When
        drop(telemetry);

        // Then
        let messages = crate::test_support::captured_error_messages();
        for provider in ["tracer", "meter", "logger"] {
            assert!(messages.contains(&format!(
                "OTLP {provider} provider shutdown failed: Shutdown already invoked"
            )));
        }
    }
}
