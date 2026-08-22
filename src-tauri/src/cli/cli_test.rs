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
fn test_cli_parse境界_未知入力と現在の引数errorを拒否する() {
    assert!(Cli::try_parse_from(["releash", "workflow", "future-command"]).is_err());

    let unknown_option = Cli::try_parse_from([
        "releash",
        "workflow",
        "status",
        "550e8400-e29b-41d4-a716-446655440000",
        "--future-option",
        "ignored",
    ])
    .unwrap_err();
    assert_eq!(
        unknown_option.kind(),
        clap::error::ErrorKind::UnknownArgument
    );

    let missing_required =
        Cli::try_parse_from(["releash", "workflow", "output", "submit"]).unwrap_err();
    assert_eq!(
        missing_required.kind(),
        clap::error::ErrorKind::MissingRequiredArgument
    );

    let invalid_value = Cli::try_parse_from([
        "releash",
        "hook",
        "receive",
        "--provider",
        "future-provider",
    ])
    .unwrap_err();
    assert_eq!(invalid_value.kind(), clap::error::ErrorKind::InvalidValue);
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
