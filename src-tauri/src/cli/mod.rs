//! `releash workflow ...` CLI 入口。
//!
//! workflow command/query は localhost local API を正とする。アプリ未起動時は
//! read-only query だけ backend-owned read model の file-direct fallback を許可する。

mod api_client;
mod common;
mod file_direct;
mod output;
mod review;
mod workflow;

use std::sync::OnceLock;

use clap::{CommandFactory, Parser, Subcommand};

use common::{cli_result_exit_code, resolve_data_dir, resolve_existing_data_dir, CliError};
use output::OutputSubcommand;
use review::ReviewSubcommand;
use workflow::WorkflowSubcommand;

use crate::adaptor::gateway::workflow::WorkflowDefinitionFileRepository;

/// `releash` CLI のトップ args。
#[derive(Parser, Debug)]
#[command(name = "releash", disable_help_subcommand = true)]
struct Cli {
    #[command(subcommand)]
    command: TopCommand,
}

#[derive(Subcommand, Debug)]
enum TopCommand {
    /// workflow command / query サブコマンド。
    Workflow {
        #[command(subcommand)]
        command: WorkflowSubcommand,
    },
    /// Agent review comment サブコマンド。
    Review {
        #[command(subcommand)]
        command: ReviewSubcommand,
    },
}

/// Agent system prompt に追加する、nested command を含む canonical CLI help。
pub fn render_long_help() -> &'static str {
    static CACHE: OnceLock<String> = OnceLock::new();
    CACHE.get_or_init(|| {
        let command = Cli::command();
        let mut output = String::new();
        render_command_help_tree(&command, &mut Vec::new(), &mut output);
        output
    })
}

fn render_command_help_tree(
    command: &clap::Command,
    parents: &mut Vec<String>,
    output: &mut String,
) {
    parents.push(command.get_name().to_string());
    if !output.is_empty() {
        output.push('\n');
    }
    output.push_str("$ ");
    output.push_str(&parents.join(" "));
    output.push_str(" --help\n");
    let mut rendered = command.clone();
    output.push_str(&rendered.render_long_help().to_string());
    for child in command.get_subcommands() {
        render_command_help_tree(child, parents, output);
    }
    parents.pop();
}

/// CLI のエントリーポイント。`std::process::exit` 用の終了コードを返す。
pub fn run() -> i32 {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            error.print().ok();
            return error.exit_code();
        }
    };
    let workflows_dir = WorkflowDefinitionFileRepository::default_workflows_dir();
    let result =
        match cli.command {
            TopCommand::Workflow { command } => resolve_data_dir()
                .map_err(CliError::Other)
                .and_then(|data_dir| match command {
                    WorkflowSubcommand::List { json } => {
                        workflow::cmd_list(&workflows_dir, &data_dir, json)
                    }
                    WorkflowSubcommand::Start {
                        workflow_name,
                        request,
                        worktree,
                        permission,
                    } => {
                        workflow::cmd_start(&data_dir, workflow_name, request, worktree, permission)
                    }
                    WorkflowSubcommand::Executions {
                        status,
                        worktree,
                        json,
                    } => workflow::cmd_executions(&data_dir, status, worktree, json),
                    WorkflowSubcommand::Status { execution_id, json } => {
                        workflow::cmd_status(&data_dir, &execution_id, json)
                    }
                    WorkflowSubcommand::Logs { execution_id, json } => {
                        workflow::cmd_logs(&data_dir, &execution_id, json)
                    }
                    WorkflowSubcommand::Approve {
                        execution_id,
                        node,
                        node_execution,
                        comment,
                    } => workflow::cmd_approve(
                        &data_dir,
                        &execution_id,
                        node,
                        node_execution,
                        comment,
                    ),
                    WorkflowSubcommand::Abort { execution_id } => {
                        workflow::cmd_abort(&data_dir, &execution_id)
                    }
                    WorkflowSubcommand::Output { command } => match command {
                        OutputSubcommand::Submit {
                            execution_id,
                            node,
                            node_execution,
                            contract,
                            json,
                            file,
                        } => output::cmd_output_submit(
                            &data_dir,
                            &execution_id,
                            &node,
                            node_execution,
                            &contract,
                            json,
                            file,
                        ),
                        OutputSubcommand::Validate {
                            execution_id,
                            node,
                            contract,
                            file,
                        } => output::cmd_output_validate(
                            &data_dir,
                            &execution_id,
                            &node,
                            &contract,
                            &file,
                        ),
                        OutputSubcommand::Get {
                            execution_id,
                            node,
                            json,
                        } => output::cmd_output_get(&data_dir, &execution_id, &node, json),
                    },
                }),
            TopCommand::Review { command } => resolve_existing_data_dir()
                .and_then(|data_dir| review::cmd_review(&data_dir, command)),
        };
    cli_result_exit_code(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_long_help_contains_nested_canonical_workflow_commands() {
        let help = render_long_help();
        for command in [
            "$ releash workflow --help",
            "$ releash workflow start --help",
            "$ releash workflow executions --help",
            "$ releash workflow output submit --help",
            "$ releash workflow output validate --help",
            "$ releash workflow output get --help",
            "--node",
            "--node-execution",
            "--type",
            "EXECUTION_ID",
        ] {
            assert!(help.contains(command), "missing nested help: {command}");
        }
        for legacy in [
            ["ru", "ns"].concat(),
            ["run", "_id"].concat(),
            ["--st", "ep"].concat(),
            ["Re", "ject"].concat(),
        ] {
            assert!(!help.contains(&legacy), "legacy CLI vocabulary: {legacy}");
        }
    }

    #[test]
    fn render_long_help_is_cached_across_calls() {
        let first = render_long_help();
        let second = render_long_help();
        assert!(std::ptr::eq(first.as_ptr(), second.as_ptr()));
    }

    #[test]
    fn workflow_cli_has_no_legacy_file_queue_dependency() {
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
    fn file_direct_reads_do_not_depend_on_local_api_wire_responses() {
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
}
