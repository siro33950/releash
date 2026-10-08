use super::*;

#[test]
fn test_端末起動段階_gatewayの完了がoperation付きでotlpへ記録される() {
    // Given
    let _guard = metrics::lock_test_telemetry();
    metrics::reset_test_metrics();
    metrics::set_performance_configured(true);
    metrics::set_performance_enabled(true);
    // When
    TelemetryGateway
        .start_terminal_launch_phase(TerminalLaunch::CheckpointLookup)
        .finish();
    // Then
    let records: Vec<_> = metrics::test_metric_records()
        .into_iter()
        .filter(|record| record.name == "releash.terminal.launch.duration_ms")
        .collect();
    assert_eq!(records.len(), 1);
    assert!(records[0]
        .attributes
        .iter()
        .any(|(key, value)| key == "releash.operation"
            && value == "terminal.launch.checkpoint_lookup"));
    metrics::reset_test_metrics();
}
