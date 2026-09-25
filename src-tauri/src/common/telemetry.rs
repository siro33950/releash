use opentelemetry::trace::{Span, Tracer};
use opentelemetry::{global, KeyValue};
use std::time::{Duration, Instant};

pub(crate) fn measure_result<T, E>(
    enabled: bool,
    span_name: &'static str,
    operation_name: &'static str,
    operation: impl FnOnce() -> Result<T, E>,
    record: impl FnOnce(bool, Duration),
) -> Result<T, E> {
    if !enabled {
        return operation();
    }
    let tracer = global::tracer("releash.performance");
    let mut span = tracer
        .span_builder(span_name)
        .with_attributes(vec![KeyValue::new("releash.operation", operation_name)])
        .start(&tracer);
    let started = Instant::now();
    let result = operation();
    record(result.is_ok(), started.elapsed());
    span.set_attribute(KeyValue::new(
        "releash.status",
        if result.is_ok() { "success" } else { "failure" },
    ));
    span.end();
    result
}

pub(crate) fn observe_result<T, E>(
    operation: impl FnOnce() -> Result<T, E>,
    record: impl FnOnce(&Result<T, E>, Duration),
) -> Result<T, E> {
    let started = Instant::now();
    let result = operation();
    record(&result, started.elapsed());
    result
}

pub(crate) async fn observe_result_async<T, E>(
    operation: impl std::future::Future<Output = Result<T, E>>,
    record: impl FnOnce(&Result<T, E>, Duration),
) -> Result<T, E> {
    let started = Instant::now();
    let result = operation.await;
    record(&result, started.elapsed());
    result
}

#[cfg(test)]
#[path = "telemetry_test.rs"]
mod telemetry_tests;
