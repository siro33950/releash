//! `releash workflow ...` CLI 入口。
//!
//! workflow command/query は localhost local API を正とする。アプリ未起動時は
//! read-only query だけ backend-owned read model の file-direct fallback を許可する。

mod api_client;
mod common;
mod diagnostics;
mod file_direct;
mod hook;
mod output;
mod review;
#[cfg(test)]
mod test_helpers;
mod workflow;

use std::sync::OnceLock;

use clap::{CommandFactory, Parser, Subcommand, ValueEnum};

use common::{
    cli_result_exit_code, resolve_data_dir, resolve_existing_data_dir, CliError, CliSuccess,
};
use output::OutputSubcommand;
use review::ReviewSubcommand;
use workflow::WorkflowSubcommand;

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
    /// Provider Hook 専用入口。
    #[command(hide = true)]
    Hook {
        #[command(subcommand)]
        command: HookSubcommand,
    },
}

#[derive(Subcommand, Debug)]
enum HookSubcommand {
    /// Provider lifecycle signal を受信する。
    #[command(hide = true)]
    Receive {
        #[arg(long, value_enum)]
        provider: HookProvider,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum HookProvider {
    Claude,
    Codex,
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
        if child.is_hide_set() {
            continue;
        }
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
    if let Ok(data_dir) = resolve_data_dir() {
        if let Err(error) = crate::infrastructure::local_log::init(
            &data_dir,
            crate::infrastructure::local_log::LocalLogProcess::Cli,
        ) {
            eprintln!("{error}");
        }
    }
    let result = match cli.command {
        TopCommand::Workflow { command } => {
            resolve_data_dir()
                .map_err(CliError::Other)
                .and_then(|data_dir| match command {
                    WorkflowSubcommand::Diagnostics { dir, json } => {
                        diagnostics::cmd_diagnostics(&data_dir, dir, json)
                    }
                    WorkflowSubcommand::Status { execution_id, json } => {
                        workflow::cmd_status(&data_dir, &execution_id, json).map(CliSuccess::ok)
                    }
                    WorkflowSubcommand::Output { command } => match command {
                        OutputSubcommand::Submit {
                            node_execution,
                            contract,
                            json,
                            file,
                        } => output::cmd_output_submit(
                            &data_dir,
                            node_execution,
                            contract.as_deref(),
                            json,
                            file,
                        )
                        .map(CliSuccess::ok),
                        OutputSubcommand::Get {
                            execution_id,
                            node,
                            json,
                        } => output::cmd_output_get(&data_dir, &execution_id, &node, json)
                            .map(CliSuccess::ok),
                    },
                })
        }
        TopCommand::Review { command } => resolve_existing_data_dir()
            .and_then(|data_dir| review::cmd_review(&data_dir, command).map(CliSuccess::ok)),
        TopCommand::Hook { command } => match command {
            HookSubcommand::Receive { provider } => hook::cmd_receive(provider).map(CliSuccess::ok),
        },
    };
    let exit_code = cli_result_exit_code(result);
    log::logger().flush();
    exit_code
}

#[cfg(test)]
#[path = "cli_test.rs"]
mod cli_tests;
