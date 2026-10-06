use super::*;

#[test]
fn test_local_apiの段はoutputとhookがworkflowでその他がdefault() {
    for path in [
        "/v1/provider-lifecycle/signals",
        "/v1/workflow/node-executions/node/submit",
        "/v1/workflow/executions/execution/artifacts:validate",
        "/v1/workflow/executions/execution/artifacts/node",
    ] {
        assert_eq!(local_priority_level(path), Some("workflow"));
    }
    for path in [
        "/v1/workflow/executions",
        "/v1/workflow/executions/execution/abort",
    ] {
        assert_eq!(local_priority_level(path), Some("default"));
    }
}
