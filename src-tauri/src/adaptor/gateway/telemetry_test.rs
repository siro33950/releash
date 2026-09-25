use super::*;

#[test]
fn test_性能sample_output境界経由でも入力と起動の記録を保持する() {
    let _guard = metrics::lock_test_telemetry();
    metrics::reset_test_metrics();
    metrics::start_terminal_launch_sample_collection();
    TelemetryGateway
        .start_terminal_launch_phase(TerminalLaunch::CheckpointLookup)
        .finish();
    let samples = metrics::take_terminal_launch_samples();
    assert_eq!(samples.len(), 1);
    assert_eq!(samples[0].phase, "terminal.launch.checkpoint_lookup");
    assert!(samples[0].duration_ms >= 0.0);

    metrics::start_terminal_input_sample_collection();
    TelemetryGateway.start_terminal_input_trace("attachment", 42, metrics::unix_time_ms());
    TelemetryGateway.record_terminal_input_admission("attachment", 42);
    metrics::record_terminal_input_writer_enqueue("attachment", 42);
    let key = metrics::terminal_input_trace_key("attachment", 42).unwrap();
    metrics::record_terminal_input_output_read(&key);
    metrics::record_terminal_input_model_apply(&key);
    metrics::record_terminal_input_event_publish(&key);
    let samples = metrics::take_terminal_input_samples();
    assert_eq!(samples.len(), 1);
    let output: crate::usecase::telemetry::TerminalInputSample = samples[0].clone().into();
    assert_eq!(output.sequence, 42);
    assert_eq!(
        output.on_data_to_command_ingress_ms,
        samples[0].on_data_to_command_ingress_ms
    );
    assert_eq!(
        output.command_ingress_to_admission_ms,
        samples[0].command_ingress_to_admission_ms
    );
    assert_eq!(
        output.admission_to_writer_enqueue_ms,
        samples[0].admission_to_writer_enqueue_ms
    );
    assert_eq!(
        output.writer_enqueue_to_output_read_ms,
        samples[0].writer_enqueue_to_output_read_ms
    );
    assert_eq!(
        output.output_read_to_model_apply_ms,
        samples[0].output_read_to_model_apply_ms
    );
    assert_eq!(
        output.model_apply_to_event_publish_ms,
        samples[0].model_apply_to_event_publish_ms
    );
    assert_eq!(
        output.event_published_at_unix_ms,
        samples[0].event_published_at_unix_ms
    );
    metrics::reset_test_metrics();
}
