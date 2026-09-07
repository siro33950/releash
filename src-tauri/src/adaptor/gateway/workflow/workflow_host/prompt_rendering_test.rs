use super::render_parameter_references;
use crate::domain::workflow::WorkflowDefinition;
use crate::infrastructure::process::command_runner::{spawn_shell_command, OutputLimit};
use serde_json::{json, Value};
use tempfile::TempDir;

#[tokio::test]
async fn test_fanout集約command_fixtureのjqがmapの全slotのlgtmを判定する() {
    // Given
    let workflow: WorkflowDefinition =
        serde_saphyr::from_str(include_str!("../fixtures/valid/fanout-command-reducer.yml"))
            .unwrap();
    let command = workflow.node_by_name("judge").unwrap().command().unwrap();
    let cwd = TempDir::new().unwrap();

    for (reviews, expected) in [
        (
            json!({"review-a": {"lgtm": true}, "review-b": {"lgtm": true}}),
            true,
        ),
        (
            json!({"review-a": {"lgtm": true}, "review-b": {"lgtm": false}}),
            false,
        ),
    ] {
        let bindings = [("reviews".to_string(), reviews)];

        // When
        let rendered = render_parameter_references(command, &bindings);
        let output = spawn_shell_command(
            cwd.path(),
            &rendered,
            std::iter::empty::<(String, String)>(),
            "fanout command reducer",
            OutputLimit {
                max_bytes: 4096,
                truncation_marker: "[truncated]",
            },
        )
        .unwrap()
        .wait()
        .await
        .unwrap();

        // Then
        assert_eq!(output.exit_code, 0, "{output:?}");
        let artifact: Value = serde_json::from_str(&output.stdout).unwrap();
        assert_eq!(artifact["all_lgtm"], json!(expected), "{bindings:?}");
    }
}
