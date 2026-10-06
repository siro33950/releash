use crate::cli::Cli;
use clap::Parser;

#[test]
fn test_workflow_status_cli_正規語彙でparseできる() {
    let execution_id = "550e8400-e29b-41d4-a716-446655440000";
    assert!(Cli::try_parse_from(["releash", "workflow", "status", execution_id, "--json"]).is_ok());
}
