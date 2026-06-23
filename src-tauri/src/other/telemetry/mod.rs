pub(crate) mod attributes;
mod resource;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use attributes::{
    usage_event_allowed, HotPathMetric, OpStatus, PayloadChannel, StartupMetric, KEY_CHANNEL,
    KEY_OPERATION, KEY_STATUS, KEY_USAGE_EVENT,
};
use opentelemetry::global;
use opentelemetry::metrics::{Counter, Histogram, ObservableGauge};
use opentelemetry::trace::{Span, Tracer};
use opentelemetry::KeyValue;
use resource::ProcessResourceObserver;

pub(crate) use attributes::HotPathMetric as HotPath;
pub(crate) use attributes::{PayloadChannel as Payload, StartupMetric as Startup};

static PERFORMANCE_CONFIGURED: AtomicBool = AtomicBool::new(false);
static PERFORMANCE_ENABLED: AtomicBool = AtomicBool::new(true);
static MOUNTED_XTERM_COUNT: AtomicU64 = AtomicU64::new(0);
static ACTIVE_PTY_COUNT: AtomicU64 = AtomicU64::new(0);
static METRICS: OnceLock<Metrics> = OnceLock::new();
static STARTUP_ORIGIN: Mutex<Option<Instant>> = Mutex::new(None);
static FIRST_REPO_SNAPSHOT_RECORDED: AtomicBool = AtomicBool::new(false);

#[cfg(test)]
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TestMetricRecord {
    pub(crate) name: &'static str,
    pub(crate) value: f64,
    pub(crate) attributes: Vec<(String, String)>,
}

#[cfg(test)]
static TEST_METRIC_RECORDS: Mutex<Vec<TestMetricRecord>> = Mutex::new(Vec::new());
#[cfg(test)]
static TEST_TELEMETRY_LOCK: Mutex<()> = Mutex::new(());

struct Metrics {
    hot_path_duration: Histogram<f64>,
    startup_duration: Histogram<f64>,
    stream_payload_bytes: Histogram<f64>,
    stream_emit_interval_ms: Histogram<f64>,
    session_save_bytes: Histogram<f64>,
    operation_status: Counter<u64>,
    dropped_stream_frames: Counter<u64>,
    ws_reconnects: Counter<u64>,
    usage_events: Counter<u64>,
    _rss_gauge: ObservableGauge<u64>,
    _cpu_gauge: ObservableGauge<f64>,
    _xterm_gauge: ObservableGauge<u64>,
    _pty_gauge: ObservableGauge<u64>,
}

pub(crate) fn set_performance_configured(configured: bool) {
    PERFORMANCE_CONFIGURED.store(configured, Ordering::Relaxed);
}

pub(crate) fn set_performance_enabled(enabled: bool) {
    PERFORMANCE_ENABLED.store(enabled, Ordering::Relaxed);
}

pub(crate) fn set_startup_origin(origin: Instant) {
    *STARTUP_ORIGIN.lock().unwrap_or_else(|e| e.into_inner()) = Some(origin);
    FIRST_REPO_SNAPSHOT_RECORDED.store(false, Ordering::Relaxed);
}

pub(crate) fn is_performance_active() -> bool {
    PERFORMANCE_CONFIGURED.load(Ordering::Relaxed) && PERFORMANCE_ENABLED.load(Ordering::Relaxed)
}

pub(crate) fn set_mounted_xterm_count(count: u64) {
    MOUNTED_XTERM_COUNT.store(count, Ordering::Relaxed);
}

pub(crate) fn set_active_pty_count(count: u64) {
    ACTIVE_PTY_COUNT.store(count, Ordering::Relaxed);
}

pub(crate) fn install_metrics() {
    let meter = global::meter("releash.performance");
    let process_observer = Arc::new(ProcessResourceObserver::default());
    let rss_observer = Arc::clone(&process_observer);
    let cpu_observer = Arc::clone(&process_observer);

    let metrics = Metrics {
        hot_path_duration: meter
            .f64_histogram("releash.hot_path.duration_ms")
            .with_unit("ms")
            .build(),
        startup_duration: meter
            .f64_histogram("releash.startup.duration_ms")
            .with_unit("ms")
            .build(),
        stream_payload_bytes: meter
            .f64_histogram("releash.agent_stream.payload_bytes")
            .with_unit("By")
            .build(),
        stream_emit_interval_ms: meter
            .f64_histogram("releash.agent_stream.emit_interval_ms")
            .with_unit("ms")
            .build(),
        session_save_bytes: meter
            .f64_histogram("releash.session.save_bytes")
            .with_unit("By")
            .build(),
        operation_status: meter.u64_counter("releash.operation.status").build(),
        dropped_stream_frames: meter
            .u64_counter("releash.agent_stream.dropped_frames")
            .build(),
        ws_reconnects: meter
            .u64_counter("releash.agent_stream.ws_reconnects")
            .build(),
        usage_events: meter.u64_counter("releash.usage.events").build(),
        _rss_gauge: meter
            .u64_observable_gauge("releash.process.rss_bytes")
            .with_unit("By")
            .with_callback(move |observer| {
                if !is_performance_active() {
                    return;
                }
                if let Some(sample) = rss_observer.sample() {
                    observer.observe(sample.rss_bytes, &[]);
                }
            })
            .build(),
        _cpu_gauge: meter
            .f64_observable_gauge("releash.process.cpu_percent")
            .with_unit("%")
            .with_callback(move |observer| {
                if !is_performance_active() {
                    return;
                }
                if let Some(sample) = cpu_observer.sample() {
                    observer.observe(sample.cpu_percent, &[]);
                }
            })
            .build(),
        _xterm_gauge: meter
            .u64_observable_gauge("releash.frontend.mounted_xterm_count")
            .with_callback(|observer| {
                if is_performance_active() {
                    observer.observe(MOUNTED_XTERM_COUNT.load(Ordering::Relaxed), &[]);
                }
            })
            .build(),
        _pty_gauge: meter
            .u64_observable_gauge("releash.pty.active_count")
            .with_callback(|observer| {
                if is_performance_active() {
                    observer.observe(ACTIVE_PTY_COUNT.load(Ordering::Relaxed), &[]);
                }
            })
            .build(),
    };

    let _ = METRICS.set(metrics);
}

fn startup_elapsed() -> Option<Duration> {
    STARTUP_ORIGIN
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .map(|origin| origin.elapsed())
}

pub(crate) fn record_startup_from_origin(metric: StartupMetric) {
    if let Some(elapsed) = startup_elapsed() {
        record_startup(metric, elapsed);
    }
}

pub(crate) fn record_first_repo_snapshot_ready() {
    if !is_performance_active() {
        return;
    }
    let Some(elapsed) = startup_elapsed() else {
        return;
    };
    if FIRST_REPO_SNAPSHOT_RECORDED.swap(true, Ordering::AcqRel) {
        return;
    }
    record_startup(StartupMetric::FirstRepoSnapshotReady, elapsed);
}

#[cfg(test)]
fn record_test_metric(name: &'static str, value: f64, attrs: &[KeyValue]) {
    TEST_METRIC_RECORDS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(TestMetricRecord {
            name,
            value,
            attributes: attrs
                .iter()
                .map(|kv| (kv.key.as_str().to_string(), kv.value.to_string()))
                .collect(),
        });
}

#[cfg(test)]
pub(crate) fn reset_test_metrics() {
    TEST_METRIC_RECORDS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clear();
    *STARTUP_ORIGIN.lock().unwrap_or_else(|e| e.into_inner()) = None;
    FIRST_REPO_SNAPSHOT_RECORDED.store(false, Ordering::Relaxed);
    set_performance_configured(false);
    set_performance_enabled(true);
}

#[cfg(test)]
pub(crate) fn test_metric_records() -> Vec<TestMetricRecord> {
    TEST_METRIC_RECORDS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

#[cfg(test)]
pub(crate) fn first_repo_snapshot_recorded_for_tests() -> bool {
    FIRST_REPO_SNAPSHOT_RECORDED.load(Ordering::Acquire)
}

#[cfg(test)]
pub(crate) fn lock_test_telemetry() -> std::sync::MutexGuard<'static, ()> {
    TEST_TELEMETRY_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

pub(crate) fn measure_result<T, E, F>(metric: HotPathMetric, f: F) -> Result<T, E>
where
    F: FnOnce() -> Result<T, E>,
{
    if !is_performance_active() {
        return f();
    }

    let tracer = global::tracer("releash.performance");
    let mut span = tracer
        .span_builder(metric.span_name())
        .with_attributes(vec![KeyValue::new(KEY_OPERATION, metric.operation())])
        .start(&tracer);

    let started = Instant::now();
    let result = f();
    let status = if result.is_ok() {
        OpStatus::Success
    } else {
        OpStatus::Failure
    };
    let elapsed = started.elapsed();

    record_hot_path_duration(metric, status, elapsed);
    span.set_attribute(KeyValue::new(KEY_STATUS, status.as_str()));
    span.end();

    result
}

pub(crate) fn record_hot_path_duration(metric: HotPathMetric, status: OpStatus, elapsed: Duration) {
    if !is_performance_active() {
        return;
    }
    let attrs = [
        KeyValue::new(KEY_OPERATION, metric.operation()),
        KeyValue::new(KEY_STATUS, status.as_str()),
    ];
    #[cfg(test)]
    record_test_metric(
        "releash.hot_path.duration_ms",
        elapsed.as_secs_f64() * 1000.0,
        &attrs,
    );
    #[cfg(test)]
    record_test_metric("releash.operation.status", 1.0, &attrs);
    let Some(metrics) = METRICS.get() else {
        return;
    };
    metrics
        .hot_path_duration
        .record(elapsed.as_secs_f64() * 1000.0, &attrs);
    metrics.operation_status.add(1, &attrs);
}

pub(crate) fn record_session_save_bytes<F>(metric: HotPathMetric, bytes: F)
where
    F: FnOnce() -> usize,
{
    if !is_performance_active() {
        return;
    }
    let bytes = bytes();
    let attrs = [KeyValue::new(KEY_OPERATION, metric.operation())];
    #[cfg(test)]
    record_test_metric("releash.session.save_bytes", bytes as f64, &attrs);
    let Some(metrics) = METRICS.get() else {
        return;
    };
    metrics.session_save_bytes.record(bytes as f64, &attrs);
}

pub(crate) fn record_payload_size<F>(channel: PayloadChannel, bytes: F)
where
    F: FnOnce() -> usize,
{
    if !is_performance_active() {
        return;
    }
    let bytes = bytes();
    let attrs = [KeyValue::new(KEY_CHANNEL, channel.as_str())];
    #[cfg(test)]
    record_test_metric("releash.agent_stream.payload_bytes", bytes as f64, &attrs);
    let Some(metrics) = METRICS.get() else {
        return;
    };
    metrics.stream_payload_bytes.record(bytes as f64, &attrs);
}

pub(crate) fn record_emit_interval(elapsed: Duration) {
    if !is_performance_active() {
        return;
    }
    #[cfg(test)]
    record_test_metric(
        "releash.agent_stream.emit_interval_ms",
        elapsed.as_secs_f64() * 1000.0,
        &[],
    );
    let Some(metrics) = METRICS.get() else {
        return;
    };
    metrics
        .stream_emit_interval_ms
        .record(elapsed.as_secs_f64() * 1000.0, &[]);
}

pub(crate) fn increment_dropped_stream_frames() {
    if !is_performance_active() {
        return;
    }
    #[cfg(test)]
    record_test_metric("releash.agent_stream.dropped_frames", 1.0, &[]);
    if let Some(metrics) = METRICS.get() {
        metrics.dropped_stream_frames.add(1, &[]);
    }
}

pub(crate) fn increment_ws_reconnects() {
    if !is_performance_active() {
        return;
    }
    #[cfg(test)]
    record_test_metric("releash.agent_stream.ws_reconnects", 1.0, &[]);
    if let Some(metrics) = METRICS.get() {
        metrics.ws_reconnects.add(1, &[]);
    }
}

pub(crate) fn record_startup(metric: StartupMetric, elapsed: Duration) {
    if !is_performance_active() {
        return;
    }
    let attrs = [KeyValue::new(KEY_OPERATION, metric.operation())];
    #[cfg(test)]
    record_test_metric(
        "releash.startup.duration_ms",
        elapsed.as_secs_f64() * 1000.0,
        &attrs,
    );
    let Some(metrics) = METRICS.get() else {
        return;
    };
    metrics
        .startup_duration
        .record(elapsed.as_secs_f64() * 1000.0, &attrs);
}

pub(crate) fn record_usage_event(name: &str) {
    if !is_performance_active() || !usage_event_allowed(name) {
        return;
    }
    let attrs = [KeyValue::new(KEY_USAGE_EVENT, name.to_string())];
    #[cfg(test)]
    record_test_metric("releash.usage.events", 1.0, &attrs);
    let Some(metrics) = METRICS.get() else {
        return;
    };
    metrics.usage_events.add(1, &attrs);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    fn set_active(active: bool) {
        set_performance_configured(active);
        set_performance_enabled(active);
    }

    fn has_attr(record: &TestMetricRecord, key: &str, value: &str) -> bool {
        record
            .attributes
            .iter()
            .any(|(attr_key, attr_value)| attr_key == key && attr_value == value)
    }

    fn records_named(name: &'static str) -> Vec<TestMetricRecord> {
        test_metric_records()
            .into_iter()
            .filter(|record| record.name == name)
            .collect()
    }

    #[test]
    fn performance_requires_configured_and_enabled() {
        let _guard = lock_test_telemetry();
        reset_test_metrics();
        set_performance_configured(false);
        set_performance_enabled(true);
        assert!(!is_performance_active());

        set_performance_configured(true);
        set_performance_enabled(false);
        assert!(!is_performance_active());

        set_performance_enabled(true);
        assert!(is_performance_active());
        reset_test_metrics();
    }

    #[test]
    fn xterm_count_is_saturating_on_public_setter() {
        let _guard = lock_test_telemetry();
        reset_test_metrics();
        set_mounted_xterm_count(3);
        assert_eq!(MOUNTED_XTERM_COUNT.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn record_size_closures_are_not_evaluated_when_inactive() {
        let _guard = lock_test_telemetry();
        reset_test_metrics();
        set_active(false);
        let payload_called = AtomicBool::new(false);
        let save_called = AtomicBool::new(false);

        record_payload_size(PayloadChannel::TauriEvent, || {
            payload_called.store(true, Ordering::Relaxed);
            123
        });
        record_session_save_bytes(HotPathMetric::SessionAppend, || {
            save_called.store(true, Ordering::Relaxed);
            456
        });

        assert!(!payload_called.load(Ordering::Relaxed));
        assert!(!save_called.load(Ordering::Relaxed));
        assert!(test_metric_records().is_empty());
    }

    #[test]
    fn record_apis_capture_values_and_attributes_when_active() {
        let _guard = lock_test_telemetry();
        reset_test_metrics();
        set_active(true);

        record_payload_size(PayloadChannel::TauriEvent, || 128);
        record_session_save_bytes(HotPathMetric::SessionAppend, || 256);
        record_emit_interval(Duration::from_millis(33));
        increment_dropped_stream_frames();
        increment_ws_reconnects();
        record_usage_event("settings_saved");

        let records = test_metric_records();
        assert!(records.iter().any(|record| {
            record.name == "releash.agent_stream.payload_bytes"
                && record.value == 128.0
                && has_attr(record, KEY_CHANNEL, "tauri_event")
        }));
        assert!(records.iter().any(|record| {
            record.name == "releash.session.save_bytes"
                && record.value == 256.0
                && has_attr(record, KEY_OPERATION, "session.append")
        }));
        assert!(records.iter().any(|record| {
            record.name == "releash.agent_stream.emit_interval_ms" && record.value == 33.0
        }));
        assert!(records
            .iter()
            .any(|record| record.name == "releash.agent_stream.dropped_frames"));
        assert!(records
            .iter()
            .any(|record| record.name == "releash.agent_stream.ws_reconnects"));
        assert!(records.iter().any(|record| {
            record.name == "releash.usage.events"
                && has_attr(record, KEY_USAGE_EVENT, "settings_saved")
        }));
        reset_test_metrics();
    }

    #[test]
    fn record_apis_are_noop_when_inactive() {
        let _guard = lock_test_telemetry();
        reset_test_metrics();
        set_active(false);

        record_emit_interval(Duration::from_millis(33));
        increment_dropped_stream_frames();
        increment_ws_reconnects();
        record_usage_event("settings_saved");

        assert!(test_metric_records().is_empty());
    }

    #[test]
    fn measure_result_returns_ok_and_err_unchanged_when_inactive() {
        let _guard = lock_test_telemetry();
        reset_test_metrics();
        set_active(false);

        let ok: Result<u32, &str> = measure_result(HotPathMetric::GitStatusScan, || Ok(7));
        let err: Result<u32, &str> = measure_result(HotPathMetric::GitStatusScan, || Err("boom"));

        assert_eq!(ok.unwrap(), 7);
        assert_eq!(err.unwrap_err(), "boom");
        assert!(test_metric_records().is_empty());
    }

    #[test]
    fn measure_result_maps_ok_and_err_status_when_active() {
        let _guard = lock_test_telemetry();
        reset_test_metrics();
        set_active(true);

        let _: Result<(), &str> = measure_result(HotPathMetric::GitStatusScan, || Ok(()));
        let _: Result<(), &str> = measure_result(HotPathMetric::GitStatusScan, || Err("boom"));

        let status_records = records_named("releash.operation.status");
        assert!(status_records.iter().any(|record| {
            has_attr(record, KEY_OPERATION, "git.status_scan")
                && has_attr(record, KEY_STATUS, "success")
        }));
        assert!(status_records.iter().any(|record| {
            has_attr(record, KEY_OPERATION, "git.status_scan")
                && has_attr(record, KEY_STATUS, "failure")
        }));
        reset_test_metrics();
    }

    #[test]
    fn first_repo_snapshot_ready_records_once_from_startup_origin() {
        let _guard = lock_test_telemetry();
        reset_test_metrics();
        set_active(true);
        set_startup_origin(Instant::now() - Duration::from_millis(250));

        record_first_repo_snapshot_ready();
        record_first_repo_snapshot_ready();

        let startup_records = records_named("releash.startup.duration_ms");
        assert_eq!(startup_records.len(), 1);
        assert!(has_attr(
            &startup_records[0],
            KEY_OPERATION,
            "startup.first_repo_snapshot_ready"
        ));
        assert!(startup_records[0].value >= 250.0);
        assert!(first_repo_snapshot_recorded_for_tests());
        reset_test_metrics();
    }

    #[test]
    fn first_repo_snapshot_ready_does_not_consume_guard_without_origin() {
        let _guard = lock_test_telemetry();
        reset_test_metrics();
        set_active(true);

        record_first_repo_snapshot_ready();

        assert!(test_metric_records().is_empty());
        assert!(!first_repo_snapshot_recorded_for_tests());
        reset_test_metrics();
    }
}
