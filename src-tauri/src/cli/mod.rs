//! [05] / [06] `releash workflow ...` CLI 入口。
//!
//! read-only 観測経路は engine と IPC せず、`workflow_runs/` 配下と `workflows/`
//! YAML / builtin を file-direct で読む（spec [05] design）。
//! mutating CLI (`approve` / `abort`) も engine と直接 IPC せず、pending
//! command file を enqueue するところまでを CLI の責務に閉じる（spec [06] CLI 完了
//! 基準境界）。

mod common;
mod output;
mod review;
mod workflow;
mod workflow_io;

use std::sync::OnceLock;

use clap::{CommandFactory, Parser, Subcommand};

use common::{cli_result_exit_code, resolve_existing_data_dir};
use output::OutputSubcommand;
use review::ReviewSubcommand;
use workflow::WorkflowSubcommand;

use crate::adaptor::gateway::workflow::WorkflowDefinitionFileRepository;

/// `releash` CLI のトップ args。
///
/// clap AST は外部公開せず、エントリーポイントは `cli::run()` に限定する
/// （spec [05] scope の境界 + 内部 AST の非公開境界）。
#[derive(Parser, Debug)]
#[command(name = "releash", disable_help_subcommand = true)]
struct Cli {
    #[command(subcommand)]
    command: TopCommand,
}

#[derive(Subcommand, Debug)]
enum TopCommand {
    /// workflow 観測サブコマンド。
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

/// Releash CLI の long help 文字列を返す（OnceLock キャッシュ）。
///
/// Agent backend 起動時に system_prompt へ append される単一ソース。
/// clap derive 由来の help を使うため、CLI 定義の追加・変更に自動追従する。
/// （Issue #1022 / spec [09]: Agent process environment contract / Agent backend orchestration 責務）
pub fn render_long_help() -> &'static str {
    static CACHE: OnceLock<String> = OnceLock::new();
    CACHE.get_or_init(|| Cli::command().render_long_help().to_string())
}

/// CLI のエントリーポイント。`std::process::exit` 用の終了コードを返す。
pub fn run() -> i32 {
    let cli = match Cli::try_parse() {
        Ok(c) => c,
        Err(e) => {
            // clap の help / version は exit_code() でハンドリング。
            e.print().ok();
            return e.exit_code();
        }
    };
    let workflows_dir = WorkflowDefinitionFileRepository::default_workflows_dir();
    // data_dir は List / Runs / Status / Logs それぞれの branch 内で解決する。
    // List も workflow_runs/ 由来の `is_running` 反映のため data_dir を必要とするが、
    // 各 branch 内で解決することで未到達の branch では I/O を走らせない。
    // [05] 観測経路境界: data_dir 自体が存在しない場合は `NotFound` として扱い、
    // 「run が 0 件」と「向き先がそもそも無い」を区別する（5-1 修正）。
    let resolve = || resolve_existing_data_dir();
    let result =
        match cli.command {
            TopCommand::Workflow { command } => match command {
                WorkflowSubcommand::List { json } => resolve()
                    .and_then(|data_dir| workflow::cmd_list(&workflows_dir, &data_dir, json)),
                WorkflowSubcommand::Runs {
                    status,
                    worktree,
                    json,
                } => resolve()
                    .and_then(|data_dir| workflow::cmd_runs(&data_dir, status, worktree, json)),
                WorkflowSubcommand::Status { run_id, json } => {
                    resolve().and_then(|data_dir| workflow::cmd_status(&data_dir, &run_id, json))
                }
                WorkflowSubcommand::Logs { run_id, json } => {
                    resolve().and_then(|data_dir| workflow::cmd_logs(&data_dir, &run_id, json))
                }
                WorkflowSubcommand::Approve {
                    run_id,
                    node,
                    comment,
                } => resolve()
                    .and_then(|data_dir| workflow::cmd_approve(&data_dir, &run_id, node, comment)),
                WorkflowSubcommand::Abort { run_id, node } => {
                    resolve().and_then(|data_dir| workflow::cmd_abort(&data_dir, &run_id, node))
                }
                WorkflowSubcommand::Output { command } => {
                    resolve().and_then(|data_dir| match command {
                        OutputSubcommand::Submit {
                            run_id,
                            step,
                            contract,
                            json,
                            file,
                        } => output::cmd_output_submit(
                            &data_dir, &run_id, &step, &contract, json, file,
                        ),
                        OutputSubcommand::Validate { run_id, step, file } => {
                            output::cmd_output_validate(&data_dir, &run_id, &step, &file)
                        }
                        OutputSubcommand::Get { run_id, step, json } => {
                            output::cmd_output_get(&data_dir, &run_id, &step, json)
                        }
                    })
                }
            },
            TopCommand::Review { command } => {
                resolve().and_then(|data_dir| review::cmd_review(&data_dir, command))
            }
        };
    cli_result_exit_code(result)
}

#[cfg(test)]
mod tests {
    /// Issue #1022: Agent process environment contract により、Releash CLI の
    /// long help が system_prompt 注入用の単一ソースとして取得可能でなければならない。
    /// 分割前 main の実出力と 1 文字単位で一致することで、clap 由来の
    /// subcommand 説明 / value_name / 順序 / doc comment 文言の観測不変性を担保する。
    #[test]
    fn render_long_help_matches_split_before_golden() {
        let expected = "`releash` CLI のトップ args。\n\nclap AST は外部公開せず、エントリーポイントは `cli::run()` に限定する （spec [05] scope の境界 + 内部 AST の非公開境界）。\n\nUsage: releash <COMMAND>\n\nCommands:\n  workflow  workflow 観測サブコマンド。\n  review    Agent review comment サブコマンド。\n\nOptions:\n  -h, --help\n          Print help (see a summary with '-h')\n";

        assert_eq!(super::render_long_help(), expected);
    }

    /// OnceLock キャッシュにより、再呼び出しでも同じ参照を返すこと。
    #[test]
    fn render_long_help_is_cached_across_calls() {
        let first = super::render_long_help();
        let second = super::render_long_help();
        assert!(
            std::ptr::eq(first.as_ptr(), second.as_ptr()),
            "render_long_help must return the same cached string instance across calls"
        );
    }

    #[test]
    fn output_module_does_not_depend_on_workflow_module() {
        let output_src = include_str!("output.rs");
        let forbidden_lines = workflow_module_dependency_lines(output_src);
        assert!(
            forbidden_lines.is_empty(),
            "output.rs must use workflow_io for shared workflow CLI helpers instead of importing workflow.rs; forbidden references: {forbidden_lines:?}"
        );
    }

    #[test]
    fn workflow_module_dependency_detector_preserves_workflow_io_boundary() {
        for src in [
            "use crate::cli::workflow::WorkflowSubcommand;",
            "use super::workflow;",
            "let _ = super::super::workflow::cmd_logs;",
        ] {
            assert!(
                line_has_workflow_module_dependency(src),
                "detector must reject workflow.rs dependency in: {src}"
            );
        }
        for src in [
            "use super::workflow_io;",
            "let _ = super::workflow_io::read_domain_log;",
            "use crate::cli::workflow_io::PendingEnqueueOutput;",
        ] {
            assert!(
                !line_has_workflow_module_dependency(src),
                "detector must allow workflow_io helper dependency in: {src}"
            );
        }
    }

    fn workflow_module_dependency_lines(source: &str) -> Vec<(usize, String)> {
        source
            .lines()
            .enumerate()
            .filter(|(_, line)| line_has_workflow_module_dependency(line))
            .map(|(index, line)| (index + 1, line.trim().to_string()))
            .collect()
    }

    fn line_has_workflow_module_dependency(line: &str) -> bool {
        contains_path_with_module_boundary(line, "crate::cli::workflow")
            || contains_path_with_module_boundary(line, "super::workflow")
    }

    fn contains_path_with_module_boundary(line: &str, needle: &str) -> bool {
        let mut offset = 0;
        while let Some(index) = line[offset..].find(needle) {
            let end = offset + index + needle.len();
            if line[end..]
                .chars()
                .next()
                .is_none_or(|ch| !is_rust_ident_continue(ch))
            {
                return true;
            }
            offset = end;
        }
        false
    }

    fn is_rust_ident_continue(ch: char) -> bool {
        ch == '_' || ch.is_ascii_alphanumeric()
    }
}
