//! [05] / [06] `releash workflow ...` CLI 入口。
//!
//! read-only 観測経路は engine と IPC せず、`workflow_runs/` 配下と `workflows/`
//! YAML / builtin を file-direct で読む（spec [05] アーキテクチャ概要）。
//! mutating CLI (`approve` / `reject` / `abort`) も engine と直接 IPC せず、pending
//! command file を enqueue するところまでを CLI の責務に閉じる（spec [06] CLI 完了
//! 基準境界）。

use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};

use crate::agent_status::current_timestamp;
use crate::config::read_config_if_exists;
use crate::protocol::WorkflowStateView;
use crate::workflow::command_input::{
    validate_optional_comment_text, validate_reject_reason_text, CommandInputError,
};
use crate::workflow::event::WorkflowEvent;
use crate::workflow::event_projection::reconstruct_state_from_events;
use crate::workflow::log::WorkflowEventLog;
use crate::workflow::pending_command::{CliRequestPayload, PendingCommand, PendingCommandStore};
use crate::workflow::run::{
    iter_valid_run_metadata, project_runs_to_summaries, running_workflow_names_from_metadata,
    RunListFilter, RunStatusFilter, WorkflowRunSummary,
};
use crate::workflow::storage;
use crate::workflow::worktree::canonicalize_managed_worktree_path_inner;
use crate::workflow_state_presenter::workflow_state_to_view;

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
}

/// CLI の workflow サブコマンド集合。
///
/// engine domain の `workflow::command::WorkflowCommand`（state mutating typed
/// command）と語彙衝突しないよう CLI 側は `WorkflowSubcommand` として分離する
/// （spec [05] read-only と mutating の分離 / observation source-of-truth の境界）。
#[derive(Subcommand, Debug)]
enum WorkflowSubcommand {
    /// 利用可能な workflow template 一覧を表示する。
    List {
        #[arg(long)]
        json: bool,
    },
    /// 過去 / 進行中の workflow run 一覧を表示する。
    Runs {
        /// status filter: `active` または `terminal`。省略時は両方。
        #[arg(long)]
        status: Option<String>,
        /// worktree_path filter。
        #[arg(long)]
        worktree: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// 指定 run の現在 state を表示する。
    Status {
        run_id: String,
        #[arg(long)]
        json: bool,
    },
    /// 指定 run の event log を表示する。
    Logs {
        run_id: String,
        #[arg(long)]
        json: bool,
    },
    /// [06] approval node を承認する。CLI は pending command を file 仲介で
    /// 書き出すまでで完了し、engine への到達は稼働中アプリの watcher が担う
    /// （spec [06] CLI 完了基準境界）。
    Approve {
        run_id: String,
        /// 対象 node を限定する。省略時は engine が現在の承認待ち node を解決する。
        #[arg(long)]
        node: Option<String>,
        /// 任意の承認コメント。`ApprovalResolved.comment` に伝播する。
        #[arg(long)]
        comment: Option<String>,
    },
    /// [06] approval node を却下する。`--reason` 必須。
    Reject {
        run_id: String,
        #[arg(long)]
        node: Option<String>,
        /// 却下理由（必須）。`WorkflowEvent` に平文で永続化される。
        #[arg(long)]
        reason: String,
    },
    /// [06] 進行中の workflow run を中止する。`--node` 指定時は当該 node に対する
    /// abort、未指定時は run 全体への abort として engine が処理する。
    Abort {
        run_id: String,
        #[arg(long)]
        node: Option<String>,
    },
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
    let workflows_dir = storage::workflows_dir();
    // data_dir は List / Runs / Status / Logs それぞれの branch 内で解決する。
    // List も workflow_runs/ 由来の `is_running` 反映のため data_dir を必要とするが、
    // 各 branch 内で解決することで未到達の branch では I/O を走らせない。
    let resolve = || -> Result<PathBuf, CliError> { resolve_data_dir().map_err(CliError::Other) };
    let result = match cli.command {
        TopCommand::Workflow { command } => match command {
            WorkflowSubcommand::List { json } => match resolve() {
                Ok(data_dir) => cmd_list(&workflows_dir, &data_dir, json),
                Err(e) => Err(e),
            },
            WorkflowSubcommand::Runs {
                status,
                worktree,
                json,
            } => match resolve() {
                Ok(data_dir) => cmd_runs(&data_dir, status, worktree, json),
                Err(e) => Err(e),
            },
            WorkflowSubcommand::Status { run_id, json } => match resolve() {
                Ok(data_dir) => cmd_status(&data_dir, &run_id, json),
                Err(e) => Err(e),
            },
            WorkflowSubcommand::Logs { run_id, json } => match resolve() {
                Ok(data_dir) => cmd_logs(&data_dir, &run_id, json),
                Err(e) => Err(e),
            },
            WorkflowSubcommand::Approve {
                run_id,
                node,
                comment,
            } => match resolve() {
                Ok(data_dir) => {
                    match validate_optional_cli_text_len(comment.as_deref(), "--comment") {
                        Ok(()) => cmd_enqueue_pending(
                            &data_dir,
                            &run_id,
                            CliRequestPayload::Approve {
                                node_name: node,
                                comment,
                            },
                        ),
                        Err(e) => Err(e),
                    }
                }
                Err(e) => Err(e),
            },
            WorkflowSubcommand::Reject {
                run_id,
                node,
                reason,
            } => match resolve() {
                Ok(data_dir) => match validate_reject_reason(&reason) {
                    Ok(()) => cmd_enqueue_pending(
                        &data_dir,
                        &run_id,
                        CliRequestPayload::Reject {
                            node_name: node,
                            reason,
                        },
                    ),
                    Err(e) => Err(e),
                },
                Err(e) => Err(e),
            },
            WorkflowSubcommand::Abort { run_id, node } => match resolve() {
                Ok(data_dir) => cmd_enqueue_pending(
                    &data_dir,
                    &run_id,
                    CliRequestPayload::Abort { node_name: node },
                ),
                Err(e) => Err(e),
            },
        },
    };
    match result {
        Ok(()) => 0,
        Err(CliError::NotFound(msg)) => {
            eprintln!("{msg}");
            4
        }
        Err(CliError::InvalidInput(msg)) => {
            eprintln!("error: {msg}");
            2
        }
        Err(CliError::Other(msg)) => {
            eprintln!("error: {msg}");
            1
        }
    }
}

#[derive(Debug)]
enum CliError {
    /// run / template が見つからない（spec Rule: 存在しない run_id は明示的に「該当 run なし」）。
    NotFound(String),
    /// 入力フォーマット不正（不正な run_id、不正な status filter 値など）。
    InvalidInput(String),
    /// その他の I/O / serialization エラー。
    Other(String),
}

impl From<String> for CliError {
    fn from(msg: String) -> Self {
        CliError::Other(msg)
    }
}

/// データディレクトリの解決。
///
/// Tauri 側 `AppHandle::path().app_data_dir()` と同等のパスを CLI 側で計算する。
/// CLI 起動独立性境界: デスクトップアプリ非稼働でも動作する。
fn resolve_data_dir() -> Result<PathBuf, String> {
    if let Ok(custom) = std::env::var("RELEASH_DATA_DIR") {
        return Ok(PathBuf::from(custom));
    }
    let base = dirs::data_dir().ok_or_else(|| "Cannot resolve OS data_dir".to_string())?;
    Ok(base.join("com.releash.app"))
}

/// workflow template 一覧。
///
/// `is_running` は `workflow_runs/` 配下の active metadata から導出した
/// workflow_name set を反映する（spec [05] Rule: 観測経路は API と CLI で
/// 等価な手段を提供する）。
fn cmd_list(workflows_dir: &Path, data_dir: &Path, json: bool) -> Result<(), CliError> {
    let summaries = list_workflows_file_direct(workflows_dir, data_dir)?;
    if json {
        let text = serde_json::to_string_pretty(&summaries)
            .map_err(|e| format!("serialize workflows: {e}"))?;
        println!("{text}");
    } else {
        if summaries.is_empty() {
            println!("(no workflows)");
            return Ok(());
        }
        for s in &summaries {
            let tag = if s.builtin { "[builtin]" } else { "         " };
            let running_marker = if s.is_running { " (running)" } else { "" };
            println!("{tag} {:<32}  {}{running_marker}", s.name, s.description);
        }
    }
    Ok(())
}

/// workflow run 一覧。
fn cmd_runs(
    data_dir: &Path,
    status: Option<String>,
    worktree: Option<String>,
    json: bool,
) -> Result<(), CliError> {
    let status_filter = match status.as_deref() {
        None | Some("") => None,
        Some("active") => Some(RunStatusFilter::Active),
        Some("terminal") => Some(RunStatusFilter::Terminal),
        Some(other) => {
            return Err(CliError::InvalidInput(format!(
                "Invalid --status value: {other} (expected: active | terminal)"
            )));
        }
    };
    // [05] API / CLI 等価性境界: Tauri API 経路と同じ
    // `canonicalize_managed_worktree_path_inner` を経由する。
    // 相対パス / symlink で同一 worktree を指定した場合に API / CLI の観測結果が
    // 分岐しないようにするだけでなく、managed worktree かどうかの検証も同じ
    // ルートで実施する（spec L92-96 API / CLI の意味的等価性境界）。
    let worktree_path = match worktree.as_deref() {
        Some(path) => Some(canonicalize_cli_worktree_filter_path(data_dir, path)?),
        None => None,
    };
    let filter = RunListFilter {
        status: status_filter,
        worktree_path,
    };
    let summaries = list_runs_file_direct(data_dir, filter);
    if json {
        let text =
            serde_json::to_string_pretty(&summaries).map_err(|e| format!("serialize runs: {e}"))?;
        println!("{text}");
    } else {
        if summaries.is_empty() {
            println!("(no runs)");
            return Ok(());
        }
        println!(
            "{:<36}  {:<20}  {:<18}  WORKTREE",
            "RUN_ID", "WORKFLOW", "STATUS"
        );
        for s in &summaries {
            let status = format!("{:?}", s.status);
            println!(
                "{:<36}  {:<20}  {:<18}  {}",
                s.run_id,
                truncate(&s.workflow_name, 20),
                status,
                s.worktree_path
            );
        }
    }
    Ok(())
}

/// 指定 run の現在 state。
fn cmd_status(data_dir: &Path, run_id: &str, json: bool) -> Result<(), CliError> {
    validate_run_id(run_id)?;
    if get_run_summary_file_direct(data_dir, run_id).is_none() {
        return Err(CliError::NotFound(format!(
            "Workflow run not found: {run_id}"
        )));
    }
    let view = reconstruct_state_view(data_dir, run_id)?;
    if json {
        let text =
            serde_json::to_string_pretty(&view).map_err(|e| format!("serialize state: {e}"))?;
        println!("{text}");
    } else {
        println!(
            "run_id:        {}\nworkflow:      {}\nstate:         {:?}\ncurrent_step:  {}\nupdated_at:    {}",
            view.state.execution_id,
            view.state.workflow_name,
            view.state.state,
            view.state.current_step_name,
            view.state.updated_at,
        );
    }
    Ok(())
}

/// [06] CLI mutating 経路の pending command 投入。
///
/// spec [06] Rule:「CLI の完了基準は『受理キュー投入』までで統一する」に従い、
/// 本関数は pending command file の atomic 書き出しが完了した時点で `Ok(())`
/// を返す。engine への到達 / 認可結果は CLI 側で待たない（spec [06] CLI 完了
/// 基準境界）。
fn cmd_enqueue_pending(
    data_dir: &Path,
    run_id: &str,
    payload: CliRequestPayload,
) -> Result<(), CliError> {
    let output = enqueue_pending_command(data_dir, run_id, payload)?;
    println!("{}", output.format_stdout_line());
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingEnqueueOutput {
    run_id: String,
    request_id: String,
    path: String,
}

impl PendingEnqueueOutput {
    fn format_stdout_line(&self) -> String {
        format!(
            "queued: run_id={} request_id={} ({})",
            self.run_id, self.request_id, self.path
        )
    }
}

fn enqueue_pending_command(
    data_dir: &Path,
    run_id: &str,
    payload: CliRequestPayload,
) -> Result<PendingEnqueueOutput, CliError> {
    validate_run_id(run_id)?;
    let store = PendingCommandStore::new(data_dir);
    let command = PendingCommand::new(run_id.to_string(), payload, current_timestamp());
    let path = store
        .write_pending(&command)
        .map_err(|e| CliError::Other(format!("Failed to enqueue pending command: {e}")))?;
    Ok(PendingEnqueueOutput {
        run_id: command.run_id,
        request_id: command.id,
        path: path.display().to_string(),
    })
}

/// `--reason` 必須化境界（spec [06] 振る舞い定義 Rule: 却下要求には却下理由が伴う）。
/// `clap` で `--reason` を必須化済みだが、空白のみの入力は CLI 入口で拒否する。
///
/// 文字数上限 / 空白判定はドメイン pure helper（`command_input::validate_reject_reason_text`）
/// に集約し、CLI 層は `CliError::InvalidInput` への map に閉じる（review R2-01）。
fn validate_reject_reason(reason: &str) -> Result<(), CliError> {
    validate_reject_reason_text(reason, "--reason").map_err(command_input_error_to_cli_error)
}

/// 任意の自由記述テキスト（例: `--comment`）の長さを CLI 入口で検証する。
///
/// 文字数上限はドメイン pure helper に集約（review R2-01）。
fn validate_optional_cli_text_len(
    value: Option<&str>,
    label: &'static str,
) -> Result<(), CliError> {
    validate_optional_comment_text(value, label).map_err(command_input_error_to_cli_error)
}

fn command_input_error_to_cli_error(err: CommandInputError) -> CliError {
    CliError::InvalidInput(err.to_string())
}

/// 指定 run の event log。
fn cmd_logs(data_dir: &Path, run_id: &str, json: bool) -> Result<(), CliError> {
    validate_run_id(run_id)?;
    if get_run_summary_file_direct(data_dir, run_id).is_none() {
        return Err(CliError::NotFound(format!(
            "Workflow run not found: {run_id}"
        )));
    }
    let events = read_log(data_dir, run_id)?;
    if json {
        let text =
            serde_json::to_string_pretty(&events).map_err(|e| format!("serialize log: {e}"))?;
        println!("{text}");
    } else {
        for event in &events {
            println!("{}", format_event(event));
        }
    }
    Ok(())
}

/// [05] CLI: `workflows/` + builtin と `workflow_runs/` の active metadata を file-direct
/// で読み、`is_running` を反映した `Summary` 一覧を返す（spec [05] Rule: 観測経路は
/// API と CLI で等価な手段を提供する）。
///
/// API 側（`commands::list_workflows`）は engine in-memory active map を running source として
/// 使うが、CLI は file metadata を running source として使う。両者は file 同期境界
/// （[04] atomic mutation 境界）の中で揃うので、観測結果は等価となる。
fn list_workflows_file_direct(
    workflows_dir: &Path,
    data_dir: &Path,
) -> Result<Vec<crate::workflow::schema::Summary>, CliError> {
    let mut summaries = storage::list_workflows(workflows_dir).map_err(|e| e.to_string())?;
    let running = running_workflow_names_from_metadata(data_dir);
    for s in &mut summaries {
        s.is_running = running.contains(&s.name);
    }
    Ok(summaries)
}

/// `workflow_runs/` を file-direct で走査し、filter を適用した summary 一覧を返す。
/// API 経路（`RunStore::list_runs`）と同じ `project_runs_to_summaries` を経由することで
/// 観測ロジックの divergence を防ぐ（spec [05] API / CLI の意味的等価性境界）。
fn list_runs_file_direct(data_dir: &Path, filter: RunListFilter) -> Vec<WorkflowRunSummary> {
    let runs = iter_valid_run_metadata(data_dir);
    project_runs_to_summaries(runs, &filter)
}

fn get_run_summary_file_direct(data_dir: &Path, run_id: &str) -> Option<WorkflowRunSummary> {
    if uuid::Uuid::parse_str(run_id).is_err() {
        return None;
    }
    iter_valid_run_metadata(data_dir)
        .iter()
        .find(|run| run.run_id == run_id)
        .map(WorkflowRunSummary::from)
}

/// [05] API / CLI 等価性境界: Tauri 側 `canonicalize_managed_worktree_path`
/// と同じ `canonicalize_managed_worktree_path_inner` を CLI 経路でも経由する。
/// data_dir 配下の `releash.toml` から `last_repo_paths` / `last_root_path` を読み出して
/// repo_paths を組み立て、managed worktree かどうかを Tauri API と同一ロジックで検証する。
///
/// 観測専用 CLI helper として書き込みを発生させない `read_config_if_exists` を使う
/// （spec [05] read-only と mutating の分離）。typed error:
///
/// - `CliError::Other`: config read / parse 失敗（設定読込側の問題）。
/// - `CliError::InvalidInput`: managed worktree でない入力（caller の問題）。
///
/// 両者を同じ `InvalidInput` に潰すと caller が原因を取り違えるため明確に分ける
/// （spec [05] CLI 入力の信頼境界 + read-only と mutating の分離）。
fn canonicalize_cli_worktree_filter_path(
    data_dir: &Path,
    worktree_path: &str,
) -> Result<String, CliError> {
    let config_path = data_dir.join("releash.toml");
    let repo_paths = match read_config_if_exists(&config_path).map_err(CliError::Other)? {
        Some(config) => {
            let mut paths = config.app.last_repo_paths.clone();
            if !config.app.last_root_path.is_empty() && !paths.contains(&config.app.last_root_path)
            {
                paths.push(config.app.last_root_path.clone());
            }
            paths
        }
        None => Vec::new(),
    };
    canonicalize_managed_worktree_path_inner(repo_paths, worktree_path.to_string())
        .map_err(CliError::InvalidInput)
}

fn validate_run_id(run_id: &str) -> Result<(), CliError> {
    uuid::Uuid::parse_str(run_id)
        .map(|_| ())
        .map_err(|_| CliError::InvalidInput("Invalid run_id format (must be UUID)".to_string()))
}

fn read_log(data_dir: &Path, run_id: &str) -> Result<Vec<WorkflowEvent>, CliError> {
    let event_log = WorkflowEventLog::new(data_dir);
    event_log.read_log(run_id).map_err(CliError::Other)
}

/// Spec [05] API / CLI 等価性境界: Tauri `get_workflow_run_state` と同じ
/// `WorkflowStateView` shape を CLI 側でも返すため、再構築した `WorkflowState` を
/// `workflow_state_to_view` 経由で投影し、`runtime_states` 空 HashMap で
/// `WorkflowStateView::from_parts` に通す（CLI は engine の in-memory runtime を
/// 観測しない）。
fn reconstruct_state_view(data_dir: &Path, run_id: &str) -> Result<WorkflowStateView, CliError> {
    let events = read_log(data_dir, run_id)?;
    let state = reconstruct_state_from_events(run_id, &events).map_err(CliError::Other)?;
    let state = state
        .ok_or_else(|| CliError::NotFound(format!("No event log available for run: {run_id}")))?;
    Ok(WorkflowStateView::from_parts(
        workflow_state_to_view(state),
        std::collections::HashMap::new(),
    ))
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{truncated}…")
    }
}

fn format_event(event: &WorkflowEvent) -> String {
    let kind = match event {
        WorkflowEvent::RunStarted { .. } => "RunStarted",
        WorkflowEvent::NodeStarted { .. } => "NodeStarted",
        WorkflowEvent::NodeCompleted { .. } => "NodeCompleted",
        WorkflowEvent::NodeFailed { .. } => "NodeFailed",
        WorkflowEvent::ApprovalRequested { .. } => "ApprovalRequested",
        WorkflowEvent::ApprovalResolved { .. } => "ApprovalResolved",
        WorkflowEvent::RunCompleted { .. } => "RunCompleted",
        WorkflowEvent::RunFailed { .. } => "RunFailed",
        WorkflowEvent::RunAborted { .. } => "RunAborted",
        WorkflowEvent::OutputCollected { .. } => "OutputCollected",
        WorkflowEvent::ParallelStarted { .. } => "ParallelStarted",
        WorkflowEvent::ParallelChildStarted { .. } => "ParallelChildStarted",
        WorkflowEvent::ParallelChildCompleted { .. } => "ParallelChildCompleted",
        WorkflowEvent::ParallelCompleted { .. } => "ParallelCompleted",
        WorkflowEvent::ContractRepairRequested { .. } => "ContractRepairRequested",
        WorkflowEvent::CliMutationRequested { .. } => "CliMutationRequested",
        WorkflowEvent::OutputSubmitted { .. } => "OutputSubmitted",
    };
    match serde_json::to_string(event) {
        Ok(json) => format!("{kind} {json}"),
        Err(_) => kind.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::command_input::MAX_APPROVAL_COMMENT_CHARS;
    use crate::workflow::run::{RunStatus, TriggerSource, WorkflowRun};
    use std::fs;
    use tempfile::TempDir;

    fn make_run(run_id: &str, worktree: &str, status: RunStatus, started_at: f64) -> WorkflowRun {
        WorkflowRun {
            run_id: run_id.to_string(),
            workflow_name: "wf".to_string(),
            task: None,
            status,
            worktree_path: worktree.to_string(),
            current_node_name: None,
            trigger_source: TriggerSource::Cli,
            started_at,
            updated_at: started_at,
            completed_at: if status.is_terminal() {
                Some(started_at + 1.0)
            } else {
                None
            },
            error_reason: None,
        }
    }

    fn write_run_file(data_dir: &Path, run: &WorkflowRun) {
        let runs_dir = data_dir.join("workflow_runs");
        fs::create_dir_all(&runs_dir).unwrap();
        let path = runs_dir.join(format!("{}.json", run.run_id));
        let json = serde_json::to_string_pretty(run).unwrap();
        fs::write(path, json).unwrap();
    }

    fn test_uuid(seed: u8) -> String {
        uuid::Uuid::from_bytes([seed; 16]).to_string()
    }

    /// Rule: 観測対象として存在しない run_id は明示的に「該当 run なし」として扱われる
    #[test]
    fn get_run_summary_file_direct_returns_none_for_missing_run() {
        let tmp = TempDir::new().unwrap();
        let result = get_run_summary_file_direct(tmp.path(), &test_uuid(99));
        assert!(result.is_none());
    }

    #[test]
    fn get_run_summary_file_direct_rejects_invalid_run_id() {
        let tmp = TempDir::new().unwrap();
        let result = get_run_summary_file_direct(tmp.path(), "not-a-uuid");
        assert!(result.is_none());
    }

    /// Rule: CLI 経路で active / terminal を含む run 一覧を観測できる
    #[test]
    fn list_runs_file_direct_returns_active_and_terminal_runs() {
        let tmp = TempDir::new().unwrap();
        let active_id = test_uuid(1);
        let done_id = test_uuid(2);
        write_run_file(
            tmp.path(),
            &make_run(&active_id, "/wt/a", RunStatus::Running, 100.0),
        );
        write_run_file(
            tmp.path(),
            &make_run(&done_id, "/wt/b", RunStatus::Completed, 90.0),
        );

        let all = list_runs_file_direct(tmp.path(), RunListFilter::default());
        assert_eq!(all.len(), 2);
        // active が先頭
        assert_eq!(all[0].run_id, active_id);
        assert_eq!(all[1].run_id, done_id);

        let active_only = list_runs_file_direct(
            tmp.path(),
            RunListFilter {
                status: Some(RunStatusFilter::Active),
                worktree_path: None,
            },
        );
        assert_eq!(active_only.len(), 1);
        assert_eq!(active_only[0].run_id, active_id);

        let terminal_only = list_runs_file_direct(
            tmp.path(),
            RunListFilter {
                status: Some(RunStatusFilter::Terminal),
                worktree_path: None,
            },
        );
        assert_eq!(terminal_only.len(), 1);
        assert_eq!(terminal_only[0].run_id, done_id);
    }

    /// Rule: worktree filter で対応する run のみが返る
    #[test]
    fn list_runs_file_direct_filters_by_worktree() {
        let tmp = TempDir::new().unwrap();
        let run_a = test_uuid(1);
        let run_b = test_uuid(2);
        write_run_file(
            tmp.path(),
            &make_run(&run_a, "/wt/a", RunStatus::Running, 100.0),
        );
        write_run_file(
            tmp.path(),
            &make_run(&run_b, "/wt/b", RunStatus::Running, 100.0),
        );

        let filtered = list_runs_file_direct(
            tmp.path(),
            RunListFilter {
                status: None,
                worktree_path: Some("/wt/a".to_string()),
            },
        );
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].run_id, run_a);
    }

    /// Rule: 観測される情報は engine が一次 owner として保持するデータと一致する
    #[test]
    fn get_run_summary_file_direct_matches_persisted_metadata() {
        let tmp = TempDir::new().unwrap();
        let run_id = test_uuid(1);
        let run = make_run(&run_id, "/wt/a", RunStatus::Running, 100.0);
        write_run_file(tmp.path(), &run);

        let summary = get_run_summary_file_direct(tmp.path(), &run_id).unwrap();
        assert_eq!(summary.run_id, run_id);
        assert_eq!(summary.worktree_path, "/wt/a");
        assert_eq!(summary.status, RunStatus::Running);
        assert_eq!(summary.started_at, 100.0);
    }

    #[test]
    fn validate_run_id_rejects_non_uuid() {
        assert!(validate_run_id("not-a-uuid").is_err());
        assert!(validate_run_id("").is_err());
        assert!(validate_run_id("../etc/passwd").is_err());
    }

    #[test]
    fn validate_run_id_accepts_valid_uuid() {
        assert!(validate_run_id("550e8400-e29b-41d4-a716-446655440000").is_ok());
    }

    fn write_event_log(data_dir: &Path, _run_id: &str, events: &[WorkflowEvent]) {
        let log = WorkflowEventLog::new(data_dir);
        for event in events {
            log.append(event).unwrap();
        }
    }

    fn run_started_event(run_id: &str, workflow_name: &str, worktree: &str) -> WorkflowEvent {
        WorkflowEvent::RunStarted {
            run_id: run_id.to_string(),
            workflow_name: workflow_name.to_string(),
            workflow_file_stem: workflow_name.to_string(),
            worktree_path: worktree.to_string(),
            workflow_definition: crate::workflow::schema::Workflow {
                name: workflow_name.to_string(),
                description: "test".to_string(),
                builtin: false,
                nodes: vec![],
            },
            timestamp: 100.0,
        }
    }

    /// Spec [05] Rule: CLI 経路から指定 run の現在 state を観測できる。
    #[test]
    fn cmd_status_returns_ok_for_existing_run_with_event_log() {
        let tmp = TempDir::new().unwrap();
        let run_id = test_uuid(11);
        write_run_file(
            tmp.path(),
            &make_run(&run_id, "/wt/cli-status", RunStatus::Running, 100.0),
        );
        write_event_log(
            tmp.path(),
            &run_id,
            &[run_started_event(&run_id, "wf", "/wt/cli-status")],
        );
        let result = cmd_status(tmp.path(), &run_id, true);
        assert!(result.is_ok(), "cmd_status must succeed: {result:?}");
    }

    /// Spec [05] Rule: 存在しない run_id は明示的に「該当 run なし」として扱われる（CLI 経路）。
    #[test]
    fn cmd_status_returns_not_found_for_missing_run() {
        let tmp = TempDir::new().unwrap();
        let result = cmd_status(tmp.path(), &test_uuid(99), false);
        assert!(matches!(result, Err(CliError::NotFound(_))));
    }

    #[test]
    fn cmd_status_returns_invalid_input_for_non_uuid_run_id() {
        let tmp = TempDir::new().unwrap();
        let result = cmd_status(tmp.path(), "not-a-uuid", false);
        assert!(matches!(result, Err(CliError::InvalidInput(_))));
    }

    /// Spec [05] Rule: CLI 経路から指定 run の event log を観測できる。
    #[test]
    fn cmd_logs_returns_ok_for_existing_run() {
        let tmp = TempDir::new().unwrap();
        let run_id = test_uuid(12);
        write_run_file(
            tmp.path(),
            &make_run(&run_id, "/wt/cli-logs", RunStatus::Completed, 200.0),
        );
        write_event_log(
            tmp.path(),
            &run_id,
            &[run_started_event(&run_id, "wf", "/wt/cli-logs")],
        );
        let result = cmd_logs(tmp.path(), &run_id, true);
        assert!(result.is_ok(), "cmd_logs must succeed: {result:?}");
    }

    #[test]
    fn cmd_logs_returns_not_found_for_missing_run() {
        let tmp = TempDir::new().unwrap();
        let result = cmd_logs(tmp.path(), &test_uuid(98), false);
        assert!(matches!(result, Err(CliError::NotFound(_))));
    }

    #[test]
    fn cmd_logs_returns_invalid_input_for_non_uuid_run_id() {
        let tmp = TempDir::new().unwrap();
        let result = cmd_logs(tmp.path(), "../etc/passwd", false);
        assert!(matches!(result, Err(CliError::InvalidInput(_))));
    }

    /// Spec [05] Rule: 観測経路は API と CLI で等価な手段を提供する。
    /// 同一 tempdir 上の metadata + NDJSON を入力に、API 側（RunStore + event_log +
    /// projection）と CLI 側（list_runs_file_direct / get_run_summary_file_direct /
    /// read_log / reconstruct_state_view）が同じ観測結果を返すことを直接検証する。
    #[tokio::test]
    async fn api_and_cli_observation_paths_produce_equivalent_results() {
        use crate::workflow::event_projection::reconstruct_state_from_events;
        use crate::workflow::run::RunStore;

        let tmp = TempDir::new().unwrap();
        let run_id = test_uuid(21);
        let other_id = test_uuid(22);
        write_run_file(
            tmp.path(),
            &make_run(&run_id, "/wt/api-cli", RunStatus::Completed, 300.0),
        );
        write_run_file(
            tmp.path(),
            &make_run(&other_id, "/wt/api-cli-2", RunStatus::Completed, 250.0),
        );
        write_event_log(
            tmp.path(),
            &run_id,
            &[run_started_event(&run_id, "wf", "/wt/api-cli")],
        );

        // CLI 経路
        let cli_summaries = list_runs_file_direct(tmp.path(), RunListFilter::default());
        let cli_summary = get_run_summary_file_direct(tmp.path(), &run_id)
            .expect("CLI summary must be available");
        let cli_events = read_log(tmp.path(), &run_id).unwrap();
        let cli_state_view = reconstruct_state_view(tmp.path(), &run_id).unwrap();

        // API 経路（RunStore は active in-memory map + workflow_runs/ file の両方を参照）
        let store = RunStore::default();
        store.set_data_dir(tmp.path().to_path_buf()).await;
        let api_summaries = store.list_runs(RunListFilter::default()).await;
        let api_summary = store
            .get_run(&run_id)
            .await
            .expect("API summary must be available");
        let api_events = WorkflowEventLog::new(tmp.path()).read_log(&run_id).unwrap();
        let api_state = reconstruct_state_from_events(&run_id, &api_events)
            .unwrap()
            .unwrap();
        let api_state_view = WorkflowStateView::from_parts(
            workflow_state_to_view(api_state),
            std::collections::HashMap::new(),
        );

        // 並び順 + 件数の一致
        assert_eq!(api_summaries.len(), cli_summaries.len());
        for (a, c) in api_summaries.iter().zip(cli_summaries.iter()) {
            assert_eq!(a.run_id, c.run_id);
            assert_eq!(a.status, c.status);
            assert_eq!(a.worktree_path, c.worktree_path);
        }
        // 単一 summary の一致
        assert_eq!(api_summary.run_id, cli_summary.run_id);
        assert_eq!(api_summary.status, cli_summary.status);
        assert_eq!(api_summary.worktree_path, cli_summary.worktree_path);
        // event log の件数 / 種別の一致
        assert_eq!(api_events.len(), cli_events.len());
        assert_eq!(api_events.len(), 1);
        // state view の serialize 等価性: API/CLI は同じ shape の WorkflowStateView を返す。
        let api_json = serde_json::to_value(&api_state_view).unwrap();
        let cli_json = serde_json::to_value(&cli_state_view).unwrap();
        assert_eq!(api_json, cli_json);
    }

    /// Spec [05] Rule: 観測経路は API と CLI で等価な手段を提供する（workflow template 経路）。
    /// CLI の `list_workflows_file_direct` は `workflow_runs/` 配下の active metadata から
    /// `is_running` を反映する。API 側（`engine.running_workflow_names()` + `storage::list_workflows`）
    /// と同じ workflow template 集合 + `is_running` フラグを返すことを直接検証する。
    #[tokio::test]
    async fn list_workflows_file_direct_reflects_running_active_metadata() {
        use crate::workflow::storage;

        let tmp = TempDir::new().unwrap();
        let workflows_dir = tmp.path().join("workflows");
        std::fs::create_dir_all(&workflows_dir).unwrap();
        let yaml = concat!(
            "name: api-cli-list\n",
            "description: list test\n",
            "nodes:\n",
            "  - name: step1\n",
            "    type: agent\n",
            "    instruction: do thing\n",
            "    permission: edit\n",
        );
        std::fs::write(workflows_dir.join("api-cli-list.yml"), yaml).unwrap();

        // 該当 workflow を active 状態で書き込む。
        let active_id = test_uuid(50);
        write_run_file(
            tmp.path(),
            &WorkflowRun {
                run_id: active_id.clone(),
                workflow_name: "api-cli-list".to_string(),
                task: None,
                status: RunStatus::Running,
                worktree_path: "/wt/list".to_string(),
                current_node_name: None,
                trigger_source: TriggerSource::Cli,
                started_at: 500.0,
                updated_at: 500.0,
                completed_at: None,
                error_reason: None,
            },
        );

        let cli_summaries = list_workflows_file_direct(&workflows_dir, tmp.path()).unwrap();
        let target = cli_summaries
            .iter()
            .find(|s| s.name == "api-cli-list")
            .expect("workflow must be listed");
        assert!(target.is_running, "is_running must reflect active metadata");

        // API 側と等価性: storage::list_workflows + (active metadata 由来の running set)。
        let mut api_summaries = storage::list_workflows(&workflows_dir).unwrap();
        let api_running = running_workflow_names_from_metadata(tmp.path());
        for s in &mut api_summaries {
            s.is_running = api_running.contains(&s.name);
        }
        // 同じ projection に通したので JSON shape も一致する。
        let api_json = serde_json::to_value(&api_summaries).unwrap();
        let cli_json = serde_json::to_value(&cli_summaries).unwrap();
        assert_eq!(api_json, cli_json);
    }

    /// Spec [05] API / CLI 等価性境界: list_workflows の API 経路は
    /// `engine.running_workflow_names()`（in-memory `executions` map 由来）を
    /// running source とし、CLI は `running_workflow_names_from_metadata`
    /// （`workflow_runs/` file 由来）を使う。engine が active run を登録すると
    /// 両 source が同期して同一 running 集合を返すことを実 API 経路で検証する
    /// （spec L92-96 / L160-162）。
    #[tokio::test]
    async fn engine_running_workflow_names_matches_cli_file_direct_after_register_active() {
        use crate::workflow::engine::WorkflowEngine;
        use crate::workflow::schema::{NodeDefinition, NodeType, Workflow};
        use crate::workflow::state::WorkflowExecutionState;

        let tmp = TempDir::new().unwrap();
        let engine = WorkflowEngine::new_for_test();
        engine
            .set_run_store_data_dir(tmp.path().to_path_buf())
            .await;

        let workflow = Workflow {
            name: "engine-cli-list".to_string(),
            description: String::new(),
            builtin: false,
            nodes: vec![NodeDefinition {
                name: "step1".to_string(),
                node_type: NodeType::Agent,
                ..Default::default()
            }],
        };
        let run_id = test_uuid(60);
        engine
            .seed_active_execution_for_test(
                run_id.clone(),
                workflow,
                WorkflowExecutionState::Running,
                "/wt/engine-cli-list".to_string(),
                TriggerSource::Cli,
            )
            .await;

        let api_running = engine.running_workflow_names().await;
        let cli_running = running_workflow_names_from_metadata(tmp.path());
        assert_eq!(
            api_running, cli_running,
            "API path engine.running_workflow_names() must equal CLI file-direct set"
        );
        assert!(api_running.contains("engine-cli-list"));
    }

    /// Spec [05] API / CLI 等価性境界: CLI `--worktree` 入力は
    /// `canonicalize_managed_worktree_path_inner` 経由で managed worktree 検証を
    /// 通過する。configured repo path に紐づかない非 canonical 入力は
    /// CLI 側で InvalidInput として弾かれる（API 側と同一エラー経路）。
    #[test]
    fn cli_worktree_filter_rejects_non_managed_path() {
        let tmp = TempDir::new().unwrap();
        // releash.toml に repo 設定なし → どの worktree も managed として認識されない。
        let outside = tempfile::TempDir::new().unwrap();
        let result =
            canonicalize_cli_worktree_filter_path(tmp.path(), &outside.path().to_string_lossy());
        assert!(
            result.is_err(),
            "non-managed worktree path must be rejected"
        );
    }

    /// Spec [05] API / CLI 等価性境界: configured repo path 配下の managed worktree
    /// を CLI 経路で指定した場合、相対 / 末尾スラッシュ等の非 canonical 入力でも
    /// 正規化済みの絶対パス文字列として projection helper に渡せる。
    #[test]
    fn cli_worktree_filter_accepts_managed_worktree_with_non_canonical_input() {
        let (repo_dir, repo) = crate::git::test_helpers::create_test_repo();
        crate::git::test_helpers::create_initial_commit(&repo);
        let worktree_parent = tempfile::TempDir::new().unwrap();
        let worktree_path = worktree_parent.path().join("managed-wt");
        repo.worktree("managed-wt", &worktree_path, None).unwrap();

        // CLI 用 data_dir に releash.toml を配置し、repo を configured repo として登録。
        let data_dir = tempfile::TempDir::new().unwrap();
        let config_path = data_dir.path().join("releash.toml");
        let mut config = crate::config::ReleashConfig::default();
        config.app.last_repo_paths = vec![repo_dir.path().to_string_lossy().to_string()];
        crate::config::write_config(&config_path, &config).unwrap();

        // 末尾スラッシュ / `.` を含む非 canonical 入力で呼び出しても、canonicalize した
        // 絶対パスが返り、その値は worktree_path.canonicalize() と一致する。
        let non_canonical = worktree_path.join(".").to_string_lossy().to_string();
        let normalized =
            canonicalize_cli_worktree_filter_path(data_dir.path(), &non_canonical).unwrap();
        assert_eq!(
            std::path::PathBuf::from(normalized),
            worktree_path.canonicalize().unwrap()
        );
    }

    /// Spec [05] API / CLI 等価性境界: `list_workflows` Tauri command が委譲する実
    /// inner 関数（`workflow::commands::list_workflows_inner`）と CLI 側の
    /// `list_workflows_file_direct` を、同一 tempdir / 同一 running 集合に対して比較し、
    /// 両者が JSON shape として完全に一致することを境界仕様として担保する。
    ///
    /// engine.running_workflow_names()（in-memory）と
    /// running_workflow_names_from_metadata()（file-direct）は engine の同期書き込み
    /// 境界により等価。本テストでは CLI 側と同じ source（file-direct）で running 集合を
    /// 構築して inner に渡し、両経路の出力 shape が等価であることを検証する。
    #[test]
    fn list_workflows_inner_api_path_equals_cli_file_direct_path() {
        let tmp = TempDir::new().unwrap();
        let workflows_dir = tmp.path().join("workflows");
        std::fs::create_dir_all(&workflows_dir).unwrap();
        let yaml = concat!(
            "name: api-cli-inner\n",
            "description: api-cli-inner test\n",
            "nodes:\n",
            "  - name: step1\n",
            "    type: agent\n",
            "    instruction: do thing\n",
            "    permission: edit\n",
        );
        std::fs::write(workflows_dir.join("api-cli-inner.yml"), yaml).unwrap();

        // 該当 workflow を active 状態で書き込む。
        let active_id = test_uuid(70);
        write_run_file(
            tmp.path(),
            &WorkflowRun {
                run_id: active_id.clone(),
                workflow_name: "api-cli-inner".to_string(),
                task: None,
                status: RunStatus::Running,
                worktree_path: "/wt/inner".to_string(),
                current_node_name: None,
                trigger_source: TriggerSource::Cli,
                started_at: 700.0,
                updated_at: 700.0,
                completed_at: None,
                error_reason: None,
            },
        );

        // API 経路: list_workflows_inner（Tauri command が委譲する実関数）
        let running = running_workflow_names_from_metadata(tmp.path());
        let api_summaries =
            crate::workflow::commands::list_workflows_inner(&running, &workflows_dir).unwrap();

        // CLI 経路: list_workflows_file_direct
        let cli_summaries = list_workflows_file_direct(&workflows_dir, tmp.path()).unwrap();

        // 両者は同じ projection を通すので JSON shape も完全一致。
        let api_json = serde_json::to_value(&api_summaries).unwrap();
        let cli_json = serde_json::to_value(&cli_summaries).unwrap();
        assert_eq!(
            api_json, cli_json,
            "list_workflows_inner と list_workflows_file_direct は同一観測結果を返さなければならない"
        );

        // is_running が active metadata から正しく反映されている。
        let api_target = api_summaries
            .iter()
            .find(|s| s.name == "api-cli-inner")
            .expect("workflow must be present in API path");
        assert!(api_target.is_running);
    }

    /// Spec [05] read-only と mutating の分離: 観測専用 CLI helper は config 不在時に
    /// `releash.toml` を作成しない。`load_or_create_config` の hidden write 経路を
    /// CLI から廃止したことを境界仕様として担保する。
    #[test]
    fn cli_worktree_filter_does_not_create_config_when_missing() {
        let data_dir = tempfile::TempDir::new().unwrap();
        let config_path = data_dir.path().join("releash.toml");
        let outside = tempfile::TempDir::new().unwrap();
        // 設定ファイル不在で呼び出す。managed worktree でないため Err になる。
        let _ = canonicalize_cli_worktree_filter_path(
            data_dir.path(),
            &outside.path().to_string_lossy(),
        );
        assert!(
            !config_path.exists(),
            "CLI must not create releash.toml in read-only paths"
        );
    }

    /// Spec [05] CLI 入力の信頼境界: 設定 parse エラーは「managed worktree でない」
    /// 入力エラーに化けず、原因付きで Err として伝播する。
    #[test]
    fn cli_worktree_filter_propagates_config_parse_error() {
        let data_dir = tempfile::TempDir::new().unwrap();
        let config_path = data_dir.path().join("releash.toml");
        std::fs::write(&config_path, b"this = is = invalid = toml").unwrap();
        let outside = tempfile::TempDir::new().unwrap();
        let err = canonicalize_cli_worktree_filter_path(
            data_dir.path(),
            &outside.path().to_string_lossy(),
        )
        .expect_err("parse error must propagate");
        // typed error: config parse failure は CliError::Other で伝播し、
        // 「managed worktree でない」入力エラー（InvalidInput）と取り違えない。
        let CliError::Other(msg) = &err else {
            panic!("expected CliError::Other for config parse failure, got: {err:?}");
        };
        assert!(
            msg.contains("パース失敗") || msg.to_lowercase().contains("parse"),
            "error must indicate config parse failure, got: {msg}"
        );
    }

    /// [05] CLI 公開入口の parse 境界: `releash workflow list|runs|status|logs` の
    /// 4 サブコマンドが clap で parse できることを parser-level で担保する。本テストは
    /// I/O を発生させない（`try_parse_from` のみ）。
    #[test]
    fn cli_workflow_subcommands_parse_via_clap() {
        for argv in [
            vec!["releash", "workflow", "list"],
            vec!["releash", "workflow", "list", "--json"],
            vec!["releash", "workflow", "runs"],
            vec!["releash", "workflow", "runs", "--status", "active"],
            vec![
                "releash", "workflow", "runs", "--status", "terminal", "--json",
            ],
            vec![
                "releash",
                "workflow",
                "runs",
                "--worktree",
                "/some/path",
                "--json",
            ],
            vec![
                "releash",
                "workflow",
                "status",
                "550e8400-e29b-41d4-a716-446655440000",
            ],
            vec![
                "releash",
                "workflow",
                "logs",
                "550e8400-e29b-41d4-a716-446655440000",
                "--json",
            ],
        ] {
            assert!(
                Cli::try_parse_from(&argv).is_ok(),
                "parser must accept {argv:?}"
            );
        }
    }

    /// [06] scope の境界: 本 issue では user decision 系 3 command
    /// （approve / reject / abort）の CLI 入口のみを公開する。新規 run 起動
    /// （`run`）／ structured output 提出（`output` / `submit`）は別 issue に
    /// 切り出し済みのため、parser 段階で reject される境界を担保する。
    #[test]
    fn cli_does_not_expose_out_of_scope_subcommands() {
        for argv in [
            vec!["releash", "workflow", "run"],
            vec!["releash", "workflow", "output"],
            vec!["releash", "workflow", "submit"],
        ] {
            assert!(
                Cli::try_parse_from(&argv).is_err(),
                "parser must reject out-of-scope subcommand: {argv:?}"
            );
        }
    }

    /// [06] CLI 公開入口の parse 境界: `releash workflow {approve,reject,abort}`
    /// の typed subcommand が clap で parse できることを parser-level で担保する
    /// （I/O は発生させない）。`reject` の `--reason` は必須。
    #[test]
    fn cli_mutating_subcommands_parse_via_clap() {
        let run_id = "550e8400-e29b-41d4-a716-446655440000";
        for argv in [
            vec!["releash", "workflow", "approve", run_id],
            vec!["releash", "workflow", "approve", run_id, "--node", "review"],
            vec![
                "releash",
                "workflow",
                "approve",
                run_id,
                "--comment",
                "LGTM",
            ],
            vec!["releash", "workflow", "reject", run_id, "--reason", "no"],
            vec![
                "releash", "workflow", "reject", run_id, "--node", "review", "--reason", "no",
            ],
            vec!["releash", "workflow", "abort", run_id],
            vec!["releash", "workflow", "abort", run_id, "--node", "review"],
        ] {
            assert!(
                Cli::try_parse_from(&argv).is_ok(),
                "parser must accept mutating subcommand: {argv:?}"
            );
        }
    }

    /// [06] 振る舞い定義 Rule: 却下要求には却下理由が伴う。
    /// CLI 入口で `--reason` が省略された reject は parser 段階で reject される。
    #[test]
    fn cli_reject_requires_reason_argument() {
        let run_id = "550e8400-e29b-41d4-a716-446655440000";
        let argv = vec!["releash", "workflow", "reject", run_id];
        assert!(
            Cli::try_parse_from(&argv).is_err(),
            "reject without --reason must be rejected by parser"
        );
    }

    /// [06] 振る舞い定義 Rule: 却下要求には却下理由が伴う。空白のみの reason は
    /// CLI 入口で InvalidInput として弾く（engine 側 validate_approval_decision
    /// にも同じ境界がある）。
    #[test]
    fn validate_reject_reason_rejects_whitespace_only() {
        assert!(validate_reject_reason("   ").is_err());
        assert!(validate_reject_reason("").is_err());
        assert!(validate_reject_reason("not empty").is_ok());
    }

    #[test]
    fn cli_mutation_free_text_rejects_oversized_values() {
        let oversized = "x".repeat(MAX_APPROVAL_COMMENT_CHARS + 1);
        assert!(validate_reject_reason(&oversized).is_err());
        assert!(validate_optional_cli_text_len(Some(&oversized), "--comment").is_err());
        assert!(validate_optional_cli_text_len(Some("ok"), "--comment").is_ok());
    }

    /// [06] CLI 完了基準境界: `cmd_enqueue_pending` は受理キュー投入
    /// （pending file の書き出し）まで完了した時点で `Ok(())` を返す。engine への
    /// 到達結果は CLI 側で待たない（spec [06] CLI 完了基準境界）。書き出された
    /// pending file は `PendingCommandStore` 経由で取り出せる。
    #[test]
    fn cmd_enqueue_pending_writes_pending_file_for_approve() {
        let tmp = TempDir::new().unwrap();
        let run_id = test_uuid(81);
        let payload = CliRequestPayload::Approve {
            node_name: Some("review".to_string()),
            comment: Some("LGTM".to_string()),
        };
        let output = enqueue_pending_command(tmp.path(), &run_id, payload.clone()).unwrap();
        let stdout = output.format_stdout_line();
        assert!(stdout.starts_with(&format!("queued: run_id={run_id} request_id=")));
        assert!(stdout.contains("/workflow_pending/pending/"));
        let entries = PendingCommandStore::new(tmp.path()).list_pending().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].command.run_id, run_id);
        assert_eq!(entries[0].command.payload, payload);
    }

    #[test]
    fn cmd_enqueue_pending_writes_pending_file_for_reject_with_reason_and_node() {
        let tmp = TempDir::new().unwrap();
        let run_id = test_uuid(82);
        let payload = CliRequestPayload::Reject {
            node_name: Some("review".to_string()),
            reason: "needs changes".to_string(),
        };

        let output = enqueue_pending_command(tmp.path(), &run_id, payload.clone()).unwrap();

        assert_eq!(output.run_id, run_id);
        let entries = PendingCommandStore::new(tmp.path()).list_pending().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].command.payload, payload);
    }

    #[test]
    fn cmd_enqueue_pending_writes_pending_file_for_abort_with_node() {
        let tmp = TempDir::new().unwrap();
        let run_id = test_uuid(83);
        let payload = CliRequestPayload::Abort {
            node_name: Some("review".to_string()),
        };

        let output = enqueue_pending_command(tmp.path(), &run_id, payload.clone()).unwrap();

        assert_eq!(output.run_id, run_id);
        let entries = PendingCommandStore::new(tmp.path()).list_pending().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].command.payload, payload);
    }

    /// [06] CLI 入力の信頼境界: `cmd_enqueue_pending` は run_id の UUID 形式を弾く。
    #[test]
    fn cmd_enqueue_pending_rejects_non_uuid_run_id() {
        let tmp = TempDir::new().unwrap();
        let payload = CliRequestPayload::Abort { node_name: None };
        let err = enqueue_pending_command(tmp.path(), "not-a-uuid", payload).unwrap_err();
        assert!(matches!(err, CliError::InvalidInput(_)));
    }

    /// [05] CLI 入力の信頼境界: managed worktree でない入力は
    /// `CliError::InvalidInput` として伝播し、`CliError::Other`（config 読込側）と
    /// 混同しない。typed error を直接 match して境界を担保する。
    #[test]
    fn cli_worktree_filter_returns_invalid_input_for_non_managed_path() {
        let data_dir = tempfile::TempDir::new().unwrap();
        let outside = tempfile::TempDir::new().unwrap();
        let err = canonicalize_cli_worktree_filter_path(
            data_dir.path(),
            &outside.path().to_string_lossy(),
        )
        .expect_err("non-managed path must be rejected");
        assert!(
            matches!(err, CliError::InvalidInput(_)),
            "non-managed input must surface as InvalidInput, got: {err:?}"
        );
    }
}
