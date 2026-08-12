use super::*;

#[test]
fn test_cli長文help_対応中のagent向けcommandだけを表示する() {
    let help = render_long_help();
    for command in [
        "$ releash workflow --help",
        "$ releash workflow status --help",
        "$ releash workflow output submit --help",
        "$ releash workflow output get --help",
        "$ releash review list --help",
        "--node",
        "--node-execution",
        "--type",
        "EXECUTION_ID",
    ] {
        assert!(help.contains(command), "missing nested help: {command}");
    }
    for removed in [
        "$ releash workflow list --help",
        "$ releash workflow start --help",
        "$ releash workflow executions --help",
        "$ releash workflow logs --help",
        "$ releash workflow approve --help",
        "$ releash workflow abort --help",
        "$ releash workflow stop --help",
        "$ releash workflow resume --help",
        "$ releash workflow output validate --help",
        "$ releash hook",
    ] {
        assert!(!help.contains(removed), "removed CLI surface: {removed}");
    }
}

#[test]
fn test_clicommand整理_削除対象workflow_commandを拒否する() {
    let execution_id = "550e8400-e29b-41d4-a716-446655440000";
    for argv in [
        vec!["releash", "workflow", "list"],
        vec!["releash", "workflow", "start", "review"],
        vec!["releash", "workflow", "executions"],
        vec!["releash", "workflow", "logs", execution_id],
        vec![
            "releash",
            "workflow",
            "approve",
            execution_id,
            "--node",
            "review",
        ],
        vec!["releash", "workflow", "abort", execution_id],
        vec!["releash", "workflow", "stop", execution_id],
        vec!["releash", "workflow", "resume", execution_id],
        vec![
            "releash",
            "workflow",
            "output",
            "validate",
            execution_id,
            "--node",
            "review",
            "--type",
            "review-verdict",
            "--file",
            "out.json",
        ],
    ] {
        assert!(
            Cli::try_parse_from(argv.clone()).is_err(),
            "removed command still parses: {argv:?}"
        );
    }
}

#[test]
fn test_clicommand整理_保持対象workflowとreview_commandを受理する() {
    let execution_id = "550e8400-e29b-41d4-a716-446655440000";
    for argv in [
        vec!["releash", "workflow", "status", execution_id],
        vec![
            "releash",
            "workflow",
            "output",
            "submit",
            "--node-execution",
            "550e8400-e29b-41d4-a716-446655440001",
            "--type",
            "review-verdict",
            "--json",
            r#"{"verdict":"LGTM"}"#,
        ],
        vec![
            "releash",
            "workflow",
            "output",
            "get",
            execution_id,
            "--node",
            "review",
        ],
        vec!["releash", "review", "list", "--session-id", "session-1"],
    ] {
        assert!(
            Cli::try_parse_from(argv.clone()).is_ok(),
            "supported command failed to parse: {argv:?}"
        );
    }
}

#[test]
fn test_clicommand整理_一次owner文書が実装済みsurfaceだけを列挙する() {
    let document = include_str!("../../../docs/workflow-engine-evolution-plan.md");

    for retained in [
        "releash workflow status <execution-id>",
        "releash workflow output submit --node-execution <id>",
        "releash workflow output get <execution-id>",
    ] {
        assert!(
            document.contains(retained),
            "missing CLI surface: {retained}"
        );
    }
    for removed in [
        "releash workflow list",
        "releash workflow executions",
        "releash workflow start",
        "releash workflow logs",
        "releash workflow approve",
        "releash workflow abort",
        "releash workflow stop",
        "releash workflow resume",
        "releash workflow output validate",
    ] {
        assert!(
            !document.contains(removed),
            "primary owner still advertises removed CLI surface: {removed}"
        );
    }
}

#[test]
fn test_hook専用cli_parse可能だが全helpに表示しない() {
    for provider in ["claude", "codex"] {
        assert!(
            Cli::try_parse_from(["releash", "hook", "receive", "--provider", provider,]).is_ok()
        );
    }

    let root_help = Cli::command().render_long_help().to_string();
    assert!(!root_help.contains("hook"));
    assert!(!render_long_help().contains("$ releash hook"));

    let mut command = Cli::command();
    let hook_help = command
        .find_subcommand_mut("hook")
        .expect("hidden Hook command must still parse")
        .render_long_help()
        .to_string();
    assert!(!hook_help.contains("receive"));
}

#[test]
fn test_cli長文help_複数回呼び出しでcacheを再利用する() {
    let first = render_long_help();
    let second = render_long_help();
    assert!(std::ptr::eq(first.as_ptr(), second.as_ptr()));
}

#[test]
fn test_workflow_cli_旧file_queueへ依存しない() {
    for source in [
        include_str!("mod.rs"),
        include_str!("workflow.rs"),
        include_str!("output.rs"),
        include_str!("api_client.rs"),
        include_str!("file_direct.rs"),
    ] {
        let forbidden = ["workflow_", "pending"].concat();
        assert!(!source.contains(&forbidden));
        let mutation_event = ["CliMutation", "Requested"].concat();
        assert!(!source.contains(&mutation_event));
    }
}

#[test]
fn test_file直接読取_local_api_wire_responseへ依存しない() {
    let source = include_str!("file_direct.rs");
    for forbidden in [
        "adaptor::controller::api::protocol",
        "GetArtifactResponse",
        "ValidateArtifactResponse",
    ] {
        assert!(
            !source.contains(forbidden),
            "file-direct read fallback depends on API wire type: {forbidden}"
        );
    }
}
