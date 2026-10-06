use crate::cli::Cli;
use clap::Parser;

#[test]
fn test_workflow_output_cli_現在のcommandをparseする() {
    let execution_id = "550e8400-e29b-41d4-a716-446655440000";
    for argv in [
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
    ] {
        assert!(Cli::try_parse_from(argv).is_ok());
    }
}

#[test]
fn test_workflow_output_get_未知optionを拒否する() {
    let execution_id = "550e8400-e29b-41d4-a716-446655440000";

    for argv in [
        vec![
            "releash",
            "workflow",
            "output",
            "get",
            execution_id,
            "--node",
            "review",
            "--future-flag",
        ],
        vec![
            "releash",
            "workflow",
            "output",
            "get",
            execution_id,
            "--node",
            "review",
            "--future-option",
            "ignored",
        ],
        vec![
            "releash",
            "workflow",
            "output",
            "get",
            execution_id,
            "--node",
            "review",
            "--future-option=ignored",
        ],
    ] {
        let error = Cli::try_parse_from(argv.clone()).unwrap_err();
        assert_eq!(
            error.kind(),
            clap::error::ErrorKind::UnknownArgument,
            "unknown option must be rejected: {argv:?}"
        );
    }
}

#[test]
fn test_workflow_output_submit_requires_attempt_identity_and_accepts_optional_artifact() {
    let node_execution_id = "550e8400-e29b-41d4-a716-446655440001";

    assert!(Cli::try_parse_from([
        "releash",
        "workflow",
        "output",
        "submit",
        "--node-execution",
        node_execution_id,
    ])
    .is_ok());
    assert!(Cli::try_parse_from([
        "releash",
        "workflow",
        "output",
        "submit",
        "--node-execution",
        node_execution_id,
        "--type",
        "review-verdict",
    ])
    .is_err());
    assert!(Cli::try_parse_from(["releash", "workflow", "output", "submit"]).is_err());
}

use super::cmd_output_submit;
use crate::cli::CliError;

#[test]
fn test_workflow_output_submit_rejects_blank_node_execution_id() {
    assert_eq!(
        cmd_output_submit(
            std::path::Path::new("/unused"),
            "   ".to_string(),
            None,
            None,
            None
        )
        .unwrap_err(),
        CliError::InvalidInput("--node-execution must not be empty".to_string())
    );
}
