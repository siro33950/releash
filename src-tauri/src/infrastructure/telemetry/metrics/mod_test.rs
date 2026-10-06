pub(crate) mod tests {
    use super::super::*;
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
    fn test_端末起動時間_利用者の匿名計測設定が有効なときだけotlpへ記録する() {
        // Given
        let _guard = lock_test_telemetry();
        reset_test_metrics();
        for enabled in [false, true] {
            set_active(enabled);
            // When
            record_terminal_launch(TerminalLaunch::CheckpointLookup, Duration::from_millis(7));
            // Then
            let records = records_named("releash.terminal.launch.duration_ms");
            assert_eq!(records.len(), usize::from(enabled));
            if enabled {
                assert_eq!(records[0].value, 7.0);
                assert!(has_attr(
                    &records[0],
                    KEY_OPERATION,
                    "terminal.launch.checkpoint_lookup"
                ));
            }
        }
        reset_test_metrics();
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
    fn workflow_node_failure_records_failure_attributes() {
        let _guard = lock_test_telemetry();
        reset_test_metrics();
        set_active(true);

        record_workflow_node_failure("startup_timeout", "retryable", Some("startup"), Some(2));

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

        record_workflow_node_failure("user_abort", "user-action-required", None, None);

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
}
