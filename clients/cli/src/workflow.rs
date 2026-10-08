use crate::output::OutputSubcommand;
use clap::Subcommand;
#[derive(Subcommand, Debug)]
pub(super) enum WorkflowSubcommand {
    /// workflow 定義と Facet を診断する。
    #[command(after_long_help = "出力形式: 既定は人向け、--json は診断結果 JSON。
終了コード:
  0  severity error の項目がない
  3  severity error の項目がある
  1  command 自体の失敗
  2  引数が不正")]
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
