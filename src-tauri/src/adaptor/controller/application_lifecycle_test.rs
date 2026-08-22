use std::sync::{Arc, Mutex};

use super::{shutdown_provider_observer_terminal_surface_and_local_api, workflow_shutdown_targets};

#[test]
fn test_通常終了_durable_targetはworkflow_executionだけで構成する() {
    let targets =
        workflow_shutdown_targets(vec!["workflow-1".to_string(), "workflow-2".to_string()]);

    assert_eq!(targets.len(), 2);
    assert!(targets
        .iter()
        .all(|target| target.kind == "workflow_execution"));
    assert_eq!(targets[0].target_id, "workflow-1");
    assert_eq!(targets[1].target_id, "workflow-2");
}

#[test]
fn test_通常終了_terminal_surface実行環境停止後にlocal_apiを停止する() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let observer_calls = Arc::clone(&calls);
    let terminal_calls = Arc::clone(&calls);
    let local_api_calls = Arc::clone(&calls);

    shutdown_provider_observer_terminal_surface_and_local_api(
        &move || {
            observer_calls
                .lock()
                .unwrap()
                .push("provider-exit-observer-stop");
        },
        &move || {
            terminal_calls
                .lock()
                .unwrap()
                .push("terminal-stop-drain-flush");
            Ok(())
        },
        &move || local_api_calls.lock().unwrap().push("local-api-stop"),
    )
    .unwrap();

    assert_eq!(
        calls.lock().unwrap().as_slice(),
        &[
            "provider-exit-observer-stop",
            "terminal-stop-drain-flush",
            "local-api-stop"
        ]
    );
}
