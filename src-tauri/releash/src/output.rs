use clap::{ArgGroup, Subcommand};
use std::path::PathBuf;
#[derive(Subcommand, Debug)]
pub(crate) enum OutputSubcommand {
    /// node の Artifact schema に従う値を提出する。
    #[command(group(
        ArgGroup::new("artifact_value")
            .args(["json", "file"])
            .multiple(false)
    ))]
    Submit {
        #[arg(long = "node-execution", value_name = "NODE_EXECUTION_ID")]
        node_execution: String,
        #[arg(long = "type", value_name = "CONTRACT", requires = "artifact_value")]
        contract: Option<String>,
        #[arg(
            long,
            conflicts_with = "file",
            requires = "contract",
            value_name = "JSON"
        )]
        json: Option<String>,
        #[arg(
            long,
            conflicts_with = "json",
            requires = "contract",
            value_name = "PATH"
        )]
        file: Option<PathBuf>,
    },
    /// 提出済み Artifact を取得する。
    Get {
        execution_id: String,
        #[arg(long, value_name = "NODE_NAME")]
        node: String,
        #[arg(long)]
        json: bool,
    },
}
