use std::path::Path;

use clap::Subcommand;

use super::api_client;
use super::common::{validate_execution_id, CliError};
use super::file_direct;
use super::output::OutputSubcommand;
use crate::adaptor::protocol::workflow::{ExecutionStatusView, WorkflowExecutionView};

/// `releash workflow` の Agent-facing command / query 集合。
#[derive(Subcommand, Debug)]
pub(super) enum WorkflowSubcommand {
    /// workflow 定義と Facet を診断する。
    #[command(long_about = "workflow 定義と Facet を診断する。\n\n\
対象 directory:\n\
  --dir <PATH> で指定した directory を workflow source directory として扱う。\n\
  Facet base は <PATH>/facets が directory ならそこを使い、無ければ <PATH> を使う。\n\
  指定 directory 配下の workflow 定義を起点に、参照される Facet までを判定範囲とする。\n\
  --dir を省略した場合は、適用済み Workflow の config directory を対象にする。\n\n\
出力形式:\n\
  既定は 1 診断 1 行の human-readable 形式で、末尾に severity 別の件数を出す。\n\
  --json を付けると、UI が受け取るものと同じ診断結果 JSON をそのまま出力する。\n\n\
終了コード:\n\
  0  severity error の診断が 0 件\n\
  3  severity error の診断が 1 件以上\n\
  1  command 自体の失敗（Releash アプリ未起動、I/O 失敗、serialize 失敗）\n\
  2  引数が不正\n\
  4  対象 directory が存在しない\n\n\
この command は Releash アプリの起動を必要とする。")]
    Diagnostics {
        /// 診断対象 directory。Facet base は facets/ があればそこを、無ければこの directory を使う。省略時は適用済み config directory。
        #[arg(long, value_name = "PATH")]
        dir: Option<std::path::PathBuf>,
        /// 診断結果 JSON をそのまま出力する。
        #[arg(long)]
        json: bool,
    },
    /// 指定 execution の現在 read model を表示する。
    Status {
        execution_id: String,
        #[arg(long)]
        json: bool,
    },
    /// node の Artifact に対する typed CLI 入口。
    Output {
        #[command(subcommand)]
        command: OutputSubcommand,
    },
}

pub(super) fn cmd_status(
    data_dir: &Path,
    execution_id: &str,
    json: bool,
) -> Result<String, CliError> {
    validate_execution_id(execution_id)?;
    let execution = api_client::read_with_fallback(
        data_dir,
        |client| client.execution_status(execution_id),
        || file_direct::execution_status(data_dir, execution_id),
    )?;
    format_execution_status(execution, json)
}

fn format_execution_status(
    execution: WorkflowExecutionView,
    json: bool,
) -> Result<String, CliError> {
    if json {
        let text = serde_json::to_string_pretty(&execution)
            .map_err(|error| CliError::Other(format!("serialize execution: {error}")))?;
        return Ok(format!("{text}\n"));
    }
    let current_node = execution.current_node.as_deref().unwrap_or("");
    Ok(format!(
        "execution_id:  {}\nworkflow:      {}\nstatus:        {}\ncurrent_node:  {}\nupdated_at:    {}\ninput_tokens:  {}\noutput_tokens: {}\n",
        execution.id,
        execution.workflow_name,
        execution_status_name(execution.status),
        current_node,
        execution.updated_at,
        execution.total_token_usage.input_tokens,
        execution.total_token_usage.output_tokens,
    ))
}

fn execution_status_name(status: ExecutionStatusView) -> &'static str {
    match status {
        ExecutionStatusView::Running => "running",
        ExecutionStatusView::WaitingApproval => "waiting_approval",
        ExecutionStatusView::Interrupted => "interrupted",
        ExecutionStatusView::Completed => "completed",
        ExecutionStatusView::Aborted => "aborted",
    }
}

#[cfg(test)]
#[path = "workflow_test.rs"]
mod workflow_tests;
