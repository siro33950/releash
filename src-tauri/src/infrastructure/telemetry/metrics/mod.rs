pub(crate) mod attributes;
pub(crate) mod resource;

#[cfg(not(any(test, feature = "test-support")))]
use std::sync::atomic::AtomicBool;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use attributes::{
    usage_event_allowed, HotPathMetric, OpStatus, StartupMetric, TerminalLaunchMetric,
    KEY_OPERATION, KEY_STATUS, KEY_USAGE_EVENT,
};
use opentelemetry::global;
use opentelemetry::metrics::{Counter, Histogram, ObservableGauge};
use opentelemetry::KeyValue;
use resource::ProcessResourceObserver;

pub(crate) use attributes::HotPathMetric as HotPath;

pub use attributes::StartupMetric as Startup;
pub(crate) use attributes::TerminalLaunchMetric as TerminalLaunch;

#[cfg(not(any(test, feature = "test-support")))]
static PERFORMANCE_CONFIGURED: AtomicBool = AtomicBool::new(false);
#[cfg(not(any(test, feature = "test-support")))]
static PERFORMANCE_ENABLED: AtomicBool = AtomicBool::new(true);
static MOUNTED_XTERM_COUNT: AtomicU64 = AtomicU64::new(0);
static ACTIVE_PTY_COUNT: AtomicU64 = AtomicU64::new(0);
static METRICS: OnceLock<Metrics> = OnceLock::new();
#[cfg(not(any(test, feature = "test-support")))]
static STARTUP_ORIGIN: Mutex<Option<Instant>> = Mutex::new(None);
#[cfg(not(any(test, feature = "test-support")))]
static FIRST_REPO_SNAPSHOT_RECORDED: AtomicBool = AtomicBool::new(false);

#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Debug, PartialEq)]
pub struct TestMetricRecord {
    pub name: &'static str,
    pub value: f64,
    pub attributes: Vec<(String, String)>,
}

#[cfg(any(test, feature = "test-support"))]
static TEST_METRIC_RECORDS: Mutex<Vec<TestMetricRecord>> = Mutex::new(Vec::new());
#[cfg(any(test, feature = "test-support"))]
static TEST_TELEMETRY_LOCK: Mutex<()> = Mutex::new(());
#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static PERFORMANCE_CONFIGURED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static PERFORMANCE_ENABLED: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
    static STARTUP_ORIGIN: std::cell::RefCell<Option<Instant>> = const { std::cell::RefCell::new(None) };
    static FIRST_REPO_SNAPSHOT_RECORDED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static TEST_TELEMETRY_RECORDING_ENABLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(any(test, feature = "test-support"))]
fn test_telemetry_recording_enabled() -> bool {
    TEST_TELEMETRY_RECORDING_ENABLED.with(|enabled| enabled.get())
}

#[cfg(any(test, feature = "test-support"))]
pub struct TestTelemetryGuard {
    _guard: std::sync::MutexGuard<'static, ()>,
}

#[cfg(any(test, feature = "test-support"))]
impl Drop for TestTelemetryGuard {
    fn drop(&mut self) {
        TEST_TELEMETRY_RECORDING_ENABLED.with(|enabled| enabled.set(false));
    }
}

struct Metrics {
    hot_path_duration: Histogram<f64>,
    startup_duration: Histogram<f64>,
    terminal_launch_duration: Histogram<f64>,
    operation_status: Counter<u64>,
    usage_events: Counter<u64>,
    _rss_gauge: ObservableGauge<u64>,
    _cpu_gauge: ObservableGauge<f64>,
    _xterm_gauge: ObservableGauge<u64>,
    _pty_gauge: ObservableGauge<u64>,
}

#[cfg(not(any(test, feature = "test-support")))]
fn store_performance_configured(configured: bool) {
    PERFORMANCE_CONFIGURED.store(configured, Ordering::Relaxed);
}

#[cfg(any(test, feature = "test-support"))]
fn store_performance_configured(configured: bool) {
    PERFORMANCE_CONFIGURED.with(|value| value.set(configured));
}

#[cfg(not(any(test, feature = "test-support")))]
fn store_performance_enabled(enabled: bool) {
    PERFORMANCE_ENABLED.store(enabled, Ordering::Relaxed);
}

#[cfg(any(test, feature = "test-support"))]
fn store_performance_enabled(enabled: bool) {
    PERFORMANCE_ENABLED.with(|value| value.set(enabled));
}

#[cfg(not(any(test, feature = "test-support")))]
fn load_performance_configured() -> bool {
    PERFORMANCE_CONFIGURED.load(Ordering::Relaxed)
}

#[cfg(any(test, feature = "test-support"))]
fn load_performance_configured() -> bool {
    PERFORMANCE_CONFIGURED.with(|value| value.get())
}

#[cfg(not(any(test, feature = "test-support")))]
fn load_performance_enabled() -> bool {
    PERFORMANCE_ENABLED.load(Ordering::Relaxed)
}

#[cfg(any(test, feature = "test-support"))]
fn load_performance_enabled() -> bool {
    PERFORMANCE_ENABLED.with(|value| value.get())
}

#[cfg(not(any(test, feature = "test-support")))]
fn store_startup_origin(origin: Option<Instant>) {
    *STARTUP_ORIGIN.lock().unwrap_or_else(|e| e.into_inner()) = origin;
}

#[cfg(any(test, feature = "test-support"))]
fn store_startup_origin(origin: Option<Instant>) {
    STARTUP_ORIGIN.with(|value| *value.borrow_mut() = origin);
}

#[cfg(not(any(test, feature = "test-support")))]
fn load_startup_elapsed() -> Option<Duration> {
    STARTUP_ORIGIN
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .map(|origin| origin.elapsed())
}

#[cfg(any(test, feature = "test-support"))]
fn load_startup_elapsed() -> Option<Duration> {
    STARTUP_ORIGIN.with(|value| value.borrow().map(|origin| origin.elapsed()))
}

#[cfg(not(any(test, feature = "test-support")))]
fn reset_first_repo_snapshot_recorded() {
    FIRST_REPO_SNAPSHOT_RECORDED.store(false, Ordering::Relaxed);
}

#[cfg(any(test, feature = "test-support"))]
fn reset_first_repo_snapshot_recorded() {
    FIRST_REPO_SNAPSHOT_RECORDED.with(|value| value.set(false));
}

#[cfg(not(any(test, feature = "test-support")))]
fn mark_first_repo_snapshot_recorded() -> bool {
    FIRST_REPO_SNAPSHOT_RECORDED.swap(true, Ordering::AcqRel)
}

#[cfg(any(test, feature = "test-support"))]
fn mark_first_repo_snapshot_recorded() -> bool {
    FIRST_REPO_SNAPSHOT_RECORDED.with(|value| {
        let already_recorded = value.get();
        value.set(true);
        already_recorded
    })
}

#[cfg(any(test, feature = "test-support"))]
fn first_repo_snapshot_recorded() -> bool {
    FIRST_REPO_SNAPSHOT_RECORDED.with(|value| value.get())
}

pub fn set_performance_configured(configured: bool) {
    store_performance_configured(configured);
}

pub fn set_performance_enabled(enabled: bool) {
    store_performance_enabled(enabled);
}

pub fn set_startup_origin(origin: Instant) {
    store_startup_origin(Some(origin));
    reset_first_repo_snapshot_recorded();
}

pub fn is_performance_active() -> bool {
    load_performance_configured() && load_performance_enabled()
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
        terminal_launch_duration: meter
            .f64_histogram("releash.terminal.launch.duration_ms")
            .with_unit("ms")
            .build(),
        operation_status: meter.u64_counter("releash.operation.status").build(),
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
    load_startup_elapsed()
}

pub fn record_startup_from_origin(metric: StartupMetric) {
    if let Some(elapsed) = startup_elapsed() {
        record_startup(metric, elapsed);
    }
}

pub fn record_first_repo_snapshot_ready() {
    if !is_performance_active() {
        return;
    }
    #[cfg(any(test, feature = "test-support"))]
    if !test_telemetry_recording_enabled() {
        return;
    }
    let Some(elapsed) = startup_elapsed() else {
        return;
    };
    if mark_first_repo_snapshot_recorded() {
        return;
    }
    record_startup(StartupMetric::FirstRepoSnapshotReady, elapsed);
}

#[cfg(any(test, feature = "test-support"))]
fn record_test_metric(name: &'static str, value: f64, attrs: &[KeyValue]) {
    if !TEST_TELEMETRY_RECORDING_ENABLED.with(|enabled| enabled.get()) {
        return;
    }
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

#[cfg(any(test, feature = "test-support"))]
pub fn reset_test_metrics() {
    TEST_METRIC_RECORDS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clear();
    store_startup_origin(None);
    reset_first_repo_snapshot_recorded();
    set_performance_configured(false);
    set_performance_enabled(true);
}

#[cfg(any(test, feature = "test-support"))]
pub fn test_metric_records() -> Vec<TestMetricRecord> {
    TEST_METRIC_RECORDS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

#[cfg(any(test, feature = "test-support"))]
pub fn first_repo_snapshot_recorded_for_tests() -> bool {
    first_repo_snapshot_recorded()
}

#[cfg(any(test, feature = "test-support"))]
pub fn lock_test_telemetry() -> TestTelemetryGuard {
    let guard = TEST_TELEMETRY_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    TEST_TELEMETRY_RECORDING_ENABLED.with(|enabled| enabled.set(true));
    TestTelemetryGuard { _guard: guard }
}

pub(crate) fn measure_result<T, E>(
    metric: HotPathMetric,
    operation: impl FnOnce() -> Result<T, E>,
) -> Result<T, E> {
    crate::common::telemetry::measure_result(
        is_performance_active(),
        metric.span_name(),
        metric.operation(),
        operation,
        |success, elapsed| {
            record_hot_path_duration(
                metric,
                if success {
                    OpStatus::Success
                } else {
                    OpStatus::Failure
                },
                elapsed,
            );
        },
    )
}

pub(crate) fn record_hot_path_duration(metric: HotPathMetric, status: OpStatus, elapsed: Duration) {
    if !is_performance_active() {
        return;
    }
    let attrs = [
        KeyValue::new(KEY_OPERATION, metric.operation()),
        KeyValue::new(KEY_STATUS, status.as_str()),
    ];
    #[cfg(any(test, feature = "test-support"))]
    record_test_metric(
        "releash.hot_path.duration_ms",
        elapsed.as_secs_f64() * 1000.0,
        &attrs,
    );
    #[cfg(any(test, feature = "test-support"))]
    record_test_metric("releash.operation.status", 1.0, &attrs);
    let Some(metrics) = METRICS.get() else {
        return;
    };
    metrics
        .hot_path_duration
        .record(elapsed.as_secs_f64() * 1000.0, &attrs);
    metrics.operation_status.add(1, &attrs);
}

pub(crate) fn record_workflow_node_failure(
    kind: &'static str,
    disposition: &'static str,
    timeout_kind: Option<&'static str>,
    retry_count: Option<u32>,
) {
    if !is_performance_active() {
        return;
    }
    let mut attrs = vec![
        KeyValue::new(KEY_OPERATION, "workflow.node.failure"),
        KeyValue::new(KEY_STATUS, OpStatus::Failure.as_str()),
        KeyValue::new(attributes::KEY_FAILURE_KIND, kind),
        KeyValue::new(attributes::KEY_FAILURE_DISPOSITION, disposition),
    ];
    if let Some(retry_count) = retry_count {
        attrs.push(KeyValue::new(
            attributes::KEY_RETRY_COUNT,
            retry_count.to_string(),
        ));
    }
    if let Some(timeout_kind) = timeout_kind {
        attrs.push(KeyValue::new(attributes::KEY_TIMEOUT_KIND, timeout_kind));
    }
    #[cfg(any(test, feature = "test-support"))]
    record_test_metric("releash.operation.status", 1.0, &attrs);
    let Some(metrics) = METRICS.get() else {
        return;
    };
    metrics.operation_status.add(1, &attrs);
}

pub(crate) fn record_startup(metric: StartupMetric, elapsed: Duration) {
    if !is_performance_active() {
        return;
    }
    let attrs = [KeyValue::new(KEY_OPERATION, metric.operation())];
    #[cfg(any(test, feature = "test-support"))]
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

pub(crate) struct TerminalLaunchPhaseTimer {
    metric: TerminalLaunchMetric,
    started: Instant,
}

impl TerminalLaunchPhaseTimer {
    pub(crate) fn finish(self) {
        record_terminal_launch(self.metric, self.started.elapsed());
    }
}

pub(crate) fn start_terminal_launch_phase(
    metric: TerminalLaunchMetric,
) -> TerminalLaunchPhaseTimer {
    TerminalLaunchPhaseTimer {
        metric,
        started: Instant::now(),
    }
}

pub(crate) fn record_terminal_launch(metric: TerminalLaunchMetric, elapsed: Duration) {
    let duration_ms = elapsed.as_secs_f64() * 1000.0;
    if !is_performance_active() {
        return;
    }
    let attrs = [KeyValue::new(KEY_OPERATION, metric.operation())];
    #[cfg(any(test, feature = "test-support"))]
    record_test_metric("releash.terminal.launch.duration_ms", duration_ms, &attrs);
    let Some(metrics) = METRICS.get() else {
        return;
    };
    metrics.terminal_launch_duration.record(duration_ms, &attrs);
}

pub(crate) fn record_usage_event(name: &str) {
    if !is_performance_active() || !usage_event_allowed(name) {
        return;
    }
    let attrs = [KeyValue::new(KEY_USAGE_EVENT, name.to_string())];
    #[cfg(any(test, feature = "test-support"))]
    record_test_metric("releash.usage.events", 1.0, &attrs);
    let Some(metrics) = METRICS.get() else {
        return;
    };
    metrics.usage_events.add(1, &attrs);
}

#[cfg(test)]
#[path = "mod_test.rs"]
pub(crate) mod mod_tests;
