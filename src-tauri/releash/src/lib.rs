mod client;
mod commands;
pub mod json;
mod output;
mod review;
mod server;
mod workflow;

use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "releash", disable_help_subcommand = true)]
struct Cli {
    #[arg(long, global = true)]
    data_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<TopCommand>,
}

#[derive(Subcommand, Debug)]
enum TopCommand {
    Status {
        #[arg(long)]
        json: bool,
    },
    Server {
        #[command(subcommand)]
        command: server::ServerSubcommand,
    },
    Workflow {
        #[command(subcommand)]
        command: workflow::WorkflowSubcommand,
    },
    Review {
        #[command(subcommand)]
        command: review::ReviewSubcommand,
    },
    #[command(hide = true)]
    Hook {
        #[command(subcommand)]
        command: HookCommand,
    },
    Completion {
        shell: clap_complete::Shell,
    },
}

#[derive(Subcommand, Debug)]
enum HookCommand {
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

pub fn run() {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            let args: Vec<_> = std::env::args().collect();
            let machine = args.iter().any(|arg| arg == "--json")
                && !args.windows(2).any(|args| args == ["output", "submit"]);
            if machine && error.use_stderr() {
                eprintln!(
                    "{}",
                    serde_json::json!({"error":{"code":"invalid_argument","message":error.to_string()}})
                );
            } else if let Err(error) = error.print() {
                eprintln!("{error}");
            }
            std::process::exit(error.exit_code());
        }
    };
    if let Some(TopCommand::Completion { shell }) = cli.command {
        clap_complete::generate(
            shell,
            &mut Cli::command(),
            "releash",
            &mut std::io::stdout(),
        );
        return;
    }
    let json = cli.command.as_ref().is_some_and(commands::json_output);
    let hook = matches!(cli.command, Some(TopCommand::Hook { .. }));
    let result = releash_sdk::data_dir::resolve_data_dir(cli.data_dir)
        .map_err(connectrpc::ConnectError::unavailable)
        .and_then(|dir| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| connectrpc::ConnectError::internal(error.to_string()))?
                .block_on(async {
                    match cli.command {
                        Some(command) => commands::run(&dir, command).await,
                        None => server::launch(&dir).await.map(|output| (output, 0)),
                    }
                })
        });
    let code = match result.map_err(client::error_guidance) {
        Ok((output, code)) => {
            print!("{output}");
            code
        }
        Err(error) => {
            if json {
                eprintln!(
                    "{}",
                    serde_json::json!({"error": {"code": error.code.as_str(), "message": error.message.clone().unwrap_or_else(|| error.to_string())}})
                );
            } else {
                eprintln!(
                    "error: {}: {}",
                    error.code.as_str(),
                    error.message.as_deref().unwrap_or("Request failed")
                );
            }
            1
        }
    };
    if hook {
        print!("{{}}");
    }
    std::process::exit(if hook { 0 } else { code });
}
