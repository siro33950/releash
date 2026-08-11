pub(crate) mod attributes;
mod resource;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::domain::workflow::FailureClassification;
#[cfg(test)]
use crate::domain::workflow::NodeExecutionFailureKind;
use attributes::{
    usage_event_allowed, HotPathMetric, OpStatus, StartupMetric, TerminalLaunchMetric,
    KEY_OPERATION, KEY_STATUS, KEY_USAGE_EVENT,
};
use opentelemetry::global;
use opentelemetry::metrics::{Counter, Histogram, ObservableGauge};
use opentelemetry::trace::{Span, Tracer};
use opentelemetry::KeyValue;
use resource::ProcessResourceObserver;

pub(crate) use attributes::HotPathMetric as HotPath;
pub(crate) use attributes::{StartupMetric as Startup, TerminalLaunchMetric as TerminalLaunch};

#[cfg(not(test))]
static PERFORMANCE_CONFIGURED: AtomicBool = AtomicBool::new(false);
#[cfg(not(test))]
static PERFORMANCE_ENABLED: AtomicBool = AtomicBool::new(true);
static MOUNTED_XTERM_COUNT: AtomicU64 = AtomicU64::new(0);
static ACTIVE_PTY_COUNT: AtomicU64 = AtomicU64::new(0);
static METRICS: OnceLock<Metrics> = OnceLock::new();
const TERMINAL_LAUNCH_SAMPLE_CAPACITY: usize = 4_096;
static TERMINAL_LAUNCH_SAMPLES: Mutex<Option<Vec<TerminalLaunchSample>>> = Mutex::new(None);
static TERMINAL_INPUT_SAMPLES: Mutex<
    Option<HashMap<TerminalInputTraceKey, PendingTerminalInputSample>>,
> = Mutex::new(None);
static TERMINAL_INPUT_COLLECTION_ACTIVE: AtomicBool = AtomicBool::new(false);
#[cfg(not(test))]
static STARTUP_ORIGIN: Mutex<Option<Instant>> = Mutex::new(None);
#[cfg(not(test))]
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
#[cfg(test)]
thread_local! {
    static PERFORMANCE_CONFIGURED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static PERFORMANCE_ENABLED: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
    static STARTUP_ORIGIN: std::cell::RefCell<Option<Instant>> = const { std::cell::RefCell::new(None) };
    static FIRST_REPO_SNAPSHOT_RECORDED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static TEST_TELEMETRY_RECORDING_ENABLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
fn test_telemetry_recording_enabled() -> bool {
    TEST_TELEMETRY_RECORDING_ENABLED.with(|enabled| enabled.get())
}

#[cfg(test)]
pub(crate) struct TestTelemetryGuard {
    _guard: std::sync::MutexGuard<'static, ()>,
}

#[cfg(test)]
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

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TerminalLaunchSample {
    pub(crate) phase: &'static str,
    pub(crate) duration_ms: f64,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub(crate) struct TerminalInputTraceKey {
    attachment_id: String,
    sequence: u64,
}

impl TerminalInputTraceKey {
    pub(crate) fn new(attachment_id: &str, sequence: u64) -> Self {
        Self {
            attachment_id: attachment_id.to_string(),
            sequence,
        }
    }

    pub(crate) fn attachment_id(&self) -> &str {
        &self.attachment_id
    }

    pub(crate) fn sequence(&self) -> u64 {
        self.sequence
    }
}

struct PendingTerminalInputSample {
    sequence: u64,
    on_data_to_command_ingress_ms: f64,
    command_ingress: Instant,
    admission: Option<Instant>,
    writer_enqueue: Option<Instant>,
    output_read: Option<Instant>,
    model_apply: Option<Instant>,
    event_publish: Option<Instant>,
    event_published_at_unix_ms: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TerminalInputSample {
    pub(crate) sequence: u64,
    pub(crate) on_data_to_command_ingress_ms: f64,
    pub(crate) command_ingress_to_admission_ms: f64,
    pub(crate) admission_to_writer_enqueue_ms: f64,
    pub(crate) writer_enqueue_to_output_read_ms: f64,
    pub(crate) output_read_to_model_apply_ms: f64,
    pub(crate) model_apply_to_event_publish_ms: f64,
    pub(crate) event_published_at_unix_ms: f64,
}

pub(crate) fn unix_time_ms() -> f64 {
    duration_to_unix_time_ms(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default(),
    )
}

fn duration_to_unix_time_ms(duration: Duration) -> f64 {
    duration.as_millis() as f64
}

fn between_ms(start: Instant, end: Instant) -> f64 {
    end.checked_duration_since(start)
        .unwrap_or_default()
        .as_secs_f64()
        * 1_000.0
}

pub(crate) fn start_terminal_input_sample_collection() {
    *TERMINAL_INPUT_SAMPLES
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = Some(HashMap::new());
    TERMINAL_INPUT_COLLECTION_ACTIVE.store(true, Ordering::Release);
}

pub(crate) fn terminal_input_trace_key(
    attachment_id: &str,
    sequence: u64,
) -> Option<TerminalInputTraceKey> {
    TERMINAL_INPUT_COLLECTION_ACTIVE
        .load(Ordering::Acquire)
        .then(|| TerminalInputTraceKey::new(attachment_id, sequence))
}

pub(crate) fn start_terminal_input_trace(
    attachment_id: &str,
    sequence: u64,
    on_data_at_unix_ms: f64,
) {
    if !on_data_at_unix_ms.is_finite() {
        return;
    }
    let command_ingress = Instant::now();
    let mut collection = TERMINAL_INPUT_SAMPLES
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let Some(samples) = collection.as_mut() else {
        return;
    };
    if samples.len() >= TERMINAL_LAUNCH_SAMPLE_CAPACITY {
        return;
    }
    samples.insert(
        TerminalInputTraceKey::new(attachment_id, sequence),
        PendingTerminalInputSample {
            sequence,
            on_data_to_command_ingress_ms: (unix_time_ms() - on_data_at_unix_ms).max(0.0),
            command_ingress,
            admission: None,
            writer_enqueue: None,
            output_read: None,
            model_apply: None,
            event_publish: None,
            event_published_at_unix_ms: None,
        },
    );
}

fn update_terminal_input_sample(
    attachment_id: &str,
    sequence: u64,
    update: impl FnOnce(&mut PendingTerminalInputSample, Instant),
) {
    let mut collection = TERMINAL_INPUT_SAMPLES
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let Some(sample) = collection
        .as_mut()
        .and_then(|samples| samples.get_mut(&TerminalInputTraceKey::new(attachment_id, sequence)))
    else {
        return;
    };
    update(sample, Instant::now());
}

pub(crate) fn record_terminal_input_admission(attachment_id: &str, sequence: u64) {
    update_terminal_input_sample(attachment_id, sequence, |sample, now| {
        sample.admission = Some(now);
    });
}

pub(crate) fn record_terminal_input_writer_enqueue(attachment_id: &str, sequence: u64) {
    update_terminal_input_sample(attachment_id, sequence, |sample, now| {
        sample.writer_enqueue = Some(now);
    });
}

pub(crate) fn record_terminal_input_output_read(key: &TerminalInputTraceKey) {
    update_terminal_input_sample(key.attachment_id(), key.sequence(), |sample, now| {
        sample.output_read = Some(now);
    });
}

pub(crate) fn record_terminal_input_model_apply(key: &TerminalInputTraceKey) {
    update_terminal_input_sample(key.attachment_id(), key.sequence(), |sample, now| {
        sample.model_apply = Some(now);
    });
}

pub(crate) fn record_terminal_input_event_publish(key: &TerminalInputTraceKey) {
    update_terminal_input_sample(key.attachment_id(), key.sequence(), |sample, now| {
        sample.event_publish = Some(now);
        sample.event_published_at_unix_ms = Some(unix_time_ms());
    });
}

pub(crate) fn take_terminal_input_samples() -> Vec<TerminalInputSample> {
    TERMINAL_INPUT_COLLECTION_ACTIVE.store(false, Ordering::Release);
    let samples = TERMINAL_INPUT_SAMPLES
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take()
        .unwrap_or_default();
    let mut completed = samples
        .into_values()
        .filter_map(|sample| {
            let admission = sample.admission?;
            let writer_enqueue = sample.writer_enqueue?;
            let output_read = sample.output_read?;
            let model_apply = sample.model_apply?;
            let event_publish = sample.event_publish?;
            Some(TerminalInputSample {
                sequence: sample.sequence,
                on_data_to_command_ingress_ms: sample.on_data_to_command_ingress_ms,
                command_ingress_to_admission_ms: between_ms(sample.command_ingress, admission),
                admission_to_writer_enqueue_ms: between_ms(admission, writer_enqueue),
                writer_enqueue_to_output_read_ms: between_ms(writer_enqueue, output_read),
                output_read_to_model_apply_ms: between_ms(output_read, model_apply),
                model_apply_to_event_publish_ms: between_ms(model_apply, event_publish),
                event_published_at_unix_ms: sample.event_published_at_unix_ms?,
            })
        })
        .collect::<Vec<_>>();
    completed.sort_by_key(|sample| sample.sequence);
    completed
}

pub(crate) fn start_terminal_launch_sample_collection() {
    *TERMINAL_LAUNCH_SAMPLES
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = Some(Vec::new());
}

pub(crate) fn take_terminal_launch_samples() -> Vec<TerminalLaunchSample> {
    TERMINAL_LAUNCH_SAMPLES
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take()
        .unwrap_or_default()
}

#[cfg(not(test))]
fn store_performance_configured(configured: bool) {
    PERFORMANCE_CONFIGURED.store(configured, Ordering::Relaxed);
}

#[cfg(test)]
fn store_performance_configured(configured: bool) {
    PERFORMANCE_CONFIGURED.with(|value| value.set(configured));
}

#[cfg(not(test))]
fn store_performance_enabled(enabled: bool) {
    PERFORMANCE_ENABLED.store(enabled, Ordering::Relaxed);
}

#[cfg(test)]
fn store_performance_enabled(enabled: bool) {
    PERFORMANCE_ENABLED.with(|value| value.set(enabled));
}

#[cfg(not(test))]
fn load_performance_configured() -> bool {
    PERFORMANCE_CONFIGURED.load(Ordering::Relaxed)
}

#[cfg(test)]
fn load_performance_configured() -> bool {
    PERFORMANCE_CONFIGURED.with(|value| value.get())
}

#[cfg(not(test))]
fn load_performance_enabled() -> bool {
    PERFORMANCE_ENABLED.load(Ordering::Relaxed)
}

#[cfg(test)]
fn load_performance_enabled() -> bool {
    PERFORMANCE_ENABLED.with(|value| value.get())
}

#[cfg(not(test))]
fn store_startup_origin(origin: Option<Instant>) {
    *STARTUP_ORIGIN.lock().unwrap_or_else(|e| e.into_inner()) = origin;
}

#[cfg(test)]
fn store_startup_origin(origin: Option<Instant>) {
    STARTUP_ORIGIN.with(|value| *value.borrow_mut() = origin);
}

#[cfg(not(test))]
fn load_startup_elapsed() -> Option<Duration> {
    STARTUP_ORIGIN
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .map(|origin| origin.elapsed())
}

#[cfg(test)]
fn load_startup_elapsed() -> Option<Duration> {
    STARTUP_ORIGIN.with(|value| value.borrow().map(|origin| origin.elapsed()))
}

#[cfg(not(test))]
fn reset_first_repo_snapshot_recorded() {
    FIRST_REPO_SNAPSHOT_RECORDED.store(false, Ordering::Relaxed);
}

#[cfg(test)]
fn reset_first_repo_snapshot_recorded() {
    FIRST_REPO_SNAPSHOT_RECORDED.with(|value| value.set(false));
}

#[cfg(not(test))]
fn mark_first_repo_snapshot_recorded() -> bool {
    FIRST_REPO_SNAPSHOT_RECORDED.swap(true, Ordering::AcqRel)
}

#[cfg(test)]
fn mark_first_repo_snapshot_recorded() -> bool {
    FIRST_REPO_SNAPSHOT_RECORDED.with(|value| {
        let already_recorded = value.get();
        value.set(true);
        already_recorded
    })
}

#[cfg(test)]
fn first_repo_snapshot_recorded() -> bool {
    FIRST_REPO_SNAPSHOT_RECORDED.with(|value| value.get())
}

pub(crate) fn set_performance_configured(configured: bool) {
    store_performance_configured(configured);
}

pub(crate) fn set_performance_enabled(enabled: bool) {
    store_performance_enabled(enabled);
}

pub(crate) fn set_startup_origin(origin: Instant) {
    store_startup_origin(Some(origin));
    reset_first_repo_snapshot_recorded();
}

pub(crate) fn is_performance_active() -> bool {
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

pub(crate) fn record_startup_from_origin(metric: StartupMetric) {
    if let Some(elapsed) = startup_elapsed() {
        record_startup(metric, elapsed);
    }
}

pub(crate) fn record_first_repo_snapshot_ready() {
    if !is_performance_active() {
        return;
    }
    #[cfg(test)]
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

#[cfg(test)]
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

#[cfg(test)]
pub(crate) fn reset_test_metrics() {
    TEST_METRIC_RECORDS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clear();
    store_startup_origin(None);
    reset_first_repo_snapshot_recorded();
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
    first_repo_snapshot_recorded()
}

#[cfg(test)]
pub(crate) fn lock_test_telemetry() -> TestTelemetryGuard {
    let guard = TEST_TELEMETRY_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    TEST_TELEMETRY_RECORDING_ENABLED.with(|enabled| enabled.set(true));
    TestTelemetryGuard { _guard: guard }
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

pub(crate) fn record_workflow_node_failure(
    classification: FailureClassification,
    retry_count: Option<u32>,
) {
    if !is_performance_active() {
        return;
    }
    let mut attrs = vec![
        KeyValue::new(KEY_OPERATION, "workflow.node.failure"),
        KeyValue::new(KEY_STATUS, OpStatus::Failure.as_str()),
        KeyValue::new(attributes::KEY_FAILURE_KIND, classification.kind.as_str()),
        KeyValue::new(
            attributes::KEY_FAILURE_DISPOSITION,
            classification.disposition.as_str(),
        ),
    ];
    if let Some(retry_count) = retry_count {
        attrs.push(KeyValue::new(
            attributes::KEY_RETRY_COUNT,
            retry_count.to_string(),
        ));
    }
    if let Some(timeout_kind) = classification.timeout_kind {
        attrs.push(KeyValue::new(
            attributes::KEY_TIMEOUT_KIND,
            timeout_kind.as_str(),
        ));
    }
    #[cfg(test)]
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
    if let Some(samples) = TERMINAL_LAUNCH_SAMPLES
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .as_mut()
    {
        if samples.len() < TERMINAL_LAUNCH_SAMPLE_CAPACITY {
            samples.push(TerminalLaunchSample {
                phase: metric.operation(),
                duration_ms,
            });
        }
    }
    if !is_performance_active() {
        return;
    }
    let attrs = [KeyValue::new(KEY_OPERATION, metric.operation())];
    #[cfg(test)]
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
    use std::sync::atomic::Ordering;

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
    fn test_terminal_launch_sample_collection_is_explicit_bounded_and_drainable() {
        let _guard = lock_test_telemetry();
        reset_test_metrics();
        set_active(true);

        record_terminal_launch(TerminalLaunch::CommandIngress, Duration::from_millis(1));
        assert!(take_terminal_launch_samples().is_empty());

        start_terminal_launch_sample_collection();
        for _ in 0..4_097 {
            record_terminal_launch(TerminalLaunch::FirstProviderByte, Duration::from_millis(2));
        }
        let samples = take_terminal_launch_samples();
        assert_eq!(samples.len(), 4_096);
        assert_eq!(samples[0].phase, "terminal.launch.first_provider_byte");
        assert_eq!(samples[0].duration_ms, 2.0);

        record_terminal_launch(TerminalLaunch::FirstPaint, Duration::from_millis(3));
        assert!(take_terminal_launch_samples().is_empty());
        reset_test_metrics();
    }

    #[test]
    fn test_explicit_terminal_launch_collection_does_not_depend_on_telemetry_export_setting() {
        let _guard = lock_test_telemetry();
        reset_test_metrics();
        set_active(false);

        start_terminal_launch_sample_collection();
        record_terminal_launch(TerminalLaunch::CommandIngress, Duration::from_millis(4));

        assert_eq!(
            take_terminal_launch_samples(),
            vec![TerminalLaunchSample {
                phase: "terminal.launch.command_ingress",
                duration_ms: 4.0,
            }]
        );
        reset_test_metrics();
    }

    #[test]
    fn workflow_node_failure_records_failure_attributes() {
        let _guard = lock_test_telemetry();
        reset_test_metrics();
        set_active(true);

        record_workflow_node_failure(
            FailureClassification::new(NodeExecutionFailureKind::StartupTimeout),
            Some(2),
        );

        let records = records_named("releash.operation.status");
        assert!(records.iter().any(|record| {
            has_attr(record, KEY_OPERATION, "workflow.node.failure")
                && has_attr(record, KEY_STATUS, "failure")
                && has_attr(record, attributes::KEY_FAILURE_KIND, "startup_timeout")
                && has_attr(record, attributes::KEY_FAILURE_DISPOSITION, "retryable")
                && has_attr(record, attributes::KEY_RETRY_COUNT, "2")
                && has_attr(record, attributes::KEY_TIMEOUT_KIND, "startup")
        }));
        reset_test_metrics();
    }

    #[test]
    fn workflow_node_failure_records_user_abort_disposition() {
        let _guard = lock_test_telemetry();
        reset_test_metrics();
        set_active(true);

        record_workflow_node_failure(
            FailureClassification::new(NodeExecutionFailureKind::UserAbort),
            None,
        );

        let records = records_named("releash.operation.status");
        assert!(records.iter().any(|record| {
            has_attr(record, KEY_OPERATION, "workflow.node.failure")
                && has_attr(record, attributes::KEY_FAILURE_KIND, "user_abort")
                && has_attr(
                    record,
                    attributes::KEY_FAILURE_DISPOSITION,
                    "user-action-required",
                )
        }));
        reset_test_metrics();
    }

    #[test]
    fn record_apis_are_noop_when_inactive() {
        let _guard = lock_test_telemetry();
        reset_test_metrics();
        set_active(false);

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

    #[test]
    fn test_terminal_input_sample_requires_the_complete_backend_path() {
        start_terminal_input_sample_collection();
        let on_data_at_unix_ms = unix_time_ms() - 1.0;
        start_terminal_input_trace("attachment-a", 7, on_data_at_unix_ms);
        record_terminal_input_admission("attachment-a", 7);
        record_terminal_input_writer_enqueue("attachment-a", 7);
        let key = TerminalInputTraceKey::new("attachment-a", 7);
        record_terminal_input_output_read(&key);
        record_terminal_input_model_apply(&key);
        record_terminal_input_event_publish(&key);

        let samples = take_terminal_input_samples();

        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].sequence, 7);
        assert!(samples[0].on_data_to_command_ingress_ms >= 0.0);
        assert!(samples[0].command_ingress_to_admission_ms >= 0.0);
        assert!(samples[0].admission_to_writer_enqueue_ms >= 0.0);
        assert!(samples[0].writer_enqueue_to_output_read_ms >= 0.0);
        assert!(samples[0].output_read_to_model_apply_ms >= 0.0);
        assert!(samples[0].model_apply_to_event_publish_ms >= 0.0);
        assert!(samples[0].event_published_at_unix_ms >= on_data_at_unix_ms);
    }

    #[test]
    fn test_terminal_input_cross_process_timestamp_uses_integer_unix_milliseconds() {
        assert_eq!(
            duration_to_unix_time_ms(Duration::new(1, 999_999_999)),
            1_999.0
        );
    }
}
