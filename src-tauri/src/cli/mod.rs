//! [05] / [06] `releash workflow ...` CLI 入口。
//!
//! read-only 観測経路は engine と IPC せず、`workflow_runs/` 配下と `workflows/`
//! YAML / builtin を file-direct で読む（spec [05] design）。
//! mutating CLI (`approve` / `reject` / `abort`) も engine と直接 IPC せず、pending
//! command file を enqueue するところまでを CLI の責務に閉じる（spec [06] CLI 完了
//! 基準境界）。

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use clap::{CommandFactory, Parser, Subcommand};

use crate::agent_status::current_timestamp;
use crate::config::read_config_if_exists;
use crate::protocol::WorkflowStateView;
use crate::review_comments::{
    AuthorScope, ReviewActor, ReviewCommentStore, ReviewTarget, ReviewThreadFilter,
    ReviewThreadState,
};
use crate::session::{SessionState, SessionStore};
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
    /// Agent review comment サブコマンド。
    Review {
        #[command(subcommand)]
        command: ReviewSubcommand,
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
    /// [08] step に対する構造化出力の typed CLI 入口。`submit` / `validate` / `get`
    /// を持つ。submit のみ pending command 経由で engine に届ける（spec [08]）。
    Output {
        #[command(subcommand)]
        command: OutputSubcommand,
    },
}

#[derive(Subcommand, Debug)]
enum OutputSubcommand {
    /// step の `output_contract` に従う構造化出力を提出する。
    /// `--json` と `--file` は相互排他であり、いずれか必須。
    Submit {
        run_id: String,
        #[arg(long, value_name = "STEP_NAME")]
        step: String,
        #[arg(long = "type", value_name = "CONTRACT")]
        contract: String,
        #[arg(long, conflicts_with = "file", value_name = "JSON")]
        json: Option<String>,
        #[arg(long, conflicts_with = "json", value_name = "PATH")]
        file: Option<PathBuf>,
    },
    /// 構造化出力の `output_contract` 適合性を副作用なしで確認する。
    /// engine state / event log は変化しない。
    Validate {
        run_id: String,
        #[arg(long, value_name = "STEP_NAME")]
        step: String,
        #[arg(long, value_name = "PATH")]
        file: PathBuf,
    },
    /// 提出済みの構造化出力を取得する。未提出時は決定論的に「未提出」を返す。
    Get {
        run_id: String,
        #[arg(long, value_name = "STEP_NAME")]
        step: String,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand, Debug)]
enum ReviewSubcommand {
    /// review Thread 一覧を表示する。
    List {
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        file: Option<String>,
        #[arg(long)]
        state: Option<String>,
        #[arg(long)]
        author: Option<String>,
        #[arg(long)]
        unread: Option<String>,
        #[arg(long = "thread-id")]
        thread_id: Vec<String>,
        #[arg(long)]
        json: bool,
    },
    /// review Thread 詳細を表示する。
    Get {
        thread_id: String,
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        json: bool,
    },
    /// 初回 Comment とともに review Thread を作成する。
    Create {
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        content: String,
        #[arg(long)]
        file: Option<String>,
        #[arg(long)]
        line: Option<u32>,
        #[arg(long)]
        end_line: Option<u32>,
        #[arg(long)]
        json: bool,
    },
    /// open Thread に Comment を追記する。
    Comment {
        thread_id: String,
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        content: String,
        #[arg(long)]
        json: bool,
    },
    /// 作成者 Agent として open Thread を resolve する。
    Resolve {
        thread_id: String,
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        outcome: String,
        #[arg(long)]
        summary: String,
        #[arg(long)]
        json: bool,
    },
    /// Thread 履歴を表示する。
    History {
        thread_id: String,
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        json: bool,
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
    let workflows_dir = storage::workflows_dir();
    // data_dir は List / Runs / Status / Logs それぞれの branch 内で解決する。
    // List も workflow_runs/ 由来の `is_running` 反映のため data_dir を必要とするが、
    // 各 branch 内で解決することで未到達の branch では I/O を走らせない。
    // [05] 観測経路境界: data_dir 自体が存在しない場合は `NotFound` として扱い、
    // 「run が 0 件」と「向き先がそもそも無い」を区別する（5-1 修正）。
    let resolve = || -> Result<PathBuf, CliError> { resolve_existing_data_dir() };
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
            WorkflowSubcommand::Output { command } => match resolve() {
                Ok(data_dir) => match command {
                    OutputSubcommand::Submit {
                        run_id,
                        step,
                        contract,
                        json,
                        file,
                    } => cmd_output_submit(&data_dir, &run_id, &step, &contract, json, file),
                    OutputSubcommand::Validate { run_id, step, file } => {
                        cmd_output_validate(&data_dir, &run_id, &step, &file)
                    }
                    OutputSubcommand::Get { run_id, step, json } => {
                        cmd_output_get(&data_dir, &run_id, &step, json)
                    }
                },
                Err(e) => Err(e),
            },
        },
        TopCommand::Review { command } => match resolve() {
            Ok(data_dir) => cmd_review(&data_dir, command),
            Err(e) => Err(e),
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

/// データディレクトリの解決（パス計算のみ。存在チェックは行わない）。
///
/// Tauri 側 `AppHandle::path().app_data_dir()` と同等のパスを CLI 側で計算する。
/// CLI 起動独立性境界: デスクトップアプリ非稼働でも動作する。
///
/// 解決順序（spec [01]「解決順序: 明示指定 > alias 内包値 > プロセス既定」）:
/// 1. `RELEASH_DATA_DIR` の明示指定
/// 2. `PathAliases` から決まる alias 内包の data_dir。`PathAliases` が CLI alias 名・
///    実行 binary・data dir の単一所有者として dev / 本番 を切り分けるため、CLI 側で
///    `cfg!(debug_assertions)` を再実装しない
fn resolve_data_dir() -> Result<PathBuf, String> {
    resolve_data_dir_from_env(std::env::var("RELEASH_DATA_DIR").ok())
}

/// `resolve_data_dir` の pure 版（env を入力で受ける）。
///
/// spec [01] 解決順序「明示指定 > alias 内包値」をテストで検証可能にするための分離。
/// 明示指定が空文字列の場合は未設定扱いとし、alias 内包値にフォールバックする。
fn resolve_data_dir_from_env(env_value: Option<String>) -> Result<PathBuf, String> {
    if let Some(custom) = env_value.filter(|s| !s.is_empty()) {
        return Ok(PathBuf::from(custom));
    }
    let aliases = crate::path_aliases::PathAliases::from_runtime(None)?;
    Ok(aliases.releash().data_dir.clone())
}

/// data_dir を解決し、パスが実在することを確認する。
///
/// [05] 観測経路境界: `RELEASH_DATA_DIR` の typo / アプリ未起動などで data_dir
/// が存在しない場合に「runs が 0 件」と紛れないよう、CLI 入口で `NotFound`
/// として弾く（5-1 修正）。
fn resolve_existing_data_dir() -> Result<PathBuf, CliError> {
    let path = resolve_data_dir().map_err(CliError::Other)?;
    ensure_existing_data_dir(&path)?;
    Ok(path)
}

/// data_dir パスの実在を確認する純粋判定（環境変数に依存せずテスト可能）。
fn ensure_existing_data_dir(path: &Path) -> Result<(), CliError> {
    if !path.exists() {
        return Err(CliError::NotFound(format!(
            "data directory does not exist: {}",
            path.display()
        )));
    }
    Ok(())
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
    // [06] 入口バリデーション (5-2 修正): 不在 run_id への mutation は engine で
    // silent-drop されるため、pending file 書き出し前に CLI で弾く。spec [06] の
    // 「CLI 完了基準＝pending file 書き出しまで」境界は維持され、本チェックは
    // 書き出し前の入口バリデーションとして位置づける。
    if get_run_summary_file_direct(data_dir, run_id).is_none() {
        return Err(CliError::NotFound(format!(
            "Workflow run not found: {run_id}"
        )));
    }
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

#[cfg(test)]
fn review_actor(data_dir: &Path, session_id: &str) -> Result<ReviewActor, CliError> {
    review_actor_and_worktree(data_dir, session_id).map(|(actor, _)| actor)
}

fn review_actor_and_worktree(
    data_dir: &Path,
    session_id: &str,
) -> Result<(ReviewActor, String), CliError> {
    if session_id.trim().is_empty() {
        return Err(CliError::InvalidInput(
            "--session-id must not be empty".to_string(),
        ));
    }
    let session_store = SessionStore::default();
    let session = session_store
        .get_session(data_dir, session_id)
        .map_err(CliError::Other)?
        .ok_or_else(|| CliError::NotFound(format!("Session not found: {session_id}")))?;
    if session.state == SessionState::Closed {
        return Err(CliError::InvalidInput(format!(
            "Session is closed and cannot be used as a review actor: {session_id}"
        )));
    }
    let backend_id = session.backend_id.clone().ok_or_else(|| {
        CliError::InvalidInput(format!(
            "Session has no backend_id and cannot be used as a review actor: {session_id}"
        ))
    })?;
    let model = session.selected_model.clone().ok_or_else(|| {
        CliError::InvalidInput(format!(
            "Session has no selected_model and cannot be used as a review actor: {session_id}"
        ))
    })?;
    Ok(ReviewActor::agent(
        backend_id,
        model,
        Some(session_id.to_string()),
    ))
    .map(|actor| (actor, session.worktree_path))
}

/// 読み取り専用 review コマンド (`get` / `history`) 向けの軽量 helper。
///
/// `review_actor_and_worktree` は actor 構築のため `backend_id` / `selected_model` /
/// `state != Closed` を必須としているが、Get / History は worktree path しか必要としない。
/// このため過去セッションや actor 用フィールドを持たないセッションでも閲覧できるよう、
/// session 存在チェックと worktree path 取り出しのみを行う。Closed セッションも許可する。
fn review_worktree_from_session(data_dir: &Path, session_id: &str) -> Result<String, CliError> {
    if session_id.trim().is_empty() {
        return Err(CliError::InvalidInput(
            "--session-id must not be empty".to_string(),
        ));
    }
    let session_store = SessionStore::default();
    let session = session_store
        .get_session(data_dir, session_id)
        .map_err(CliError::Other)?
        .ok_or_else(|| CliError::NotFound(format!("Session not found: {session_id}")))?;
    Ok(session.worktree_path)
}

fn parse_review_state(value: Option<String>) -> Result<Option<ReviewThreadState>, CliError> {
    match value.as_deref() {
        None | Some("") => Ok(None),
        Some("open") => Ok(Some(ReviewThreadState::Open)),
        Some("resolved") => Ok(Some(ReviewThreadState::Resolved)),
        Some(other) => Err(CliError::InvalidInput(format!(
            "Invalid --state value: {other} (expected: open | resolved)"
        ))),
    }
}

fn parse_optional_author_scope(value: Option<String>) -> Result<Option<AuthorScope>, CliError> {
    match value.as_deref() {
        None | Some("") => Ok(None),
        Some("self") => Ok(Some(AuthorScope::Mine)),
        Some("other") => Ok(Some(AuthorScope::Other)),
        Some(other) => Err(CliError::InvalidInput(format!(
            "Invalid --author value: {other} (expected: self | other)"
        ))),
    }
}

fn parse_optional_unread(value: Option<String>) -> Result<Option<bool>, CliError> {
    match value.as_deref() {
        None | Some("") => Ok(None),
        Some("true") => Ok(Some(true)),
        Some("false") => Ok(Some(false)),
        Some(other) => Err(CliError::InvalidInput(format!(
            "Invalid --unread value: {other} (expected: true | false)"
        ))),
    }
}

fn review_error_to_cli_error(error: crate::review_comments::ReviewError) -> CliError {
    match error {
        crate::review_comments::ReviewError::InvalidInput(msg) => CliError::InvalidInput(msg),
        crate::review_comments::ReviewError::NotFound(msg) => CliError::NotFound(msg),
        crate::review_comments::ReviewError::AlreadyResolved(msg)
        | crate::review_comments::ReviewError::PermissionDenied(msg) => CliError::InvalidInput(msg),
        crate::review_comments::ReviewError::Io(e) => CliError::Other(e.to_string()),
        crate::review_comments::ReviewError::Serialize(e) => CliError::Other(e.to_string()),
    }
}

fn print_review_thread(
    thread: &crate::review_comments::ReviewThread,
    json: bool,
) -> Result<(), CliError> {
    if json {
        let text =
            serde_json::to_string_pretty(thread).map_err(|e| format!("serialize thread: {e}"))?;
        println!("{text}");
        return Ok(());
    }
    let location = match (
        thread.target.file_path.as_deref(),
        thread.target.line_number,
        thread.target.end_line,
    ) {
        (Some(file), Some(start), Some(end)) => format!("{file}:L{start}-L{end}"),
        (Some(file), Some(start), None) => format!("{file}:L{start}"),
        (Some(file), None, _) => file.to_string(),
        (None, _, _) => "(general)".to_string(),
    };
    println!(
        "thread_id: {}\nstate:     {:?}\nauthor:    {}\nlocation:  {}\nupdated:   {}\ncomments:  {}",
        thread.id,
        thread.state,
        thread.author.display_name,
        location,
        thread.updated_at,
        thread.comments.len()
    );
    if let Some(resolve) = &thread.resolve {
        println!(
            "resolve:   {} by {} ({})",
            resolve.outcome, resolve.actor.display_name, resolve.summary
        );
    }
    Ok(())
}

fn cmd_review(data_dir: &Path, command: ReviewSubcommand) -> Result<(), CliError> {
    let store = ReviewCommentStore::default();
    match command {
        ReviewSubcommand::List {
            session_id,
            file,
            state,
            author,
            unread,
            thread_id,
            json,
        } => {
            let (actor, review_worktree) = review_actor_and_worktree(data_dir, &session_id)?;
            let filter = ReviewThreadFilter {
                file,
                state: parse_review_state(state)?,
                author: parse_optional_author_scope(author)?,
                unread: parse_optional_unread(unread)?,
                thread_id,
            };
            let threads = store
                .list_threads(data_dir, &review_worktree, Some(filter), actor)
                .map_err(review_error_to_cli_error)?;
            if json {
                let text = serde_json::to_string_pretty(&threads)
                    .map_err(|e| format!("serialize threads: {e}"))?;
                println!("{text}");
            } else if threads.is_empty() {
                println!("(no review threads)");
            } else {
                println!(
                    "{:<36}  {:<9}  {:<20}  UPDATED",
                    "THREAD_ID", "STATE", "AUTHOR"
                );
                for thread in &threads {
                    println!(
                        "{:<36}  {:<9}  {:<20}  {}",
                        thread.id,
                        format!("{:?}", thread.state).to_lowercase(),
                        truncate(&thread.author.display_name, 20),
                        thread.updated_at
                    );
                }
            }
            Ok(())
        }
        ReviewSubcommand::Get {
            thread_id,
            session_id,
            json,
        } => {
            let review_worktree = review_worktree_from_session(data_dir, &session_id)?;
            let thread = store
                .get_thread(data_dir, &review_worktree, &thread_id)
                .map_err(review_error_to_cli_error)?;
            print_review_thread(&thread, json)
        }
        ReviewSubcommand::Create {
            session_id,
            content,
            file,
            line,
            end_line,
            json,
        } => {
            let (actor, review_worktree) = review_actor_and_worktree(data_dir, &session_id)?;
            let target = ReviewTarget {
                file_path: file,
                line_number: line,
                end_line,
            };
            let thread = store
                .create_thread(data_dir, &review_worktree, actor, target, content)
                .map_err(review_error_to_cli_error)?;
            print_review_thread(&thread, json)
        }
        ReviewSubcommand::Comment {
            thread_id,
            session_id,
            content,
            json,
        } => {
            let (actor, review_worktree) = review_actor_and_worktree(data_dir, &session_id)?;
            let thread = store
                .append_comment(data_dir, &review_worktree, actor, &thread_id, content)
                .map_err(review_error_to_cli_error)?;
            print_review_thread(&thread, json)
        }
        ReviewSubcommand::Resolve {
            thread_id,
            session_id,
            outcome,
            summary,
            json,
        } => {
            let (actor, review_worktree) = review_actor_and_worktree(data_dir, &session_id)?;
            let thread = store
                .resolve_thread(
                    data_dir,
                    &review_worktree,
                    actor,
                    &thread_id,
                    outcome,
                    summary,
                )
                .map_err(review_error_to_cli_error)?;
            print_review_thread(&thread, json)
        }
        ReviewSubcommand::History {
            thread_id,
            session_id,
            json,
        } => {
            let review_worktree = review_worktree_from_session(data_dir, &session_id)?;
            let events = store
                .history(data_dir, &review_worktree, &thread_id)
                .map_err(review_error_to_cli_error)?;
            if json {
                let text = serde_json::to_string_pretty(&events)
                    .map_err(|e| format!("serialize history: {e}"))?;
                println!("{text}");
            } else if events.is_empty() {
                println!("(no review history)");
            } else {
                for event in events {
                    println!("{:?}", event);
                }
            }
            Ok(())
        }
    }
}

/// [08] `releash workflow output submit`: 構造化出力を pending command として書き出す。
/// engine 到達は稼働中アプリの watcher が担う（spec [08] CLI 完了基準境界）。
///
/// CLI 入口で同期的に以下を検証し、不適合な入力は pending file を作らずに
/// `CliError::InvalidInput` として返す（spec [08]:「不適合な入力は決定論的な
/// validation error として CLI 終了コードで返り、`step_outputs` / 事実履歴は
/// 副作用なしの状態に保たれる」）。
///
/// 検証項目:
///   - run が存在する（event log の RunStarted から workflow を解決）
///   - step が workflow に存在し output_contract を持つ
///   - caller の `--type` が step の expected contract と一致する
///   - 入力 JSON が contract 適合（pure validator 再利用）
///
/// stale step 判定（受付中であるか）は engine の権威に委ねる。
fn cmd_output_submit(
    data_dir: &Path,
    run_id: &str,
    step: &str,
    contract: &str,
    json_arg: Option<String>,
    file_arg: Option<PathBuf>,
) -> Result<(), CliError> {
    validate_run_id(run_id)?;
    validate_step_argument(step)?;
    validate_contract_argument(contract)?;
    let raw_json = read_submit_input_json(json_arg, file_arg)?;
    let structured_output: serde_json::Value = serde_json::from_str(&raw_json)
        .map_err(|e| CliError::InvalidInput(format!("Failed to parse JSON: {e}")))?;

    // 同期検証: run / step / contract type 一致 / contract validation。
    let expected_contract = resolve_step_output_contract_via_log(data_dir, run_id, step)?;
    if expected_contract != contract {
        return Err(CliError::InvalidInput(format!(
            "contract mismatch: step '{step}' expects '{expected_contract}', got '{contract}'"
        )));
    }
    // [08] preflight と本 submit (`handle_submit_output`) で同一の前処理 + validation を
    // 共有するため、`preprocess_and_validate_output_with_secrets` 経由で呼ぶ。
    // CLI は別プロセスでアプリ状態 (`AppConfig` / `AppHandle`) を持たないため、ここでは
    // `secrets = &[]` で呼び、最終的な masking 込み判定は engine 側 watcher 経由で再評価される
    // （spec [08] CLI 完了基準: pending を書き出した時点で CLI は完了、最終判定は engine 側）。
    match crate::workflow::engine::WorkflowEngine::preprocess_and_validate_output_with_secrets(
        contract,
        structured_output.clone(),
        &[],
    ) {
        crate::workflow::contract::ContractValidationResult::Valid { .. } => {}
        crate::workflow::contract::ContractValidationResult::Invalid(violation) => {
            return Err(CliError::InvalidInput(format!(
                "contract violation ({}): {}",
                violation.reason, violation.details
            )));
        }
    }

    let output = enqueue_pending_command(
        data_dir,
        run_id,
        CliRequestPayload::SubmitOutput {
            step_name: step.to_string(),
            contract: contract.to_string(),
            structured_output,
        },
    )?;
    println!("{}", output.format_stdout_line());
    Ok(())
}

/// [08] `releash workflow output validate`: contract 適合性のみを確認する。
/// pending file / event log / state のいずれにも触れない（spec [08] 振る舞い定義 Rule 2）。
fn cmd_output_validate(
    data_dir: &Path,
    run_id: &str,
    step: &str,
    file: &Path,
) -> Result<(), CliError> {
    validate_run_id(run_id)?;
    validate_step_argument(step)?;
    let contract = resolve_step_output_contract_via_log(data_dir, run_id, step)?;
    let raw_json = std::fs::read_to_string(file)
        .map_err(|e| CliError::InvalidInput(format!("Failed to read file {:?}: {e}", file)))?;
    let value: serde_json::Value = serde_json::from_str(&raw_json)
        .map_err(|e| CliError::InvalidInput(format!("Failed to parse JSON: {e}")))?;
    // [08] preflight と本 submit (`handle_submit_output`) で同一の前処理 + validation を
    // 共有するため、`preprocess_and_validate_output_with_secrets` 経由で呼ぶ。
    // CLI は別プロセスでアプリ状態を持たないため、ここでは `secrets = &[]` で呼ぶ
    // （最終 masking 込み judging は engine 側で再評価される）。
    match crate::workflow::engine::WorkflowEngine::preprocess_and_validate_output_with_secrets(
        &contract,
        value,
        &[],
    ) {
        crate::workflow::contract::ContractValidationResult::Valid { .. } => {
            println!("ok: contract '{contract}' is satisfied");
            Ok(())
        }
        crate::workflow::contract::ContractValidationResult::Invalid(violation) => {
            Err(CliError::InvalidInput(format!(
                "contract violation ({}): {}",
                violation.reason, violation.details
            )))
        }
    }
}

/// [08] `releash workflow output get`: 提出済みの構造化出力と付随メタを取得する。
/// 未提出の場合は決定論的に「未提出」を返す（spec [08] 振る舞い定義 Rule 3）。
fn cmd_output_get(data_dir: &Path, run_id: &str, step: &str, json: bool) -> Result<(), CliError> {
    validate_run_id(run_id)?;
    validate_step_argument(step)?;
    if get_run_summary_file_direct(data_dir, run_id).is_none() {
        return Err(CliError::NotFound(format!(
            "Workflow run not found: {run_id}"
        )));
    }
    // [08] 振る舞い定義 Rule 3 (5-5 修正): step が workflow に存在しない場合は
    // `output validate` と対称に `InvalidInput` を返す。`not_submitted` 出力は
    // 「step は存在するが未提出」専用とする。
    let _contract = resolve_step_output_contract_via_log(data_dir, run_id, step)?;
    let events = read_log(data_dir, run_id)?;
    let view = build_output_get_view(events, step);
    if json {
        let text =
            serde_json::to_string_pretty(&view).map_err(|e| format!("serialize output: {e}"))?;
        println!("{text}");
    } else {
        match &view {
            OutputGetView::Submitted {
                contract,
                structured_output,
                submitted_at,
                request_id,
                timestamp,
            } => {
                println!("submitted: step={step} contract={contract}");
                if let Some(sa) = submitted_at {
                    println!("submitted_at: {sa}");
                }
                if let Some(rid) = request_id {
                    println!("request_id: {rid}");
                }
                println!("timestamp: {timestamp}");
                println!(
                    "structured_output:\n{}",
                    serde_json::to_string_pretty(structured_output)
                        .map_err(|e| format!("serialize structured_output: {e}"))?
                );
            }
            OutputGetView::NotSubmitted => {
                println!("not_submitted: step={step}");
            }
        }
    }
    Ok(())
}

fn validate_step_argument(step: &str) -> Result<(), CliError> {
    if step.trim().is_empty() {
        return Err(CliError::InvalidInput(
            "--step must not be empty".to_string(),
        ));
    }
    Ok(())
}

fn validate_contract_argument(contract: &str) -> Result<(), CliError> {
    if contract.trim().is_empty() {
        return Err(CliError::InvalidInput(
            "--type must not be empty".to_string(),
        ));
    }
    Ok(())
}

fn read_submit_input_json(
    json_arg: Option<String>,
    file_arg: Option<PathBuf>,
) -> Result<String, CliError> {
    match (json_arg, file_arg) {
        (Some(_), Some(_)) => Err(CliError::InvalidInput(
            "--json and --file are mutually exclusive".to_string(),
        )),
        (Some(raw), None) => Ok(raw),
        (None, Some(path)) => std::fs::read_to_string(&path)
            .map_err(|e| CliError::InvalidInput(format!("Failed to read file {:?}: {e}", path))),
        (None, None) => Err(CliError::InvalidInput(
            "either --json or --file is required".to_string(),
        )),
    }
}

/// event log の `RunStarted` から workflow definition を取り出し、step の
/// `output_contract` を解決する。
///
/// 経路本体は pure helper
/// `workflow::contract::resolve_step_output_contract_from_events` に委譲し、CLI
/// 層は `ContractLookupError` を `CliError` に射影するだけを担う（[08]
/// アーキテクチャ概要: Contract 解決は engine と CLI 双方から再利用される
/// pure 関数。CLI 層は engine internals に依存しない境界）。
fn resolve_step_output_contract_via_log(
    data_dir: &Path,
    run_id: &str,
    step: &str,
) -> Result<String, CliError> {
    let events = read_log(data_dir, run_id)?;
    crate::workflow::contract::resolve_step_output_contract_from_events(&events, step).map_err(
        |err| match err {
            crate::workflow::contract::ContractLookupError::RunNotFound => {
                CliError::NotFound(format!("Workflow run not found: {run_id}"))
            }
            crate::workflow::contract::ContractLookupError::NoOutputContract {
                workflow_name,
                step,
            } => CliError::InvalidInput(format!(
                "step '{step}' has no output_contract in workflow '{workflow_name}'"
            )),
        },
    )
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum OutputGetView {
    Submitted {
        contract: String,
        structured_output: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        submitted_at: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
        timestamp: f64,
    },
    NotSubmitted,
}

fn build_output_get_view(events: Vec<WorkflowEvent>, step: &str) -> OutputGetView {
    // 最新の OutputSubmitted は pure projection helper（spec [08] L165 / 振る舞い定義
    // Rule 3）に集約する。CLI / Tauri 経路はそれぞれ自層の DTO（OutputGetView /
    // WorkflowGetOutputResponse）へ map するだけで挙動を共有する。
    match crate::workflow::event_projection::latest_output_submitted_for(&events, step) {
        Some(snapshot) => OutputGetView::Submitted {
            contract: snapshot.contract,
            structured_output: snapshot.structured_output,
            submitted_at: snapshot.submitted_at,
            request_id: snapshot.request_id,
            timestamp: snapshot.timestamp,
        },
        None => OutputGetView::NotSubmitted,
    }
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
    // CLI は独立したエントリポイント（composition root）として repository usecase を
    // 組み立て、Tauri 経路と同一ロジックで検証する。
    let usecase = crate::adaptor::controller::wiring::build_repository_usecase();
    canonicalize_managed_worktree_path_inner(&usecase, repo_paths, worktree_path.to_string())
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
        WorkflowEvent::CliMutationRejected { .. } => "CliMutationRejected",
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

    fn write_review_config(data_dir: &Path) {
        fs::write(
            data_dir.join("releash.toml"),
            r#"
[agents]
default = "codex"

[agents.codex]
models = ["gpt-5"]

[agents.claude]
models = ["opus"]
"#,
        )
        .unwrap();
    }

    fn write_review_session(
        data_dir: &Path,
        session_id: &str,
        backend_id: Option<&str>,
        model: Option<&str>,
    ) {
        let store = SessionStore::default();
        let session = crate::session::ChatSession {
            id: session_id.to_string(),
            worktree_path: "/repo".to_string(),
            messages: Vec::new(),
            state: crate::session::SessionState::Active,
            created_at: 1.0,
            updated_at: 1.0,
            agent_session_id: None,
            permission_mode: "edit".to_string(),
            selected_model: model.map(str::to_string),
            backend_id: backend_id.map(str::to_string),
            workflow_step_session: false,
        };
        store.save_session(data_dir, &session).unwrap();
    }

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

    #[test]
    fn review_actor_resolves_backend_and_model_from_session_id() {
        let tmp = TempDir::new().unwrap();
        write_review_config(tmp.path());
        let session_id = uuid::Uuid::new_v4().to_string();
        write_review_session(tmp.path(), &session_id, Some("codex"), Some("gpt-5"));

        let actor = review_actor(tmp.path(), &session_id).unwrap();

        assert_eq!(actor.backend_id.as_deref(), Some("codex"));
        assert_eq!(actor.model.as_deref(), Some("gpt-5"));
        assert_eq!(actor.session_id.as_deref(), Some(session_id.as_str()));
    }

    #[test]
    fn review_actor_uses_saved_backend_model_without_catalog_validation() {
        let tmp = TempDir::new().unwrap();
        write_review_config(tmp.path());

        let missing = review_actor(tmp.path(), &uuid::Uuid::new_v4().to_string());
        assert!(matches!(missing, Err(CliError::NotFound(_))));

        let session_id = uuid::Uuid::new_v4().to_string();
        write_review_session(tmp.path(), &session_id, Some("codex"), Some("fake-model"));
        let actor = review_actor(tmp.path(), &session_id).unwrap();
        assert_eq!(actor.backend_id.as_deref(), Some("codex"));
        assert_eq!(actor.model.as_deref(), Some("fake-model"));

        let missing_backend_id = uuid::Uuid::new_v4().to_string();
        write_review_session(tmp.path(), &missing_backend_id, None, Some("gpt-5"));
        assert!(matches!(
            review_actor(tmp.path(), &missing_backend_id),
            Err(CliError::InvalidInput(_))
        ));

        let missing_model_id = uuid::Uuid::new_v4().to_string();
        write_review_session(tmp.path(), &missing_model_id, Some("codex"), None);
        assert!(matches!(
            review_actor(tmp.path(), &missing_model_id),
            Err(CliError::InvalidInput(_))
        ));
    }

    #[test]
    fn review_cli_rejects_mutation_for_closed_session() {
        let tmp = TempDir::new().unwrap();
        write_review_config(tmp.path());
        let session_id = uuid::Uuid::new_v4().to_string();
        write_review_session(tmp.path(), &session_id, Some("codex"), Some("gpt-5"));

        SessionStore::default()
            .set_session_state(tmp.path(), &session_id, SessionState::Closed)
            .unwrap();
        let closed = cmd_review(
            tmp.path(),
            ReviewSubcommand::Create {
                session_id,
                content: "Claim".to_string(),
                file: None,
                line: None,
                end_line: None,
                json: true,
            },
        );
        match closed {
            Err(CliError::InvalidInput(msg)) => assert!(msg.contains("Session is closed")),
            other => panic!("expected closed session rejection, got {other:?}"),
        }
    }

    #[test]
    fn review_cli_parser_accepts_review_subcommands() {
        let parsed = Cli::try_parse_from([
            "releash",
            "review",
            "create",
            "--session-id",
            "session-1",
            "--content",
            "Claim",
            "--json",
        ])
        .unwrap();

        match parsed.command {
            TopCommand::Review {
                command:
                    ReviewSubcommand::Create {
                        session_id,
                        content,
                        json,
                        ..
                    },
            } => {
                assert_eq!(session_id, "session-1");
                assert_eq!(content, "Claim");
                assert!(json);
            }
            _ => panic!("expected review create command"),
        }
    }

    /// `--worktree` フラグは spec design.md L37 で「提供しない」と明示されたため
    /// 受け付けない（session_id から worktree を解決する）。
    #[test]
    fn review_cli_parser_rejects_worktree_flag() {
        let result = Cli::try_parse_from([
            "releash",
            "review",
            "create",
            "--worktree",
            "/repo",
            "--session-id",
            "session-1",
            "--content",
            "Claim",
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn cmd_review_create_list_get_and_json_mode_use_session_worktree_key() {
        let tmp = TempDir::new().unwrap();
        write_review_config(tmp.path());
        let session_id = uuid::Uuid::new_v4().to_string();
        write_review_session(tmp.path(), &session_id, Some("codex"), Some("gpt-5"));

        cmd_review(
            tmp.path(),
            ReviewSubcommand::Create {
                session_id: session_id.clone(),
                content: "Claim".to_string(),
                file: None,
                line: None,
                end_line: None,
                json: true,
            },
        )
        .unwrap();

        let store = ReviewCommentStore::default();
        let threads = store
            .list_threads(tmp.path(), "/repo", None, ReviewActor::human())
            .unwrap();
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].worktree_name, "/repo");
        let json = serde_json::to_string(&threads[0]).unwrap();
        assert!(!json.contains("sessionId"));

        cmd_review(
            tmp.path(),
            ReviewSubcommand::List {
                session_id: session_id.clone(),
                file: None,
                state: Some("open".to_string()),
                author: None,
                unread: None,
                thread_id: Vec::new(),
                json: true,
            },
        )
        .unwrap();
        cmd_review(
            tmp.path(),
            ReviewSubcommand::Get {
                thread_id: threads[0].id.clone(),
                session_id,
                json: true,
            },
        )
        .unwrap();
    }

    #[test]
    fn cmd_review_comment_resolve_history_and_rejections_use_domain_reasons() {
        let tmp = TempDir::new().unwrap();
        write_review_config(tmp.path());
        let owner_session = uuid::Uuid::new_v4().to_string();
        let other_session = uuid::Uuid::new_v4().to_string();
        write_review_session(tmp.path(), &owner_session, Some("codex"), Some("gpt-5"));
        write_review_session(tmp.path(), &other_session, Some("claude"), Some("opus"));

        cmd_review(
            tmp.path(),
            ReviewSubcommand::Create {
                session_id: owner_session.clone(),
                content: "Claim".to_string(),
                file: Some("src/main.rs".to_string()),
                line: Some(3),
                end_line: Some(5),
                json: true,
            },
        )
        .unwrap();
        let store = ReviewCommentStore::default();
        let thread_id = store
            .list_threads(tmp.path(), "/repo", None, ReviewActor::human())
            .unwrap()[0]
            .id
            .clone();

        cmd_review(
            tmp.path(),
            ReviewSubcommand::Comment {
                thread_id: thread_id.clone(),
                session_id: owner_session.clone(),
                content: "Follow-up".to_string(),
                json: true,
            },
        )
        .unwrap();
        cmd_review(
            tmp.path(),
            ReviewSubcommand::Comment {
                thread_id: thread_id.clone(),
                session_id: owner_session.clone(),
                content: "Another follow-up".to_string(),
                json: true,
            },
        )
        .unwrap();
        cmd_review(
            tmp.path(),
            ReviewSubcommand::History {
                thread_id: thread_id.clone(),
                session_id: owner_session.clone(),
                json: true,
            },
        )
        .unwrap();

        // 別 backend/model session からの Resolve も participant identity に依らず成功する
        // (spec issues-1022: Resolve 権限は participant 識別に依存しない)。
        cmd_review(
            tmp.path(),
            ReviewSubcommand::Resolve {
                thread_id: thread_id.clone(),
                session_id: other_session,
                outcome: "accepted".to_string(),
                summary: "non-owner resolve".to_string(),
                json: true,
            },
        )
        .unwrap();
        // resolved 後の Resolve / Comment 追記は state により拒否される。
        let rejected_after_resolve = cmd_review(
            tmp.path(),
            ReviewSubcommand::Resolve {
                thread_id: thread_id.clone(),
                session_id: owner_session.clone(),
                outcome: "accepted".to_string(),
                summary: "second resolve".to_string(),
                json: true,
            },
        );
        match rejected_after_resolve {
            Err(CliError::InvalidInput(msg)) => assert!(msg.contains("already resolved")),
            other => panic!("expected resolved rejection, got {other:?}"),
        }
        let rejected_late_comment = cmd_review(
            tmp.path(),
            ReviewSubcommand::Comment {
                thread_id: thread_id.clone(),
                session_id: owner_session.clone(),
                content: "late".to_string(),
                json: true,
            },
        );
        match rejected_late_comment {
            Err(CliError::InvalidInput(msg)) => assert!(msg.contains("already resolved")),
            other => panic!("expected resolved rejection, got {other:?}"),
        }

        let missing_history = cmd_review(
            tmp.path(),
            ReviewSubcommand::History {
                thread_id: "missing-thread".to_string(),
                session_id: owner_session.clone(),
                json: true,
            },
        );
        assert!(matches!(missing_history, Err(CliError::NotFound(_))));

        let invalid_target = cmd_review(
            tmp.path(),
            ReviewSubcommand::Create {
                session_id: owner_session,
                content: "Bad target".to_string(),
                file: Some("../secret".to_string()),
                line: Some(1),
                end_line: None,
                json: true,
            },
        );
        assert!(matches!(invalid_target, Err(CliError::InvalidInput(_))));
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
                variables: Default::default(),
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
            variables: Default::default(),
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

    /// 本 issue ([08]) で `output` サブコマンド群を新規公開した後も、新規 run 起動
    /// （`run`）と top-level `submit` は別 issue 範囲のため parser 段階で reject される
    /// 境界を担保する。`output` 単独（subcommand 未指定）も clap が reject する。
    #[test]
    fn cli_does_not_expose_out_of_scope_subcommands() {
        for argv in [
            vec!["releash", "workflow", "run"],
            vec!["releash", "workflow", "submit"],
            vec!["releash", "workflow", "output"],
        ] {
            assert!(
                Cli::try_parse_from(&argv).is_err(),
                "parser must reject out-of-scope subcommand: {argv:?}"
            );
        }
    }

    /// [08] CLI 公開入口の parse 境界: `releash workflow output {submit,validate,get}`
    /// の typed subcommand が clap で parse できる。I/O は発生させない。
    #[test]
    fn cli_workflow_output_subcommands_parse_via_clap() {
        let run_id = "550e8400-e29b-41d4-a716-446655440000";
        for argv in [
            vec![
                "releash",
                "workflow",
                "output",
                "submit",
                run_id,
                "--step",
                "review",
                "--type",
                "review-verdict",
                "--json",
                "{\"verdict\":\"LGTM\"}",
            ],
            vec![
                "releash",
                "workflow",
                "output",
                "submit",
                run_id,
                "--step",
                "review",
                "--type",
                "review-verdict",
                "--file",
                "out.json",
            ],
            vec![
                "releash", "workflow", "output", "validate", run_id, "--step", "review", "--file",
                "out.json",
            ],
            vec![
                "releash", "workflow", "output", "get", run_id, "--step", "review",
            ],
            vec![
                "releash", "workflow", "output", "get", run_id, "--step", "review", "--json",
            ],
        ] {
            assert!(
                Cli::try_parse_from(&argv).is_ok(),
                "parser must accept workflow output subcommand: {argv:?}"
            );
        }
    }

    /// [08] CLI 入力境界: `submit` の `--json` と `--file` は相互排他。両方指定された
    /// 場合は parser が reject する。
    #[test]
    fn cli_workflow_output_submit_rejects_both_json_and_file() {
        let run_id = "550e8400-e29b-41d4-a716-446655440000";
        let argv = vec![
            "releash",
            "workflow",
            "output",
            "submit",
            run_id,
            "--step",
            "review",
            "--type",
            "review-verdict",
            "--json",
            "{}",
            "--file",
            "out.json",
        ];
        assert!(Cli::try_parse_from(&argv).is_err());
    }

    /// テスト用 helper: 指定 step に output_contract を持つ workflow を含む RunStarted
    /// event を log に append し、run_file も書き込む。CLI submit / validate の synchronous
    /// 解決経路は event log の RunStarted から workflow を取り出すため、テストでも本前提を
    /// 揃えてからコマンドを呼ぶ。
    fn seed_submit_workflow_log(
        data_dir: &Path,
        run_id: &str,
        worktree_path: &str,
        step_name: &str,
        contract: &str,
    ) {
        let workflow = crate::workflow::schema::Workflow {
            variables: Default::default(),
            name: "wf".to_string(),
            description: String::new(),
            builtin: false,
            nodes: vec![crate::workflow::schema::NodeDefinition {
                name: step_name.to_string(),
                node_type: crate::workflow::schema::NodeType::Agent,
                output_contract: Some(contract.to_string()),
                ..Default::default()
            }],
        };
        write_run_file(
            data_dir,
            &make_run(run_id, worktree_path, RunStatus::Running, 100.0),
        );
        let log = WorkflowEventLog::new(data_dir);
        log.append(&WorkflowEvent::RunStarted {
            run_id: run_id.to_string(),
            workflow_name: "wf".to_string(),
            workflow_file_stem: "wf".to_string(),
            worktree_path: worktree_path.to_string(),
            workflow_definition: workflow,
            timestamp: 100.0,
        })
        .unwrap();
    }

    /// [08] CLI 完了基準境界: `submit` は受理キュー投入（pending file の書き出し）まで
    /// 完了した時点で `Ok(())` を返す。書き出された pending entry は
    /// `PendingCommandPayload::SubmitOutput` shape を持ち、`run_id` と `step` /
    /// `contract` / structured_output が永続化される（spec [08] CLI 完了基準）。
    #[test]
    fn cmd_output_submit_writes_pending_file_with_typed_payload() {
        use crate::workflow::pending_command::PendingCommandPayload;
        let tmp = TempDir::new().unwrap();
        let run_id = test_uuid(91);
        seed_submit_workflow_log(
            tmp.path(),
            &run_id,
            "/wt/submit-pending",
            "review",
            "review-verdict",
        );
        let json = "{\"verdict\":\"LGTM\"}";
        cmd_output_submit(
            tmp.path(),
            &run_id,
            "review",
            "review-verdict",
            Some(json.to_string()),
            None,
        )
        .unwrap();
        let entries = PendingCommandStore::new(tmp.path()).list_pending().unwrap();
        assert_eq!(entries.len(), 1);
        match &entries[0].command.payload {
            PendingCommandPayload::SubmitOutput {
                step_name,
                contract,
                structured_output,
            } => {
                assert_eq!(step_name, "review");
                assert_eq!(contract, "review-verdict");
                assert_eq!(structured_output["verdict"], "LGTM");
            }
            other => panic!("expected SubmitOutput payload, got: {other:?}"),
        }
    }

    /// [08] CLI 完了基準境界: `--file` 入力でも pending file が書き出される。
    #[test]
    fn cmd_output_submit_writes_pending_file_from_file_arg() {
        use crate::workflow::pending_command::PendingCommandPayload;
        let tmp = TempDir::new().unwrap();
        let input_file = tmp.path().join("input.json");
        std::fs::write(&input_file, b"{\"verdict\":\"LGTM\"}").unwrap();
        let run_id = test_uuid(92);
        seed_submit_workflow_log(
            tmp.path(),
            &run_id,
            "/wt/submit-pending-file",
            "review",
            "review-verdict",
        );
        cmd_output_submit(
            tmp.path(),
            &run_id,
            "review",
            "review-verdict",
            None,
            Some(input_file),
        )
        .unwrap();
        let entries = PendingCommandStore::new(tmp.path()).list_pending().unwrap();
        assert_eq!(entries.len(), 1);
        assert!(matches!(
            entries[0].command.payload,
            PendingCommandPayload::SubmitOutput { .. }
        ));
    }

    /// [08] CLI 入力境界: `--json` も `--file` も指定されない場合は関数レベルで reject。
    #[test]
    fn cmd_output_submit_rejects_missing_json_and_file() {
        let tmp = TempDir::new().unwrap();
        let run_id = test_uuid(93);
        let err = cmd_output_submit(tmp.path(), &run_id, "review", "review-verdict", None, None)
            .unwrap_err();
        assert!(matches!(err, CliError::InvalidInput(_)));
    }

    /// [08] CLI 入力境界: `--json` に invalid JSON が渡されたら InvalidInput を返す。
    #[test]
    fn cmd_output_submit_rejects_invalid_json() {
        let tmp = TempDir::new().unwrap();
        let run_id = test_uuid(94);
        let err = cmd_output_submit(
            tmp.path(),
            &run_id,
            "review",
            "review-verdict",
            Some("{not json}".to_string()),
            None,
        )
        .unwrap_err();
        assert!(matches!(err, CliError::InvalidInput(_)));
    }

    /// [08] 振る舞い定義 Rule 1: 対象 run が存在しなければ pending file を作らず CliError。
    /// CLI 同期検証の reject 経路を担保する（spec [08] 副作用なし境界）。
    #[test]
    fn cmd_output_submit_rejects_unknown_run_without_side_effects() {
        let tmp = TempDir::new().unwrap();
        let run_id = test_uuid(101);
        let err = cmd_output_submit(
            tmp.path(),
            &run_id,
            "review",
            "review-verdict",
            Some("{\"verdict\":\"LGTM\"}".to_string()),
            None,
        )
        .unwrap_err();
        assert!(matches!(err, CliError::NotFound(_)));
        // pending file が作られていない
        assert!(PendingCommandStore::new(tmp.path())
            .list_pending()
            .unwrap()
            .is_empty());
    }

    /// [08] CLI 同期検証: workflow に存在しない step に対する submit は
    /// pending file を作らず CliError::InvalidInput を返す。
    #[test]
    fn cmd_output_submit_rejects_unknown_step_without_side_effects() {
        let tmp = TempDir::new().unwrap();
        let run_id = test_uuid(102);
        seed_submit_workflow_log(
            tmp.path(),
            &run_id,
            "/wt/submit-unknown-step",
            "review",
            "review-verdict",
        );
        let err = cmd_output_submit(
            tmp.path(),
            &run_id,
            "no-such-step",
            "review-verdict",
            Some("{\"verdict\":\"LGTM\"}".to_string()),
            None,
        )
        .unwrap_err();
        assert!(matches!(err, CliError::InvalidInput(_)));
        assert!(PendingCommandStore::new(tmp.path())
            .list_pending()
            .unwrap()
            .is_empty());
    }

    /// [08] CLI 同期検証: caller の `--type` が step の expected contract と
    /// 一致しない場合は pending file を作らず CliError::InvalidInput。
    #[test]
    fn cmd_output_submit_rejects_contract_type_mismatch_without_side_effects() {
        let tmp = TempDir::new().unwrap();
        let run_id = test_uuid(103);
        seed_submit_workflow_log(
            tmp.path(),
            &run_id,
            "/wt/submit-type-mismatch",
            "review",
            "review-verdict",
        );
        let err = cmd_output_submit(
            tmp.path(),
            &run_id,
            "review",
            "fix-result",
            Some("{\"status\":\"FIXED\"}".to_string()),
            None,
        )
        .unwrap_err();
        assert!(matches!(err, CliError::InvalidInput(_)));
        assert!(PendingCommandStore::new(tmp.path())
            .list_pending()
            .unwrap()
            .is_empty());
    }

    /// [08] CLI 同期検証: contract に適合しない JSON は pending file を作らず CliError。
    #[test]
    fn cmd_output_submit_rejects_contract_violation_without_side_effects() {
        let tmp = TempDir::new().unwrap();
        let run_id = test_uuid(104);
        seed_submit_workflow_log(
            tmp.path(),
            &run_id,
            "/wt/submit-violation",
            "review",
            "spec-directory",
        );
        let err = cmd_output_submit(
            tmp.path(),
            &run_id,
            "review",
            "spec-directory",
            Some("{\"spec_dir\":\"/not/relative\"}".to_string()),
            None,
        )
        .unwrap_err();
        assert!(matches!(err, CliError::InvalidInput(_)));
        assert!(PendingCommandStore::new(tmp.path())
            .list_pending()
            .unwrap()
            .is_empty());
    }

    /// [08] 振る舞い定義 Rule 2: validate は pure validator を呼び、副作用を起こさない。
    /// pending dir / event log / run file / 既存 step_outputs のいずれも変化しない。
    #[test]
    fn cmd_output_validate_returns_ok_for_contract_compliant_input() {
        let tmp = TempDir::new().unwrap();
        let run_id = test_uuid(95);
        // RunStarted event に workflow definition を埋め込む
        let yaml = crate::workflow::schema::Workflow {
            variables: Default::default(),
            name: "wf".to_string(),
            description: String::new(),
            builtin: false,
            nodes: vec![crate::workflow::schema::NodeDefinition {
                name: "review".to_string(),
                node_type: crate::workflow::schema::NodeType::Agent,
                output_contract: Some("spec-directory".to_string()),
                ..Default::default()
            }],
        };
        write_run_file(
            tmp.path(),
            &make_run(&run_id, "/wt/validate", RunStatus::Running, 100.0),
        );
        let log = WorkflowEventLog::new(tmp.path());
        log.append(&WorkflowEvent::RunStarted {
            run_id: run_id.clone(),
            workflow_name: "wf".to_string(),
            workflow_file_stem: "wf".to_string(),
            worktree_path: "/wt/validate".to_string(),
            workflow_definition: yaml,
            timestamp: 100.0,
        })
        .unwrap();
        // 既存の OutputSubmitted を 1 件入れておき、validate がそれを変化させないことも確認する。
        log.append(&WorkflowEvent::OutputSubmitted {
            run_id: run_id.clone(),
            workflow_name: "wf".to_string(),
            node_name: "review".to_string(),
            contract: "review-verdict".to_string(),
            structured_output: serde_json::json!({"verdict": "LGTM"}),
            request_id: Some("00000000-0000-0000-0000-0000000000aa".to_string()),
            submitted_at: Some(120.0),
            timestamp: 130.0,
        })
        .unwrap();
        let input_file = tmp.path().join("input.json");
        std::fs::write(&input_file, b"{\"spec_dir\":\"docs/specs/issues-123\"}").unwrap();

        let run_file_path = tmp
            .path()
            .join("workflow_runs")
            .join(format!("{run_id}.json"));
        let event_log_before = log.read_log(&run_id).unwrap();
        let run_file_before = std::fs::read_to_string(&run_file_path).unwrap();

        cmd_output_validate(tmp.path(), &run_id, "review", &input_file).unwrap();

        // pending dir 不変
        assert!(PendingCommandStore::new(tmp.path())
            .list_pending()
            .unwrap()
            .is_empty());
        // event log の長さ・内容が validate 前後で一致
        let event_log_after = log.read_log(&run_id).unwrap();
        assert_eq!(event_log_before.len(), event_log_after.len());
        // run file の中身が変わっていない
        let run_file_after = std::fs::read_to_string(&run_file_path).unwrap();
        assert_eq!(run_file_before, run_file_after);
        // 既存の OutputSubmitted がそのまま残る（reconstruct で step_outputs slot が不変）
        let view = build_output_get_view(event_log_after.clone(), "review");
        assert!(matches!(
            view,
            OutputGetView::Submitted {
                ref contract,
                submitted_at: Some(120.0),
                timestamp: 130.0,
                ..
            } if contract == "review-verdict"
        ));
    }

    #[test]
    fn cmd_output_validate_returns_err_for_invalid_contract_input() {
        let tmp = TempDir::new().unwrap();
        let run_id = test_uuid(96);
        let workflow = crate::workflow::schema::Workflow {
            variables: Default::default(),
            name: "wf".to_string(),
            description: String::new(),
            builtin: false,
            nodes: vec![crate::workflow::schema::NodeDefinition {
                name: "review".to_string(),
                node_type: crate::workflow::schema::NodeType::Agent,
                output_contract: Some("spec-directory".to_string()),
                ..Default::default()
            }],
        };
        write_run_file(
            tmp.path(),
            &make_run(&run_id, "/wt/validate-fail", RunStatus::Running, 100.0),
        );
        let log = WorkflowEventLog::new(tmp.path());
        log.append(&WorkflowEvent::RunStarted {
            run_id: run_id.clone(),
            workflow_name: "wf".to_string(),
            workflow_file_stem: "wf".to_string(),
            worktree_path: "/wt/validate-fail".to_string(),
            workflow_definition: workflow,
            timestamp: 100.0,
        })
        .unwrap();
        let input_file = tmp.path().join("input.json");
        std::fs::write(&input_file, b"{\"spec_dir\":\"/not/relative\"}").unwrap();

        let err = cmd_output_validate(tmp.path(), &run_id, "review", &input_file).unwrap_err();
        assert!(matches!(err, CliError::InvalidInput(_)));
    }

    /// [08] 振る舞い定義 Rule 3: `get` は提出済み output と付随メタを返す。
    /// 同 step に対する複数 OutputSubmitted のうち最後のものを採用する。
    #[test]
    fn cmd_output_get_returns_submitted_when_event_present() {
        let tmp = TempDir::new().unwrap();
        let run_id = test_uuid(97);
        write_run_file(
            tmp.path(),
            &make_run(&run_id, "/wt/get", RunStatus::Running, 100.0),
        );
        let log = WorkflowEventLog::new(tmp.path());
        log.append(&run_started_event(&run_id, "wf", "/wt/get"))
            .unwrap();
        log.append(&WorkflowEvent::OutputSubmitted {
            run_id: run_id.clone(),
            workflow_name: "wf".to_string(),
            node_name: "review".to_string(),
            contract: "review-verdict".to_string(),
            structured_output: serde_json::json!({"verdict": "LGTM"}),
            request_id: Some("req-1".to_string()),
            submitted_at: Some(150.0),
            timestamp: 200.0,
        })
        .unwrap();

        let view = build_output_get_view(log.read_log(&run_id).unwrap(), "review");
        assert!(matches!(
            view,
            OutputGetView::Submitted {
                ref contract,
                submitted_at: Some(150.0),
                timestamp: 200.0,
                ..
            } if contract == "review-verdict"
        ));
    }

    /// [08] 振る舞い定義 Rule 3: 未提出 step は `NotSubmitted` を返す。
    #[test]
    fn cmd_output_get_returns_not_submitted_when_no_event() {
        let tmp = TempDir::new().unwrap();
        let run_id = test_uuid(98);
        write_run_file(
            tmp.path(),
            &make_run(&run_id, "/wt/get-empty", RunStatus::Running, 100.0),
        );
        let log = WorkflowEventLog::new(tmp.path());
        log.append(&run_started_event(&run_id, "wf", "/wt/get-empty"))
            .unwrap();

        let view = build_output_get_view(log.read_log(&run_id).unwrap(), "review");
        assert!(matches!(view, OutputGetView::NotSubmitted));
    }

    /// [08] 振る舞い定義 Rule 3: 同 step に対し複数の OutputSubmitted が記録された場合、
    /// 最後 (= 最新) の event を採用する。
    #[test]
    fn cmd_output_get_returns_latest_event_when_multiple_submitted() {
        let tmp = TempDir::new().unwrap();
        let run_id = test_uuid(99);
        write_run_file(
            tmp.path(),
            &make_run(&run_id, "/wt/get-latest", RunStatus::Running, 100.0),
        );
        let log = WorkflowEventLog::new(tmp.path());
        log.append(&run_started_event(&run_id, "wf", "/wt/get-latest"))
            .unwrap();
        log.append(&WorkflowEvent::OutputSubmitted {
            run_id: run_id.clone(),
            workflow_name: "wf".to_string(),
            node_name: "review".to_string(),
            contract: "review-verdict".to_string(),
            structured_output: serde_json::json!({"verdict": "NEEDS_FIX", "findings": [{"severity": "error", "message": "bug"}]}),
            request_id: None,
            submitted_at: None,
            timestamp: 110.0,
        })
        .unwrap();
        log.append(&WorkflowEvent::OutputSubmitted {
            run_id: run_id.clone(),
            workflow_name: "wf".to_string(),
            node_name: "review".to_string(),
            contract: "review-verdict".to_string(),
            structured_output: serde_json::json!({"verdict": "LGTM"}),
            request_id: None,
            submitted_at: None,
            timestamp: 120.0,
        })
        .unwrap();

        let view = build_output_get_view(log.read_log(&run_id).unwrap(), "review");
        match view {
            OutputGetView::Submitted {
                structured_output, ..
            } => {
                assert_eq!(structured_output["verdict"], "LGTM");
            }
            other => panic!("expected Submitted, got: {other:?}"),
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
        write_run_file(
            tmp.path(),
            &make_run(&run_id, "/wt/approve", RunStatus::Running, 100.0),
        );
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
        write_run_file(
            tmp.path(),
            &make_run(&run_id, "/wt/reject", RunStatus::Running, 100.0),
        );
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
        write_run_file(
            tmp.path(),
            &make_run(&run_id, "/wt/abort", RunStatus::Running, 100.0),
        );
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

    /// [05] 観測経路境界 (5-1 修正): data_dir が存在しない場合は `NotFound` として
    /// 扱い、「runs 0 件」と「向き先がそもそも無い」を区別する。
    #[test]
    fn ensure_existing_data_dir_returns_not_found_for_missing_path() {
        let missing = std::path::PathBuf::from("/non/existent/releash-data-dir-test-path");
        let err = ensure_existing_data_dir(&missing).expect_err("missing data_dir must error");
        let CliError::NotFound(msg) = &err else {
            panic!("expected CliError::NotFound for missing data_dir, got: {err:?}");
        };
        assert!(
            msg.contains(&missing.display().to_string()),
            "error message must contain the path, got: {msg}"
        );
    }

    /// [05] 観測経路境界 (5-1 修正): data_dir が存在する場合は Ok を返す。
    #[test]
    fn ensure_existing_data_dir_returns_ok_for_existing_path() {
        let tmp = TempDir::new().unwrap();
        ensure_existing_data_dir(tmp.path()).expect("existing data_dir must succeed");
    }

    /// [08] 振る舞い定義 Rule 3 (5-5 修正): `output get` は step が workflow に
    /// 存在しない場合、`output validate` と対称に `CliError::InvalidInput` を返す
    /// （exit=2）。`not_submitted` は「step は存在するが未提出」専用。
    #[test]
    fn cmd_output_get_rejects_unknown_step_with_invalid_input() {
        let tmp = TempDir::new().unwrap();
        let run_id = test_uuid(96);
        write_run_file(
            tmp.path(),
            &make_run(&run_id, "/wt/get-unknown-step", RunStatus::Running, 100.0),
        );
        // step "review" を持つ workflow を log に播種し、それとは別名の step を問い合わせる。
        seed_submit_workflow_log(
            tmp.path(),
            &run_id,
            "/wt/get-unknown-step",
            "review",
            "review-verdict",
        );
        let err =
            cmd_output_get(tmp.path(), &run_id, "nonexistent_step", true).expect_err("must error");
        let CliError::InvalidInput(msg) = &err else {
            panic!("expected CliError::InvalidInput for unknown step, got: {err:?}");
        };
        assert!(
            msg.contains("nonexistent_step"),
            "error message must include step name, got: {msg}"
        );
        assert!(
            msg.contains("output_contract"),
            "error message must mention output_contract (symmetric with validate), got: {msg}"
        );
    }

    /// [06] 入口バリデーション (5-2 修正): 不存在 run_id への mutation は pending
    /// file を書き出さずに `CliError::NotFound` で弾かれる。engine 到達後の
    /// silent-drop を待たずに CLI 入口で検知する。
    #[test]
    fn cmd_enqueue_pending_rejects_unknown_run_id_without_side_effects() {
        let tmp = TempDir::new().unwrap();
        let unknown_run_id = test_uuid(99);
        let payload = CliRequestPayload::Approve {
            node_name: None,
            comment: Some("x".to_string()),
        };
        let err = enqueue_pending_command(tmp.path(), &unknown_run_id, payload).unwrap_err();
        let CliError::NotFound(msg) = &err else {
            panic!("expected CliError::NotFound for unknown run_id, got: {err:?}");
        };
        assert!(
            msg.contains(&unknown_run_id),
            "error message must include run_id, got: {msg}"
        );
        // pending file は一切書き出されていない（副作用なし）。
        let entries = PendingCommandStore::new(tmp.path()).list_pending().unwrap();
        assert!(
            entries.is_empty(),
            "pending file must not be written when run is unknown"
        );
    }

    /// spec [01] 解決順序「明示指定 > alias 内包値」: RELEASH_DATA_DIR が明示
    /// 指定されている場合は、その値がそのまま採用される（PathBuf 化のみ）。
    #[test]
    fn resolve_data_dir_uses_explicit_env_when_set() {
        let resolved = resolve_data_dir_from_env(Some("/explicit/path".to_string())).unwrap();
        assert_eq!(resolved, std::path::PathBuf::from("/explicit/path"));
    }

    /// spec [01] 解決順序「明示指定 > alias 内包値」: 明示指定が無い場合は
    /// `PathAliases` から導いた alias 内包の data_dir を返す（既定値は bundle
    /// identifier suffix を持つ）。
    #[test]
    fn resolve_data_dir_falls_back_to_alias_data_dir_when_env_unset() {
        if dirs::data_dir().is_none() {
            return;
        }
        let resolved = resolve_data_dir_from_env(None).unwrap();
        let expected_suffix = crate::path_aliases::default_data_dir_name_for_profile(
            crate::path_aliases::BuildProfile::current(),
        );
        assert!(
            resolved.ends_with(expected_suffix),
            "expected suffix {expected_suffix}, got {}",
            resolved.display()
        );
    }

    /// spec [01]: 明示指定が空文字列のときは未設定扱いとし alias 内包値に
    /// フォールバックする（空文字列を data_dir として採用すると以降の
    /// 観測経路で「runs 0 件」と紛れるため）。
    #[test]
    fn resolve_data_dir_treats_empty_env_as_unset() {
        if dirs::data_dir().is_none() {
            return;
        }
        let resolved = resolve_data_dir_from_env(Some(String::new())).unwrap();
        let expected_suffix = crate::path_aliases::default_data_dir_name_for_profile(
            crate::path_aliases::BuildProfile::current(),
        );
        assert!(
            resolved.ends_with(expected_suffix),
            "empty env should fall through to alias data_dir, got {}",
            resolved.display()
        );
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

    /// Issue #1022: Agent process environment contract により、Releash CLI の
    /// long help が system_prompt 注入用の単一ソースとして取得可能でなければならない。
    /// 主要サブコマンド名が含まれることで Agent が review/workflow CLI を発見できる。
    #[test]
    fn render_long_help_contains_main_subcommands() {
        let help = super::render_long_help();
        assert!(
            help.contains("workflow"),
            "long help must list `workflow` subcommand, got: {help}"
        );
        assert!(
            help.contains("review"),
            "long help must list `review` subcommand, got: {help}"
        );
        assert!(
            help.contains("releash"),
            "long help must mention CLI name `releash`, got: {help}"
        );
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
}
