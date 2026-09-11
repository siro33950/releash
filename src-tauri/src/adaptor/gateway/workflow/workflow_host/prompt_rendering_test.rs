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

#[test]
fn test_delegate_起動指示は同一sessionでの再提出とturn終了と予約キーを説明する() {
    // Given
    let workflow: WorkflowDefinition = serde_saphyr::from_str("name: test\ndescription: test\nnodes:\n  main: {session: {provider: codex, facets: {instruction: implement}}, artifact: result, completion: {delegate: {child: check, when: child.ok, max_iterations: 2}}}\n  check: {command: check}\nschemas:\n  result: {type: object, properties: {done: {type: boolean}}, required: [done]}").unwrap();
    let facets = crate::domain::workflow::FacetContents {
        instruction: Some("implement".into()),
        ..Default::default()
    };
    // When
    let (_, prompt) = super::build_leaf_prompt(
        workflow.node_by_name("main").unwrap(),
        Some(&facets),
        "node-1",
        &[],
        &workflow.schemas,
    )
    .unwrap();
    // Then
    assert!(prompt.contains("同じnode-executionへArtifactを再提出"));
    assert!(prompt.contains("childキーはengineが管理"));
    assert!(prompt.contains("turnを終了"));
    assert!(prompt.contains("--node-execution node-1"));
    assert!(prompt.contains("--type result"));
}
