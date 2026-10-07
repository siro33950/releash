mod client;
mod commands;
pub mod json;
mod output;
mod review;
mod workflow;

use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "releash", disable_help_subcommand = true)]
struct Cli {
    #[arg(long, global = true)]
    data_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: TopCommand,
}

#[derive(Subcommand, Debug)]
enum TopCommand {
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

fn data_dir(explicit: Option<PathBuf>) -> Result<PathBuf, connectrpc::ConnectError> {
    explicit
        .or_else(|| {
            std::env::var("RELEASH_DATA_DIR")
                .ok()
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
        })
        .map(Ok)
        .unwrap_or_else(|| {
            releash_client::data_dir::default_data_dir_for_profile(
                releash_client::data_dir::BuildProfile::current(),
            )
            .ok_or_else(|| {
                connectrpc::ConnectError::unavailable("OS data directory is unavailable")
            })
        })
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
    if let TopCommand::Completion { shell } = cli.command {
        clap_complete::generate(
            shell,
            &mut Cli::command(),
            "releash",
            &mut std::io::stdout(),
        );
        return;
    }
    let json = commands::json_output(&cli.command);
    let hook = matches!(cli.command, TopCommand::Hook { .. });
    let result = data_dir(cli.data_dir).and_then(|dir| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| connectrpc::ConnectError::internal(error.to_string()))?
            .block_on(commands::run(&dir, cli.command))
    });
    let code = match result {
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
