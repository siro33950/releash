use std::path::Path;

use clap::Subcommand;

use super::common::{
    approval_input_error_to_cli_error, truncate, validate_optional_cli_text_len, validate_run_id,
    CliError,
};
use super::output::OutputSubcommand;
use super::workflow_io;
use crate::adaptor::gateway::app_config::read_config_if_exists;
use crate::adaptor::gateway::workflow::{
    mapper, RepoPathsManagedWorktreeGateway, WorkflowDefinitionFileRepository,
    WorkflowRunFileRepository, WorkflowStateProjectionLogRepository,
};
use crate::adaptor::presenter::workflow::workflow_state_to_view;
use crate::adaptor::protocol::workflow::WorkflowStateView;
use crate::domain::workflow::{
    approval_rules, ManagedWorktreeGateway, RunId, RunListFilter, RunStatusFilter,
    WorkflowDefinitionRepository, WorkflowRunRepository, WorkflowRunSummary, WorkflowSummary,
};
use crate::usecase::workflow::ports::{WorkflowEventDraft, WorkflowStateProjectionRepository};

/// CLI の workflow サブコマンド集合。
///
/// workflow runtime の mutation usecase と語彙衝突しないよう CLI 側は
/// `WorkflowSubcommand` として分離する
/// （spec [05] read-only と mutating の分離 / observation source-of-truth の境界）。
#[derive(Subcommand, Debug)]
pub(super) enum WorkflowSubcommand {
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

pub(super) fn cmd_list(
    workflows_dir: &Path,
    data_dir: &Path,
    json: bool,
) -> Result<String, CliError> {
    let summaries = list_workflows_file_direct(workflows_dir, data_dir)?;
    if json {
        let wire: Vec<_> = summaries
            .clone()
            .into_iter()
            .map(mapper::domain_workflow_summary_to_legacy)
            .collect();
        let text =
            serde_json::to_string_pretty(&wire).map_err(|e| format!("serialize workflows: {e}"))?;
        Ok(format!("{text}\n"))
    } else {
        if summaries.is_empty() {
            return Ok("(no workflows)\n".to_string());
        }
        let mut output = String::new();
        for s in &summaries {
            let tag = if s.builtin { "[builtin]" } else { "         " };
            let running_marker = if s.is_running { " (running)" } else { "" };
            output.push_str(&format!(
                "{tag} {:<32}  {}{running_marker}\n",
                s.name, s.description
            ));
        }
        Ok(output)
    }
}

/// workflow run 一覧。
pub(super) fn cmd_runs(
    data_dir: &Path,
    status: Option<String>,
    worktree: Option<String>,
    json: bool,
) -> Result<String, CliError> {
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
    // ManagedWorktreeGateway を経由する。
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
    let summaries = list_runs_file_direct(data_dir, filter)?;
    if json {
        let wire: Vec<_> = summaries
            .clone()
            .into_iter()
            .map(mapper::domain_run_summary_to_legacy)
            .collect();
        let text =
            serde_json::to_string_pretty(&wire).map_err(|e| format!("serialize runs: {e}"))?;
        Ok(format!("{text}\n"))
    } else {
        if summaries.is_empty() {
            return Ok("(no runs)\n".to_string());
        }
        let mut output = format!(
            "{:<36}  {:<20}  {:<18}  WORKTREE\n",
            "RUN_ID", "WORKFLOW", "STATUS"
        );
        for s in &summaries {
            let status = format!("{:?}", s.status);
            output.push_str(&format!(
                "{:<36}  {:<20}  {:<18}  {}\n",
                s.run_id,
                truncate(&s.workflow_name, 20),
                status,
                s.worktree_path
            ));
        }
        Ok(output)
    }
}

/// 指定 run の現在 state。
pub(super) fn cmd_status(data_dir: &Path, run_id: &str, json: bool) -> Result<String, CliError> {
    validate_run_id(run_id)?;
    workflow_io::ensure_run_exists(data_dir, run_id)?;
    let view = reconstruct_state_view(data_dir, run_id)?;
    if json {
        let text =
            serde_json::to_string_pretty(&view).map_err(|e| format!("serialize state: {e}"))?;
        Ok(format!("{text}\n"))
    } else {
        Ok(format!(
            "run_id:        {}\nworkflow:      {}\nstate:         {:?}\ncurrent_step:  {}\nupdated_at:    {}",
            view.state.execution_id,
            view.state.workflow_name,
            view.state.state,
            view.state.current_step_name,
            view.state.updated_at,
        ) + "\n")
    }
}

/// [06] CLI mutating 経路の pending command 投入。
///
/// spec [06] Rule:「CLI の完了基準は『受理キュー投入』までで統一する」に従い、
/// 各 handler は pending command file の atomic 書き出しが完了した時点で stdout
/// 文字列を返す。engine への到達 / 認可結果は CLI 側で待たない（spec [06] CLI
/// 完了基準境界）。
pub(super) fn cmd_approve(
    data_dir: &Path,
    run_id: &str,
    node: Option<String>,
    comment: Option<String>,
) -> Result<String, CliError> {
    validate_optional_cli_text_len(comment.as_deref(), "--comment")?;
    cmd_enqueue_pending(
        data_dir,
        run_id,
        workflow_io::CliRequestPayload::Approve {
            node_name: node,
            comment,
        },
    )
}

pub(super) fn cmd_reject(
    data_dir: &Path,
    run_id: &str,
    node: Option<String>,
    reason: String,
) -> Result<String, CliError> {
    validate_reject_reason(&reason)?;
    cmd_enqueue_pending(
        data_dir,
        run_id,
        workflow_io::CliRequestPayload::Reject {
            node_name: node,
            reason,
        },
    )
}

pub(super) fn cmd_abort(
    data_dir: &Path,
    run_id: &str,
    node: Option<String>,
) -> Result<String, CliError> {
    cmd_enqueue_pending(
        data_dir,
        run_id,
        workflow_io::CliRequestPayload::Abort { node_name: node },
    )
}

fn cmd_enqueue_pending(
    data_dir: &Path,
    run_id: &str,
    payload: workflow_io::CliRequestPayload,
) -> Result<String, CliError> {
    let output = workflow_io::enqueue_pending_command(data_dir, run_id, payload)?;
    Ok(format!("{}\n", output.format_stdout_line()))
}

/// `--reason` 必須化境界（spec [06] 振る舞い定義 Rule: 却下要求には却下理由が伴う）。
/// `clap` で `--reason` を必須化済みだが、空白のみの入力は CLI 入口で拒否する。
///
/// 文字数上限 / 空白判定はドメイン pure helper
/// （`approval_rules::validate_reject_reason_text`）
/// に集約し、CLI 層は `CliError::InvalidInput` への map に閉じる（review R2-01）。
pub(super) fn validate_reject_reason(reason: &str) -> Result<(), CliError> {
    approval_rules::validate_reject_reason_text(reason, "--reason")
        .map_err(approval_input_error_to_cli_error)
}

/// 指定 run の event log。
pub(super) fn cmd_logs(data_dir: &Path, run_id: &str, json: bool) -> Result<String, CliError> {
    validate_run_id(run_id)?;
    workflow_io::ensure_run_exists(data_dir, run_id)?;
    let events = workflow_io::read_domain_log(data_dir, run_id)?;
    if json {
        let views: Vec<_> = events.iter().map(event_draft_to_cli_log_json).collect();
        let text =
            serde_json::to_string_pretty(&views).map_err(|e| format!("serialize log: {e}"))?;
        Ok(format!("{text}\n"))
    } else {
        let mut output = String::new();
        for event in &events {
            output.push_str(&format!("{}\n", format_event_draft(event)));
        }
        Ok(output)
    }
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
) -> Result<Vec<WorkflowSummary>, CliError> {
    let running: Vec<String> = running_workflow_names_file_direct(data_dir)?
        .into_iter()
        .collect();
    WorkflowDefinitionFileRepository::new(workflows_dir.to_path_buf(), workflows_dir.to_path_buf())
        .list(&running)
        .map_err(|e| CliError::Other(e.to_string()))
}

fn running_workflow_names_file_direct(
    data_dir: &Path,
) -> Result<std::collections::HashSet<String>, CliError> {
    Ok(list_runs_file_direct(
        data_dir,
        RunListFilter {
            status: Some(RunStatusFilter::Active),
            worktree_path: None,
        },
    )?
    .into_iter()
    .map(|run| run.workflow_name)
    .collect())
}

/// `workflow_runs/` を file-direct repository 経由で走査し、filter を適用した
/// domain summary 一覧を返す。API 経路と同じ `WorkflowRunRepository` port に寄せることで
/// 観測ロジックの divergence を防ぐ（spec [05] API / CLI の意味的等価性境界）。
fn list_runs_file_direct(
    data_dir: &Path,
    filter: RunListFilter,
) -> Result<Vec<WorkflowRunSummary>, CliError> {
    WorkflowRunFileRepository::new(data_dir.to_path_buf())
        .list_runs(filter)
        .map_err(|e| CliError::Other(e.to_string()))
}

/// [05] API / CLI 等価性境界: Tauri 側 `canonicalize_managed_worktree_path`
/// と同じ `ManagedWorktreeGateway` を CLI 経路でも経由する。
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
    let gateway = RepoPathsManagedWorktreeGateway::new(
        std::sync::Arc::new(crate::adaptor::controller::wiring::build_repository_usecase()),
        repo_paths,
    );
    gateway
        .resolve(worktree_path)
        .map_err(|e| CliError::InvalidInput(e.to_string()))
}

/// Spec [05] API / CLI 等価性境界: Tauri `get_workflow_run_state` と同じ
/// `WorkflowStateView` shape を CLI 側でも返すため、再構築した `WorkflowState` を
/// `workflow_state_to_view` 経由で投影し、`runtime_states` 空 HashMap で
/// `WorkflowStateView::from_parts` に通す（CLI は engine の in-memory runtime を
/// 観測しない）。
fn reconstruct_state_view(data_dir: &Path, run_id: &str) -> Result<WorkflowStateView, CliError> {
    let run_id =
        RunId::new(run_id.to_string()).map_err(|e| CliError::InvalidInput(e.to_string()))?;
    let state = WorkflowStateProjectionLogRepository::new(data_dir.to_path_buf())
        .get_state(&run_id)
        .map_err(|e| CliError::Other(e.to_string()))?;
    let state = state
        .ok_or_else(|| CliError::NotFound(format!("No event log available for run: {run_id}")))?;
    Ok(WorkflowStateView::from_parts(
        workflow_state_to_view(state),
        std::collections::HashMap::new(),
    ))
}

fn event_draft_to_cli_log_json(event: &WorkflowEventDraft) -> serde_json::Value {
    let mut object = match event.payload.clone() {
        serde_json::Value::Object(object) => object,
        other => {
            let mut object = serde_json::Map::new();
            object.insert("payload".to_string(), other);
            object
        }
    };
    object.insert(
        "event".to_string(),
        serde_json::Value::String(event.event_kind.clone()),
    );
    object.insert(
        "run_id".to_string(),
        serde_json::Value::String(event.run_id.clone()),
    );
    object.insert("timestamp".to_string(), serde_json::json!(event.timestamp));
    serde_json::Value::Object(object)
}

fn format_event_draft(event: &WorkflowEventDraft) -> String {
    let kind = event_kind_display_name(&event.event_kind);
    let view = event_draft_to_cli_log_json(event);
    match serde_json::to_string(&view) {
        Ok(json) => format!("{kind} {json}"),
        Err(_) => kind.to_string(),
    }
}

fn event_kind_display_name(kind: &str) -> &str {
    match kind {
        "run_started" => "RunStarted",
        "node_started" => "NodeStarted",
        "step_session_started" => "StepSessionStarted",
        "node_completed" => "NodeCompleted",
        "node_failed" => "NodeFailed",
        "approval_requested" => "ApprovalRequested",
        "approval_resolved" => "ApprovalResolved",
        "run_completed" => "RunCompleted",
        "run_failed" => "RunFailed",
        "run_aborted" => "RunAborted",
        "output_collected" => "OutputCollected",
        "parallel_started" => "ParallelStarted",
        "parallel_child_started" => "ParallelChildStarted",
        "parallel_child_completed" => "ParallelChildCompleted",
        "parallel_completed" => "ParallelCompleted",
        "contract_repair_requested" => "ContractRepairRequested",
        "cli_mutation_requested" => "CliMutationRequested",
        "artifact_produced" => "ArtifactProduced",
        "cli_mutation_rejected" => "CliMutationRejected",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::super::common::test_support::{
        make_run, run_started_event, test_uuid, write_run_file,
    };
    use super::super::common::{
        cli_error_exit_code, cli_error_stderr, validate_optional_cli_text_len,
    };
    use super::super::workflow_io::{
        enqueue_pending_command, get_run_summary_file_direct, read_domain_log, CliRequestPayload,
    };
    use super::super::Cli;
    use super::*;
    use crate::adaptor::gateway::workflow::event::WorkflowEvent;
    use crate::adaptor::gateway::workflow::log::WorkflowEventLog;
    use crate::adaptor::gateway::workflow::pending_command::PendingCommandStore;
    use crate::adaptor::gateway::workflow::run::{RunStatus, TriggerSource, WorkflowRun};
    use crate::domain::workflow::approval_rules::MAX_APPROVAL_COMMENT_CHARS;
    use clap::Parser;
    use tempfile::TempDir;

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

        let all = list_runs_file_direct(
            tmp.path(),
            RunListFilter {
                status: None,
                worktree_path: None,
            },
        )
        .unwrap();
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
        )
        .unwrap();
        assert_eq!(active_only.len(), 1);
        assert_eq!(active_only[0].run_id, active_id);

        let terminal_only = list_runs_file_direct(
            tmp.path(),
            RunListFilter {
                status: Some(RunStatusFilter::Terminal),
                worktree_path: None,
            },
        )
        .unwrap();
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
        )
        .unwrap();
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
        assert_eq!(format!("{:?}", summary.status), "Running");
        assert_eq!(summary.started_at, 100.0);
    }

    fn write_event_log(data_dir: &Path, _run_id: &str, events: &[WorkflowEvent]) {
        let log = WorkflowEventLog::new(data_dir);
        for event in events {
            log.append(event).unwrap();
        }
    }

    fn seed_started_run(
        data_dir: &Path,
        run_id: &str,
        worktree: &str,
        status: RunStatus,
        started_at: f64,
    ) {
        write_run_file(data_dir, &make_run(run_id, worktree, status, started_at));
        write_event_log(
            data_dir,
            run_id,
            &[run_started_event(run_id, "wf", worktree)],
        );
    }

    #[test]
    fn workflow_handler_outputs_match_split_before_golden() {
        let tmp = TempDir::new().unwrap();
        let run_id = test_uuid(31);
        seed_started_run(tmp.path(), &run_id, "/wt/runs", RunStatus::Running, 100.0);

        let runs_human = cmd_runs(tmp.path(), None, None, false).unwrap();
        assert_eq!(
            runs_human,
            format!(
                "{:<36}  {:<20}  {:<18}  WORKTREE\n{:<36}  {:<20}  {:<18}  {}\n",
                "RUN_ID", "WORKFLOW", "STATUS", run_id, "wf", "Running", "/wt/runs"
            )
        );

        let runs_json = cmd_runs(tmp.path(), None, None, true).unwrap();
        assert_eq!(
            runs_json,
            format!(
                r#"[
  {{
    "runId": "{run_id}",
    "workflowName": "wf",
    "status": "running",
    "worktreePath": "/wt/runs",
    "triggerSource": "cli",
    "startedAt": 100.0,
    "updatedAt": 100.0
  }}
]
"#
            )
        );

        let status_human = cmd_status(tmp.path(), &run_id, false).unwrap();
        assert_eq!(
            status_human,
            format!(
                "run_id:        {run_id}\nworkflow:      wf\nstate:         Running\ncurrent_step:  \nupdated_at:    100\n"
            )
        );

        let status_json = cmd_status(tmp.path(), &run_id, true).unwrap();
        assert_eq!(
            status_json,
            format!(
                r#"{{
  "executionId": "{run_id}",
  "workflowName": "wf",
  "state": {{
    "type": "running"
  }},
  "currentStepIndex": 0,
  "currentStepName": "",
  "totalSteps": 0,
  "stepHistory": [],
  "stepExecutionCounts": {{}},
  "workflowDefinition": {{
    "name": "wf",
    "description": "test",
    "builtin": false,
    "nodes": []
  }},
  "totalTokenUsage": {{
    "inputTokens": 0,
    "outputTokens": 0
  }},
  "stepStates": {{}},
  "stepOutputs": {{
    "request": {{
      "stepName": "request",
      "runIndex": 0,
      "structuredOutput": "",
      "artifactContract": "string",
      "completedAt": 100.0
    }}
  }},
  "startedAt": 100.0,
  "updatedAt": 100.0
}}
"#
            )
        );

        let logs_human = cmd_logs(tmp.path(), &run_id, false).unwrap();
        assert_eq!(
            logs_human,
            format!(
                r#"RunStarted {{"event":"run_started","request":"","run_id":"{run_id}","timestamp":100.0,"workflow_definition":{{"builtin":false,"description":"test","name":"wf","nodes":[]}},"workflow_file_stem":"wf","workflow_name":"wf","worktree_path":"/wt/runs"}}
"#
            )
        );

        let logs_json = cmd_logs(tmp.path(), &run_id, true).unwrap();
        assert_eq!(
            logs_json,
            format!(
                r#"[
  {{
    "event": "run_started",
    "request": "",
    "run_id": "{run_id}",
    "timestamp": 100.0,
    "workflow_definition": {{
      "builtin": false,
      "description": "test",
      "name": "wf",
      "nodes": []
    }},
    "workflow_file_stem": "wf",
    "workflow_name": "wf",
    "worktree_path": "/wt/runs"
  }}
]
"#
            )
        );
    }

    #[test]
    fn workflow_handler_error_stderr_and_exit_codes_match_split_before_golden() {
        let tmp = TempDir::new().unwrap();

        let invalid_status =
            cmd_runs(tmp.path(), Some("paused".to_string()), None, false).unwrap_err();
        assert_eq!(
            cli_error_stderr(&invalid_status),
            "error: Invalid --status value: paused (expected: active | terminal)"
        );
        assert_eq!(cli_error_exit_code(&invalid_status), 2);

        let missing_run_id = test_uuid(99);
        let missing = cmd_status(tmp.path(), &missing_run_id, false).unwrap_err();
        assert_eq!(
            cli_error_stderr(&missing),
            format!("Workflow run not found: {missing_run_id}")
        );
        assert_eq!(cli_error_exit_code(&missing), 4);
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
    fn cli_log_json_view_matches_existing_event_wire_shape() {
        let tmp = TempDir::new().unwrap();
        let run_id = test_uuid(13);
        let legacy = run_started_event(&run_id, "wf", "/wt/cli-logs-shape");
        write_event_log(tmp.path(), &run_id, std::slice::from_ref(&legacy));

        let events = read_domain_log(tmp.path(), &run_id).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(
            event_draft_to_cli_log_json(&events[0]),
            serde_json::to_value(legacy).unwrap()
        );
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
        use crate::adaptor::gateway::workflow::event_projection::reconstruct_state_from_events;
        use crate::adaptor::gateway::workflow::run::RunStore;

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
        let cli_summaries = list_runs_file_direct(
            tmp.path(),
            RunListFilter {
                status: None,
                worktree_path: None,
            },
        )
        .unwrap();
        let cli_summary = get_run_summary_file_direct(tmp.path(), &run_id)
            .expect("CLI summary must be available");
        let cli_events = read_domain_log(tmp.path(), &run_id).unwrap();
        let cli_state_view = reconstruct_state_view(tmp.path(), &run_id).unwrap();

        // API 経路（RunStore は active in-memory map + workflow_runs/ file の両方を参照）
        let store = RunStore::default();
        store.set_data_dir(tmp.path().to_path_buf()).await;
        let api_summaries = store
            .list_runs(crate::adaptor::gateway::workflow::run::RunListFilter::default())
            .await;
        let api_summary = store
            .get_run(&run_id)
            .await
            .expect("API summary must be available");
        let api_events = WorkflowEventLog::new(tmp.path()).read_log(&run_id).unwrap();
        let api_state = reconstruct_state_from_events(&run_id, &api_events)
            .unwrap()
            .unwrap();
        let api_state_view = WorkflowStateView::from_parts(
            workflow_state_to_view(
                crate::adaptor::gateway::workflow::state::workflow_state_to_domain_snapshot(
                    api_state,
                ),
            ),
            std::collections::HashMap::new(),
        );

        // 並び順 + 件数の一致
        assert_eq!(api_summaries.len(), cli_summaries.len());
        for (a, c) in api_summaries.iter().zip(cli_summaries.iter()) {
            assert_eq!(a.run_id, c.run_id);
            assert_eq!(format!("{:?}", a.status), format!("{:?}", c.status));
            assert_eq!(a.worktree_path, c.worktree_path);
        }
        // 単一 summary の一致
        assert_eq!(api_summary.run_id, cli_summary.run_id);
        assert_eq!(
            format!("{:?}", api_summary.status),
            format!("{:?}", cli_summary.status)
        );
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
        use crate::adaptor::gateway::workflow::storage;

        let tmp = TempDir::new().unwrap();
        let workflows_dir = tmp.path().join("workflows");
        std::fs::create_dir_all(&workflows_dir).unwrap();
        let yaml = concat!(
            "name: api-cli-list\n",
            "description: list test\n",
            "nodes:\n",
            "  - name: step1\n",
            "    session:\n",
            "      permission: edit\n",
            "      facets:\n",
            "        instruction: do thing\n",
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
        let api_running = running_workflow_names_file_direct(tmp.path()).unwrap();
        for s in &mut api_summaries {
            s.is_running = api_running.contains(&s.name);
        }
        // 同じ projection に通したので JSON shape も一致する。
        let api_json = serde_json::to_value(&api_summaries).unwrap();
        let cli_json = serde_json::to_value(
            cli_summaries
                .clone()
                .into_iter()
                .map(mapper::domain_workflow_summary_to_legacy)
                .collect::<Vec<_>>(),
        )
        .unwrap();
        assert_eq!(api_json, cli_json);
    }

    /// Spec [05] API / CLI 等価性境界: list_workflows の API 経路は
    /// `engine.running_workflow_names()`（in-memory `executions` map 由来）を
    /// running source とし、CLI は repository file-direct の active run query
    /// （`workflow_runs/` file 由来）を使う。engine が active run を登録すると
    /// 両 source が同期して同一 running 集合を返すことを実 API 経路で検証する
    /// （spec L92-96 / L160-162）。
    #[tokio::test]
    async fn engine_running_workflow_names_matches_cli_file_direct_after_register_active() {
        use crate::adaptor::gateway::workflow::schema::{
            NodeDefinition, NodeKind, SessionSpec, Workflow,
        };
        use crate::adaptor::gateway::workflow::state::WorkflowExecutionState;
        use crate::adaptor::gateway::workflow::test_support::TestRuntimeKernel;

        let tmp = TempDir::new().unwrap();
        let engine = TestRuntimeKernel::new_for_test();
        engine
            .set_run_store_data_dir(tmp.path().to_path_buf())
            .await;

        let workflow = Workflow {
            name: "engine-cli-list".to_string(),
            description: String::new(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![NodeDefinition {
                name: "step1".to_string(),
                kind: NodeKind::Session(SessionSpec::default()),
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
        let cli_running = running_workflow_names_file_direct(tmp.path()).unwrap();
        assert_eq!(
            api_running, cli_running,
            "API path engine.running_workflow_names() must equal CLI file-direct set"
        );
        assert!(api_running.contains("engine-cli-list"));
    }

    /// Spec [05] API / CLI 等価性境界: CLI `--worktree` 入力は
    /// `ManagedWorktreeGateway` 経由で managed worktree 検証を
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
        let (repo_dir, repo) = crate::test_support::git::create_test_repo();
        crate::test_support::git::create_initial_commit(&repo);
        let worktree_parent = tempfile::TempDir::new().unwrap();
        let worktree_path = worktree_parent.path().join("managed-wt");
        repo.worktree("managed-wt", &worktree_path, None).unwrap();

        // CLI 用 data_dir に releash.toml を配置し、repo を configured repo として登録。
        let data_dir = tempfile::TempDir::new().unwrap();
        let config_path = data_dir.path().join("releash.toml");
        let mut config = crate::adaptor::gateway::app_config::ReleashConfig::default();
        config.app.last_repo_paths = vec![repo_dir.path().to_string_lossy().to_string()];
        crate::adaptor::gateway::app_config::repository_impl::write_config(&config_path, &config)
            .unwrap();

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
    /// inner 関数（`adaptor::controller::command::workflow::list_workflows_inner`）と CLI 側の
    /// `list_workflows_file_direct` を、同一 tempdir / 同一 running 集合に対して比較し、
    /// 両者が JSON shape として完全に一致することを境界仕様として担保する。
    ///
    /// engine.running_workflow_names()（in-memory）と
    /// repository file-direct の active run query は engine の同期書き込み
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
            "    session:\n",
            "      permission: edit\n",
            "      facets:\n",
            "        instruction: do thing\n",
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
        let running = running_workflow_names_file_direct(tmp.path()).unwrap();
        let api_summaries = crate::adaptor::controller::command::workflow::list_workflows_inner(
            &running,
            &workflows_dir,
        )
        .unwrap();

        // CLI 経路: list_workflows_file_direct
        let cli_summaries = list_workflows_file_direct(&workflows_dir, tmp.path()).unwrap();

        // 両者は同じ projection を通すので JSON shape も完全一致。
        let api_json = serde_json::to_value(&api_summaries).unwrap();
        let cli_json = serde_json::to_value(
            cli_summaries
                .clone()
                .into_iter()
                .map(mapper::domain_workflow_summary_to_legacy)
                .collect::<Vec<_>>(),
        )
        .unwrap();
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

    #[test]
    fn cmd_approve_handler_validates_and_writes_pending_payload() {
        let tmp = TempDir::new().unwrap();
        let run_id = test_uuid(84);
        write_run_file(
            tmp.path(),
            &make_run(&run_id, "/wt/approve-handler", RunStatus::Running, 100.0),
        );

        let stdout = cmd_approve(
            tmp.path(),
            &run_id,
            Some("review".to_string()),
            Some("LGTM".to_string()),
        )
        .unwrap();

        assert!(stdout.starts_with(&format!("queued: run_id={run_id} request_id=")));
        assert!(stdout.ends_with('\n'));
        let expected = CliRequestPayload::Approve {
            node_name: Some("review".to_string()),
            comment: Some("LGTM".to_string()),
        };
        let entries = PendingCommandStore::new(tmp.path()).list_pending().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            serde_json::to_value(&entries[0].command.payload).unwrap(),
            serde_json::to_value(&expected).unwrap()
        );
    }

    #[test]
    fn cmd_approve_handler_rejects_oversized_comment_before_enqueue() {
        let tmp = TempDir::new().unwrap();
        let run_id = test_uuid(85);
        write_run_file(
            tmp.path(),
            &make_run(&run_id, "/wt/approve-too-long", RunStatus::Running, 100.0),
        );
        let oversized = "x".repeat(MAX_APPROVAL_COMMENT_CHARS + 1);

        let err = cmd_approve(tmp.path(), &run_id, None, Some(oversized)).unwrap_err();

        assert!(matches!(err, CliError::InvalidInput(_)));
        assert!(PendingCommandStore::new(tmp.path())
            .list_pending()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn cmd_reject_handler_rejects_invalid_reason_before_enqueue() {
        let tmp = TempDir::new().unwrap();
        let run_id = test_uuid(86);
        write_run_file(
            tmp.path(),
            &make_run(&run_id, "/wt/reject-invalid", RunStatus::Running, 100.0),
        );

        let err = cmd_reject(tmp.path(), &run_id, None, "   ".to_string()).unwrap_err();

        assert!(matches!(err, CliError::InvalidInput(_)));
        assert!(PendingCommandStore::new(tmp.path())
            .list_pending()
            .unwrap()
            .is_empty());
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
        assert_eq!(
            serde_json::to_value(&entries[0].command.payload).unwrap(),
            serde_json::to_value(&payload).unwrap()
        );
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
        assert_eq!(
            serde_json::to_value(&entries[0].command.payload).unwrap(),
            serde_json::to_value(&payload).unwrap()
        );
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
        assert_eq!(
            serde_json::to_value(&entries[0].command.payload).unwrap(),
            serde_json::to_value(&payload).unwrap()
        );
    }

    /// [06] CLI 入力の信頼境界: `cmd_enqueue_pending` は run_id の UUID 形式を弾く。
    #[test]
    fn cmd_enqueue_pending_rejects_non_uuid_run_id() {
        let tmp = TempDir::new().unwrap();
        let payload = CliRequestPayload::Abort { node_name: None };
        let err = enqueue_pending_command(tmp.path(), "not-a-uuid", payload).unwrap_err();
        assert!(matches!(err, CliError::InvalidInput(_)));
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
