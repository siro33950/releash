use std::collections::HashSet;
use std::path::Path;

use clap::Subcommand;

use super::common::{truncate, validate_execution_id, validate_optional_cli_text_len, CliError};
use super::output::OutputSubcommand;
use super::workflow_io;
use crate::adaptor::gateway::app_config::read_config_if_exists;
use crate::adaptor::gateway::workflow::RepoPathsManagedWorktreeGateway;
use crate::adaptor::presenter::workflow::workflow_execution_to_view;
use crate::domain::workflow::{
    ExecutionListFilter, ExecutionStatusFilter, ManagedWorktreeGateway, WorkflowSummary,
};
use crate::usecase::workflow::dto::{workflow_execution_summary_to_dto, workflow_summary_to_dto};

/// `releash workflow` の observation / checkpoint command 集合。
#[derive(Subcommand, Debug)]
pub(super) enum WorkflowSubcommand {
    /// 利用可能な workflow definition 一覧を表示する。
    List {
        #[arg(long)]
        json: bool,
    },
    /// 過去 / 進行中の workflow execution 一覧を表示する。
    Executions {
        /// status filter: `active` または `terminal`。
        #[arg(long)]
        status: Option<String>,
        /// worktree path filter。
        #[arg(long)]
        worktree: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// 指定 execution の現在 read model を表示する。
    Status {
        execution_id: String,
        #[arg(long)]
        json: bool,
    },
    /// 指定 execution の event log を表示する。
    Logs {
        execution_id: String,
        #[arg(long)]
        json: bool,
    },
    /// approval checkpoint で待機中の NodeExecution を承認する。
    Approve {
        execution_id: String,
        #[arg(long)]
        node: String,
        #[arg(long = "node-execution", value_name = "NODE_EXECUTION_ID")]
        node_execution: Option<String>,
        #[arg(long)]
        comment: Option<String>,
    },
    /// 進行中の workflow execution または指定 node を中止する。
    Abort {
        execution_id: String,
        #[arg(long)]
        node: Option<String>,
    },
    /// node の Artifact に対する typed CLI 入口。
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
        let views = summaries
            .into_iter()
            .map(workflow_summary_to_dto)
            .collect::<Vec<_>>();
        let text = serde_json::to_string_pretty(&views)
            .map_err(|error| CliError::Other(format!("serialize workflows: {error}")))?;
        return Ok(format!("{text}\n"));
    }
    if summaries.is_empty() {
        return Ok("(no workflows)\n".to_string());
    }
    let mut output = String::new();
    for summary in summaries {
        let tag = if summary.builtin {
            "[builtin]"
        } else {
            "         "
        };
        let active = if summary.is_running { " (active)" } else { "" };
        output.push_str(&format!(
            "{tag} {:<32}  {}{active}\n",
            summary.name, summary.description
        ));
    }
    Ok(output)
}

pub(super) fn cmd_executions(
    data_dir: &Path,
    status: Option<String>,
    worktree: Option<String>,
    json: bool,
) -> Result<String, CliError> {
    let status = parse_status_filter(status.as_deref())?;
    let worktree_path = worktree
        .as_deref()
        .map(|path| canonicalize_cli_worktree_filter_path(data_dir, path))
        .transpose()?;
    let executions = workflow_io::file_direct_query_service(data_dir)
        .list_executions(ExecutionListFilter {
            status,
            worktree_path,
        })
        .map_err(|error| CliError::Other(error.to_string()))?;

    if json {
        let views = executions
            .into_iter()
            .map(workflow_execution_summary_to_dto)
            .collect::<Vec<_>>();
        let text = serde_json::to_string_pretty(&views)
            .map_err(|error| CliError::Other(format!("serialize executions: {error}")))?;
        return Ok(format!("{text}\n"));
    }
    if executions.is_empty() {
        return Ok("(no executions)\n".to_string());
    }
    let mut output = format!(
        "{:<36}  {:<20}  {:<18}  WORKTREE\n",
        "EXECUTION_ID", "WORKFLOW", "STATUS"
    );
    for execution in executions {
        output.push_str(&format!(
            "{:<36}  {:<20}  {:<18}  {}\n",
            execution.execution_id,
            truncate(&execution.workflow_name, 20),
            execution.status.as_str(),
            execution.worktree_path
        ));
    }
    Ok(output)
}

pub(super) fn cmd_status(
    data_dir: &Path,
    execution_id: &str,
    json: bool,
) -> Result<String, CliError> {
    validate_execution_id(execution_id)?;
    workflow_io::ensure_execution_exists(data_dir, execution_id)?;
    let execution = workflow_io::file_direct_query_service(data_dir)
        .get_execution_state(execution_id)
        .map_err(|error| CliError::Other(error.to_string()))?
        .ok_or_else(|| {
            CliError::NotFound(format!("Workflow execution log not found: {execution_id}"))
        })?;

    if json {
        let view = workflow_execution_to_view(execution);
        let text = serde_json::to_string_pretty(&view)
            .map_err(|error| CliError::Other(format!("serialize execution: {error}")))?;
        return Ok(format!("{text}\n"));
    }
    let current_node = execution.current_node.as_deref().unwrap_or("");
    Ok(format!(
        "execution_id:  {}\nworkflow:      {}\nstatus:        {}\ncurrent_node:  {}\nupdated_at:    {}\ninput_tokens:  {}\noutput_tokens: {}\n",
        execution.id,
        execution.workflow_name,
        execution.status.as_str(),
        current_node,
        execution.updated_at,
        execution.total_token_usage.input_tokens,
        execution.total_token_usage.output_tokens,
    ))
}

pub(super) fn cmd_logs(
    data_dir: &Path,
    execution_id: &str,
    json: bool,
) -> Result<String, CliError> {
    validate_execution_id(execution_id)?;
    workflow_io::ensure_execution_exists(data_dir, execution_id)?;
    let events = workflow_io::file_direct_query_service(data_dir)
        .get_execution_log(execution_id)
        .map_err(|error| CliError::Other(error.to_string()))?;
    if json {
        let text = serde_json::to_string_pretty(&events)
            .map_err(|error| CliError::Other(format!("serialize log: {error}")))?;
        return Ok(format!("{text}\n"));
    }
    let mut output = String::new();
    for event in events {
        let kind = event
            .get("event")
            .and_then(serde_json::Value::as_str)
            .map(event_kind_display_name)
            .unwrap_or_else(|| "WorkflowEvent".to_string());
        let json = serde_json::to_string(&event)
            .map_err(|error| CliError::Other(format!("serialize log event: {error}")))?;
        output.push_str(&format!("{kind} {json}\n"));
    }
    Ok(output)
}

pub(super) fn cmd_approve(
    data_dir: &Path,
    execution_id: &str,
    node: String,
    node_execution: Option<String>,
    comment: Option<String>,
) -> Result<String, CliError> {
    validate_optional_cli_text_len(comment.as_deref(), "--comment")?;
    enqueue_and_format(
        data_dir,
        execution_id,
        workflow_io::CliRequestPayload::Approve {
            node_name: node,
            node_execution_id: workflow_io::resolve_node_execution_id(node_execution),
            comment,
        },
    )
}

pub(super) fn cmd_abort(
    data_dir: &Path,
    execution_id: &str,
    node: Option<String>,
) -> Result<String, CliError> {
    enqueue_and_format(
        data_dir,
        execution_id,
        workflow_io::CliRequestPayload::Abort { node_name: node },
    )
}

fn enqueue_and_format(
    data_dir: &Path,
    execution_id: &str,
    payload: workflow_io::CliRequestPayload,
) -> Result<String, CliError> {
    let output = workflow_io::enqueue_pending_command(data_dir, execution_id, payload)?;
    Ok(format!("{}\n", output.format_stdout_line()))
}

fn parse_status_filter(value: Option<&str>) -> Result<Option<ExecutionStatusFilter>, CliError> {
    match value {
        None | Some("") => Ok(None),
        Some("active") => Ok(Some(ExecutionStatusFilter::Active)),
        Some("terminal") => Ok(Some(ExecutionStatusFilter::Terminal)),
        Some(other) => Err(CliError::InvalidInput(format!(
            "Invalid --status value: {other} (expected: active | terminal)"
        ))),
    }
}

fn list_workflows_file_direct(
    workflows_dir: &Path,
    data_dir: &Path,
) -> Result<Vec<WorkflowSummary>, CliError> {
    let query = workflow_io::file_direct_query_service_with_workflows(data_dir, workflows_dir);
    let active_names = query
        .list_executions(ExecutionListFilter {
            status: Some(ExecutionStatusFilter::Active),
            worktree_path: None,
        })
        .map_err(|error| CliError::Other(error.to_string()))?
        .into_iter()
        .map(|execution| execution.workflow_name)
        .collect::<HashSet<_>>();
    query
        .list_workflows(&active_names.into_iter().collect::<Vec<_>>())
        .map_err(|error| CliError::Other(error.to_string()))
}

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
    RepoPathsManagedWorktreeGateway::new(
        std::sync::Arc::new(crate::adaptor::controller::wiring::build_repository_usecase()),
        repo_paths,
    )
    .resolve(worktree_path)
    .map_err(|error| CliError::InvalidInput(error.to_string()))
}

fn event_kind_display_name(kind: &str) -> String {
    kind.split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
                None => String::new(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::common::test_support::{
        execution_started_event, make_execution, test_uuid, write_execution_file,
    };
    use super::super::Cli;
    use super::*;
    use crate::adaptor::gateway::workflow::log::WorkflowEventLog;
    use crate::domain::workflow::{ExecutionOrigin, ExecutionStatus, TokenUsage};
    use clap::{CommandFactory, Parser};
    use tempfile::TempDir;

    fn seed_execution(data_dir: &Path, execution_id: &str, status: ExecutionStatus) {
        write_execution_file(
            data_dir,
            &make_execution(execution_id, "/repo", status, 100.0),
        );
        WorkflowEventLog::new(data_dir)
            .append(&execution_started_event(execution_id, "wf", "/repo"))
            .unwrap();
    }

    #[test]
    fn execution_commands_parse_with_canonical_vocabulary() {
        let execution_id = "550e8400-e29b-41d4-a716-446655440000";
        for argv in [
            vec!["releash", "workflow", "executions"],
            vec!["releash", "workflow", "executions", "--status", "active"],
            vec!["releash", "workflow", "status", execution_id, "--json"],
            vec!["releash", "workflow", "logs", execution_id],
            vec![
                "releash",
                "workflow",
                "approve",
                execution_id,
                "--node",
                "review",
            ],
        ] {
            assert!(Cli::try_parse_from(argv).is_ok());
        }
    }

    #[test]
    fn workflow_help_contains_no_legacy_execution_or_node_terms() {
        fn collect_help(command: &clap::Command, output: &mut String) {
            let mut command_for_render = command.clone();
            output.push_str(&command_for_render.render_long_help().to_string());
            for child in command.get_subcommands() {
                collect_help(child, output);
            }
        }

        let command = Cli::command();
        let mut help = String::new();
        collect_help(&command, &mut help);
        let forbidden_collection = ["ru", "ns"].concat();
        let forbidden_identifier = ["run", "_id"].concat();
        let forbidden_node = ["--st", "ep"].concat();
        for forbidden in [forbidden_collection, forbidden_identifier, forbidden_node] {
            assert!(
                !help.contains(&forbidden),
                "forbidden help term: {forbidden}"
            );
        }
        let forbidden_subject = ["r", "un"].concat();
        assert!(!help
            .split(|character: char| !character.is_ascii_alphanumeric())
            .any(|word| word == forbidden_subject));
    }

    #[test]
    fn execution_list_and_status_emit_canonical_views() {
        let temp = TempDir::new().unwrap();
        let execution_id = test_uuid(1);
        seed_execution(temp.path(), &execution_id, ExecutionStatus::Running);

        let listed = cmd_executions(temp.path(), None, None, true).unwrap();
        let listed: serde_json::Value = serde_json::from_str(&listed).unwrap();
        assert_eq!(listed[0]["executionId"], execution_id);
        assert_eq!(listed[0]["status"], "running");

        let status = cmd_status(temp.path(), &execution_id, true).unwrap();
        let status: serde_json::Value = serde_json::from_str(&status).unwrap();
        assert_eq!(status["id"], execution_id);
        assert_eq!(status["status"], "running");
        assert_eq!(status["artifacts"][0]["nodeName"], "request");
        assert!(status.get("state").is_none());
    }

    #[test]
    fn log_output_uses_execution_event_schema() {
        let temp = TempDir::new().unwrap();
        let execution_id = test_uuid(2);
        seed_execution(temp.path(), &execution_id, ExecutionStatus::Running);

        let json = cmd_logs(temp.path(), &execution_id, true).unwrap();
        let json: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(json[0]["event"], "execution_started");
        assert_eq!(json[0]["execution_id"], execution_id);

        let human = cmd_logs(temp.path(), &execution_id, false).unwrap();
        assert!(human.starts_with("ExecutionStarted "));
    }

    #[test]
    fn cli_and_tauri_surfaces_read_the_same_query_projection() {
        let temp = TempDir::new().unwrap();
        let execution_id = test_uuid(3);
        seed_execution(temp.path(), &execution_id, ExecutionStatus::Running);

        let cli = workflow_io::file_direct_query_service(temp.path())
            .get_execution_state(&execution_id)
            .unwrap()
            .unwrap();
        let tauri = crate::adaptor::controller::wiring::build_workflow_usecase(temp.path())
            .get_execution_state(&execution_id)
            .unwrap()
            .unwrap();

        assert_eq!(cli, tauri);
        assert_eq!(
            serde_json::to_value(workflow_execution_to_view(cli)).unwrap(),
            serde_json::to_value(workflow_execution_to_view(tauri)).unwrap()
        );
    }

    #[test]
    fn execution_summary_json_matches_tauri_summary_dto() {
        let temp = TempDir::new().unwrap();
        let execution_id = test_uuid(4);
        let mut execution = make_execution(
            &execution_id,
            "/repo",
            ExecutionStatus::WaitingApproval,
            5.0,
        );
        execution.current_node = Some("review".to_string());
        execution.created_from = ExecutionOrigin::Agent;
        execution.total_token_usage = TokenUsage {
            input_tokens: 8,
            output_tokens: 3,
        };
        write_execution_file(temp.path(), &execution);

        let service = workflow_io::file_direct_query_service(temp.path());
        let summary: crate::domain::workflow::WorkflowExecutionSummary =
            service.get_execution(&execution_id).unwrap().unwrap();
        let expected = serde_json::to_value(workflow_execution_summary_to_dto(summary)).unwrap();
        let actual: serde_json::Value =
            serde_json::from_str(&cmd_executions(temp.path(), None, None, true).unwrap()).unwrap();
        assert_eq!(actual[0], expected);
    }

    #[test]
    fn missing_execution_is_reported_without_creating_state() {
        let temp = TempDir::new().unwrap();
        let execution_id = test_uuid(5);
        let error = cmd_status(temp.path(), &execution_id, false).unwrap_err();
        assert_eq!(
            error,
            CliError::NotFound(format!("Workflow execution not found: {execution_id}"))
        );
    }

    #[test]
    fn invalid_filter_is_rejected() {
        let temp = TempDir::new().unwrap();
        assert!(matches!(
            cmd_executions(temp.path(), Some("paused".to_string()), None, false),
            Err(CliError::InvalidInput(_))
        ));
    }
}
