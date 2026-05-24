//! [06] CLI pending command の dispatcher adapter。
//!
//! file-direct payload の解釈と `WorkflowCommand` への変換を engine から分離し、
//! engine には typed command と mutation route context だけを渡す。

use std::sync::Arc;

use tokio::sync::Mutex;

use crate::agent_sdk::AgentProcessMap;
use crate::session::SessionStore;
use crate::workflow::command::{WorkflowCommand, WorkflowCommandResult};
use crate::workflow::engine::{WorkflowEngine, WorkflowEngineError};
use crate::workflow::event::CliMutationRequestRecord;
use crate::workflow::pending_command::{
    PendingCommand, PendingCommandEntry, PendingCommandPayload, PendingCommandStore,
};
use crate::workflow::route_context::{
    CommandCommitContext, WorkflowMutationContext, WorkflowMutationSource,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PendingCommandDispatchOutcome {
    Accepted,
    RejectedFinal(String),
    RetryableFailure(String),
}

pub(crate) async fn process_pending_command_entry<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    engine: &Arc<WorkflowEngine>,
    session_store: &Arc<SessionStore>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    store: &PendingCommandStore,
    entry: PendingCommandEntry,
) {
    let entry_id = entry.command.id.clone();
    let run_id = entry.command.run_id.clone();
    let claimed = match store.claim_pending(&entry) {
        Ok(Some(claimed)) => claimed,
        Ok(None) => return,
        Err(e) => {
            log::warn!("pending command claim failed: id={entry_id} run_id={run_id} reason={e}");
            return;
        }
    };

    match dispatch_pending_command(
        app,
        engine,
        session_store,
        handles,
        claimed.entry.command.clone(),
    )
    .await
    {
        PendingCommandDispatchOutcome::Accepted => {
            log::info!("pending command dispatched: id={entry_id} run_id={run_id}");
            if let Err(e) = store.mark_processed(&claimed.entry) {
                log::warn!(
                    "Failed to mark pending command processed: id={entry_id} run_id={run_id} reason={e}"
                );
            }
        }
        PendingCommandDispatchOutcome::RejectedFinal(reason) => {
            log::warn!(
                "pending command dispatch rejected: id={entry_id} run_id={run_id} reason={reason}"
            );
            if let Err(e) = store.mark_processed(&claimed.entry) {
                log::warn!(
                    "Failed to mark rejected pending command processed: id={entry_id} run_id={run_id} reason={e}"
                );
            }
        }
        PendingCommandDispatchOutcome::RetryableFailure(reason) => {
            log::warn!(
                "pending command dispatch retryable failure: id={entry_id} run_id={run_id} reason={reason}"
            );
            if let Err(e) = store.release_claim(&claimed.entry) {
                log::warn!(
                    "Failed to release pending command claim: id={entry_id} run_id={run_id} reason={e}"
                );
            }
        }
    }
}

pub(crate) async fn dispatch_pending_command<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    engine: &Arc<WorkflowEngine>,
    session_store: &Arc<SessionStore>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    pending: PendingCommand,
) -> PendingCommandDispatchOutcome {
    if uuid::Uuid::parse_str(&pending.run_id).is_err() {
        return PendingCommandDispatchOutcome::RejectedFinal(
            "pending command run_id must be UUID".to_string(),
        );
    }

    let PendingCommand {
        run_id,
        id,
        payload,
        requested_at,
        ..
    } = pending;

    let is_submit_output = matches!(&payload, PendingCommandPayload::SubmitOutput { .. });

    // [08] CLI 提出経路と in-process 経路は dispatch_external で合流する。
    // CLI pending の SubmitOutput は CliMutationRequested を伴わず OutputSubmitted 単体で
    // 記録されるため commit_context は `SubmitOutput { request_id, submitted_at }` を運ぶ。
    // 他の CLI mutation（Approve / Reject / Abort）は従来通り `CliPending` を運ぶ。
    let (commit_context, request_id) = if is_submit_output {
        // 5-3 修正: SubmitOutput の拒否時にも `CliMutationRejected` を補助履歴として
        // 残せるよう、step_name と contract を commit_context に保持する。
        let (step_name, contract) = match &payload {
            PendingCommandPayload::SubmitOutput {
                step_name,
                contract,
                ..
            } => (step_name.clone(), contract.clone()),
            _ => unreachable!("is_submit_output guard guarantees SubmitOutput variant"),
        };
        (
            CommandCommitContext::submit_output(id.clone(), requested_at, step_name, contract),
            id.clone(),
        )
    } else {
        let request = payload_to_cli_request(&payload);
        let metadata = WorkflowMutationContext::new(
            run_id.clone(),
            WorkflowMutationSource::CliPendingCommand {
                request_id: id.clone(),
            },
            request,
            requested_at,
        );
        (CommandCommitContext::cli_pending(metadata), id.clone())
    };
    let command = payload_to_workflow_command(payload, run_id.clone());

    let already_recorded = if is_submit_output {
        engine.output_submitted_already_recorded(app, &run_id, &request_id)
    } else {
        engine.cli_mutation_already_recorded(app, &run_id, &request_id)
    };
    match already_recorded {
        Ok(true) => return PendingCommandDispatchOutcome::Accepted,
        Ok(false) => {}
        Err(e) => return classify_dispatch_error(e),
    }

    if let Err(e) = engine
        .ensure_execution_loaded_for_external(app, session_store, &run_id)
        .await
    {
        return handle_rejected_dispatch(app, engine, e, commit_context, &run_id, is_submit_output)
            .await;
    }

    match engine
        .dispatch_external_with_commit_context(
            app,
            session_store,
            handles,
            command,
            commit_context.clone(),
        )
        .await
    {
        Ok(WorkflowCommandResult::Accepted) => PendingCommandDispatchOutcome::Accepted,
        Ok(other) => PendingCommandDispatchOutcome::RejectedFinal(format!(
            "CLI mutation dispatch returned unexpected result: {other:?}"
        )),
        Err(e) => {
            handle_rejected_dispatch(app, engine, e, commit_context, &run_id, is_submit_output)
                .await
        }
    }
}

async fn handle_rejected_dispatch<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    engine: &WorkflowEngine,
    error: WorkflowEngineError,
    commit_context: CommandCommitContext,
    run_id: &str,
    is_submit_output: bool,
) -> PendingCommandDispatchOutcome {
    let reason = error.to_string();
    let should_commit = WorkflowEngine::should_commit_rejected_external_request(&error);
    // [06] spec [08] Rule 1 維持: SubmitOutput は accepted のメイン履歴
    // （`OutputSubmitted` / `CliMutationRequested`）に拒否事実を残さない。
    // 一方、それ以外の CLI mutation（Approve / Reject / Abort）は従来通り
    // `CliMutationRequested` を記録する。
    if !is_submit_output && should_commit {
        if let Err(record_error) = engine
            .append_command_commit_context(app, commit_context.clone())
            .await
        {
            return classify_dispatch_error(record_error);
        }
    }
    // 5-3 / 5-4 修正: 全 mutation 種別について engine 判断による拒否事実を
    // `CliMutationRejected` event として補助履歴に追記する。spec [08] Rule 1
    // の意味は「accepted のメイン履歴に出ない」と再定義し、本 event は観測
    // 経路用の補助履歴として並列に存在する。
    if should_commit {
        let append_result = if is_submit_output {
            engine
                .append_cli_mutation_rejected_for_submit_output(
                    app,
                    run_id,
                    &commit_context,
                    &error,
                )
                .await
        } else {
            engine
                .append_cli_mutation_rejected(app, &commit_context, &error)
                .await
        };
        if let Err(record_error) = append_result {
            return classify_dispatch_error(record_error);
        }
    }
    match classify_dispatch_error(error) {
        PendingCommandDispatchOutcome::RejectedFinal(_) => {
            PendingCommandDispatchOutcome::RejectedFinal(reason)
        }
        other => other,
    }
}

fn classify_dispatch_error(error: WorkflowEngineError) -> PendingCommandDispatchOutcome {
    match error {
        WorkflowEngineError::SessionStore(_) | WorkflowEngineError::AgentSession(_) => {
            PendingCommandDispatchOutcome::RetryableFailure(error.to_string())
        }
        other => PendingCommandDispatchOutcome::RejectedFinal(other.to_string()),
    }
}

fn payload_to_cli_request(payload: &PendingCommandPayload) -> CliMutationRequestRecord {
    match payload {
        PendingCommandPayload::Approve { node_name, comment } => {
            CliMutationRequestRecord::Approve {
                node_name: node_name.clone(),
                comment: comment.clone(),
            }
        }
        PendingCommandPayload::Reject { node_name, reason } => CliMutationRequestRecord::Reject {
            node_name: node_name.clone(),
            reason: reason.clone(),
        },
        PendingCommandPayload::Abort { node_name } => CliMutationRequestRecord::Abort {
            node_name: node_name.clone(),
        },
        PendingCommandPayload::SubmitOutput { .. } => {
            unreachable!(
                "SubmitOutput payload must use CommandCommitContext::SubmitOutput, not CliMutationRequestRecord"
            )
        }
    }
}

/// pending payload → typed `WorkflowCommand` への純変換。
///
/// 自由記述テキストの境界バリデーション（reject reason 非空 / 文字数上限）は
/// engine 受理時の `validate_approval_decision` / `validate_approve_comment_length`
/// に委ね、本 adapter では事前検証を行わない（review R2-01: ドメイン pure helper
/// 集約に伴う dispatcher 重複検証の削除）。
fn payload_to_workflow_command(payload: PendingCommandPayload, run_id: String) -> WorkflowCommand {
    match payload {
        PendingCommandPayload::Approve { node_name, comment } => WorkflowCommand::ApproveNode {
            run_id,
            node_name,
            comment,
        },
        PendingCommandPayload::Reject { node_name, reason } => WorkflowCommand::RejectNode {
            run_id,
            node_name,
            reason,
        },
        PendingCommandPayload::Abort { node_name } => WorkflowCommand::AbortRun {
            run_id,
            expected_node_name: node_name,
        },
        PendingCommandPayload::SubmitOutput {
            step_name,
            contract,
            structured_output,
        } => WorkflowCommand::SubmitOutput {
            run_id,
            step_name,
            contract,
            structured_output,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::event::{ApprovalDecisionRecord, WorkflowEvent};
    use crate::workflow::log::WorkflowEventLog;
    use crate::workflow::pending_command::CliRequestPayload;
    use crate::workflow::run::TriggerSource;
    use crate::workflow::schema::{NodeDefinition, NodeType, TransitionRule, Workflow};
    use crate::workflow::state::WorkflowExecutionState;
    use tempfile::TempDir;

    type DispatchTestApp = tauri::App<tauri::test::MockRuntime>;

    fn make_dispatch_app() -> DispatchTestApp {
        let mut config = crate::config::ReleashConfig::default();
        config.app.last_repo_paths = Vec::new();
        config.agents.codex.models = vec!["default".to_string(), "gpt-5.5".to_string()];
        config.agents.default = Some("codex".to_string());
        let app_config = Arc::new(crate::config::AppConfig::new(
            config,
            TempDir::new().unwrap().path().join("config.toml"),
        ));
        let registry = Arc::new(crate::backends::build_registry(Arc::clone(&app_config)));
        let data_dir = std::env::temp_dir().join(format!(
            "releash-pending-dispatcher-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&data_dir).unwrap();
        tauri::test::mock_builder()
            .manage(crate::session::TestDataDir(data_dir))
            .manage(app_config)
            .manage(registry)
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("tauri mock test app must build")
    }

    fn dispatch_data_dir(app: &tauri::AppHandle<tauri::test::MockRuntime>) -> std::path::PathBuf {
        crate::session::resolve_data_dir(app).expect("mock app data dir must resolve")
    }

    fn make_dispatch_deps() -> (
        Arc<crate::session::SessionStore>,
        Arc<Mutex<AgentProcessMap>>,
    ) {
        (
            Arc::new(crate::session::SessionStore::default()),
            Arc::new(Mutex::new(AgentProcessMap::new())),
        )
    }

    fn make_approval_only_workflow() -> Workflow {
        Workflow {
            name: "boundary-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            nodes: vec![NodeDefinition {
                name: "review".to_string(),
                node_type: NodeType::Approval,
                instruction: Some("review".to_string()),
                ..NodeDefinition::default()
            }],
        }
    }

    fn make_submit_output_workflow() -> Workflow {
        Workflow {
            name: "boundary-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            nodes: vec![NodeDefinition {
                name: "review".to_string(),
                node_type: NodeType::Agent,
                instruction: Some("review".to_string()),
                output_contract: Some("review-verdict".to_string()),
                ..NodeDefinition::default()
            }],
        }
    }

    async fn seed_running_agent(
        engine: &WorkflowEngine,
        run_id: &str,
        worktree_path: &str,
        workflow: Workflow,
    ) {
        engine
            .seed_active_execution_for_test(
                run_id.to_string(),
                workflow,
                crate::workflow::state::WorkflowExecutionState::Running,
                worktree_path.to_string(),
                TriggerSource::Cli,
            )
            .await;
    }

    fn make_rejectable_approval_workflow() -> Workflow {
        Workflow {
            name: "boundary-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            nodes: vec![
                NodeDefinition {
                    name: "review".to_string(),
                    node_type: NodeType::Approval,
                    instruction: Some("review".to_string()),
                    transition_rules: vec![TransitionRule {
                        r#match: "reject".to_string(),
                        next: "fix".to_string(),
                    }],
                    ..NodeDefinition::default()
                },
                NodeDefinition {
                    name: "fix".to_string(),
                    node_type: NodeType::Agent,
                    instruction: Some("fix".to_string()),
                    ..NodeDefinition::default()
                },
            ],
        }
    }

    async fn seed_waiting_approval(
        engine: &WorkflowEngine,
        run_id: &str,
        worktree_path: &str,
        workflow: Workflow,
    ) {
        engine
            .seed_active_execution_for_test(
                run_id.to_string(),
                workflow,
                WorkflowExecutionState::WaitingApproval,
                worktree_path.to_string(),
                TriggerSource::DesktopUi,
            )
            .await;
    }

    fn read_dispatch_events(app: &DispatchTestApp, run_id: &str) -> Vec<WorkflowEvent> {
        let data_dir = dispatch_data_dir(app.handle());
        WorkflowEventLog::new(&data_dir)
            .read_log(run_id)
            .unwrap_or_default()
    }

    #[test]
    fn payload_to_workflow_command_preserves_omitted_approve_node_for_engine_resolution() {
        let payload = CliRequestPayload::Approve {
            node_name: None,
            comment: None,
        };
        let cmd = payload_to_workflow_command(payload.clone(), "run-1".to_string());
        let request = payload_to_cli_request(&payload);
        match cmd {
            WorkflowCommand::ApproveNode { node_name, .. } => assert!(node_name.is_none()),
            other => panic!("expected ApproveNode, got: {other:?}"),
        }
        assert_eq!(
            request,
            CliMutationRequestRecord::Approve {
                node_name: None,
                comment: None
            }
        );
    }

    // 注: かつて dispatcher 層で事前バリデーション（reject reason 非空 /
    // comment 文字数上限）を行っていた `payload_to_workflow_command_rejects_*`
    // テストは、review R2-01（ドメインルールの 3 層重複解消）に伴い削除済み。
    // 同等の境界バリデーションは `engine::validate_approval_decision` /
    // `engine::validate_approve_comment_length` で担保されており、engine 側
    // テスト（`validate_approval_decision_reject_*` / `approve_comment_length_*`）
    // が引き続きカバーする。dispatch 全体の RejectedFinal 経路は
    // `process_pending_entry_marks_final_reject_processed_once` でカバー済み。

    #[tokio::test]
    async fn dispatch_pending_approve_records_cli_request_after_engine_acceptance() {
        let app = make_dispatch_app();
        let engine = Arc::new(WorkflowEngine::new_for_test());
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_run_store_data_dir(data_dir).await;
        let (session_store, handles) = make_dispatch_deps();
        let run_id = uuid::Uuid::new_v4().to_string();
        seed_waiting_approval(
            &engine,
            &run_id,
            "/wt/pending-dispatcher-approve",
            make_approval_only_workflow(),
        )
        .await;

        let pending = PendingCommand::new(
            run_id.clone(),
            CliRequestPayload::Approve {
                node_name: None,
                comment: Some("cli-lgtm".to_string()),
            },
            900.0,
        );

        let result =
            dispatch_pending_command(app.handle(), &engine, &session_store, &handles, pending)
                .await;
        assert_eq!(result, PendingCommandDispatchOutcome::Accepted);

        let events = read_dispatch_events(&app, &run_id);
        assert!(matches!(
            events.as_slice(),
            [
                WorkflowEvent::ApprovalResolved {
                    decision: ApprovalDecisionRecord::Approve,
                    ..
                },
                WorkflowEvent::NodeCompleted { .. },
                WorkflowEvent::RunCompleted { .. },
                WorkflowEvent::CliMutationRequested {
                    request: CliMutationRequestRecord::Approve {
                        node_name: None,
                        comment: Some(comment),
                    },
                    requested_at,
                    ..
                },
            ] if comment == "cli-lgtm" && (*requested_at - 900.0).abs() < f64::EPSILON
        ));
    }

    #[tokio::test]
    async fn process_pending_entry_marks_final_reject_processed_once() {
        let app = make_dispatch_app();
        let engine = Arc::new(WorkflowEngine::new_for_test());
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_run_store_data_dir(data_dir.clone()).await;
        let (session_store, handles) = make_dispatch_deps();
        let run_id = uuid::Uuid::new_v4().to_string();
        seed_waiting_approval(
            &engine,
            &run_id,
            "/wt/pending-dispatcher-final-reject",
            make_approval_only_workflow(),
        )
        .await;

        let store = PendingCommandStore::new(&data_dir);
        let pending = PendingCommand::new(
            run_id.clone(),
            CliRequestPayload::Approve {
                node_name: Some("stale-review".to_string()),
                comment: None,
            },
            900.75,
        );
        store.write_pending(&pending).unwrap();
        let entry = store.list_pending().unwrap().pop().unwrap();

        process_pending_command_entry(
            app.handle(),
            &engine,
            &session_store,
            &handles,
            &store,
            entry.clone(),
        )
        .await;

        assert!(store.list_pending().unwrap().is_empty());
        let events = read_dispatch_events(&app, &run_id);
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, WorkflowEvent::CliMutationRequested { .. }))
                .count(),
            1
        );

        process_pending_command_entry(
            app.handle(),
            &engine,
            &session_store,
            &handles,
            &store,
            entry,
        )
        .await;
        let after = read_dispatch_events(&app, &run_id);
        assert_eq!(
            after
                .iter()
                .filter(|event| matches!(event, WorkflowEvent::CliMutationRequested { .. }))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn process_pending_entry_releases_retryable_failure_back_to_pending() {
        let app = make_dispatch_app();
        let engine = Arc::new(WorkflowEngine::new_for_test());
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_run_store_data_dir(data_dir.clone()).await;
        let (session_store, handles) = make_dispatch_deps();
        let run_id = uuid::Uuid::new_v4().to_string();
        seed_waiting_approval(
            &engine,
            &run_id,
            "/wt/pending-dispatcher-retryable",
            make_approval_only_workflow(),
        )
        .await;

        let store = PendingCommandStore::new(&data_dir);
        let pending = PendingCommand::new(
            run_id.clone(),
            CliRequestPayload::Approve {
                node_name: Some("review".to_string()),
                comment: None,
            },
            901.75,
        );
        store.write_pending(&pending).unwrap();
        let entry = store.list_pending().unwrap().pop().unwrap();
        engine.fail_next_required_event_append_for_test();

        process_pending_command_entry(
            app.handle(),
            &engine,
            &session_store,
            &handles,
            &store,
            entry,
        )
        .await;

        let entries = store.list_pending().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].command.id, pending.id);
    }

    #[tokio::test]
    async fn dispatch_pending_reject_preserves_cli_reason_but_redacts_approval_event_comment() {
        let app = make_dispatch_app();
        let engine = Arc::new(WorkflowEngine::new_for_test());
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_run_store_data_dir(data_dir).await;
        let (session_store, handles) = make_dispatch_deps();
        let run_id = uuid::Uuid::new_v4().to_string();
        seed_waiting_approval(
            &engine,
            &run_id,
            "/wt/pending-dispatcher-reject-secret",
            make_rejectable_approval_workflow(),
        )
        .await;

        let raw_reason = "reject because password=secret123".to_string();
        let pending = PendingCommand::new(
            run_id.clone(),
            CliRequestPayload::Reject {
                node_name: None,
                reason: raw_reason.clone(),
            },
            905.0,
        );

        let result =
            dispatch_pending_command(app.handle(), &engine, &session_store, &handles, pending)
                .await;
        assert_eq!(result, PendingCommandDispatchOutcome::Accepted);

        let events = read_dispatch_events(&app, &run_id);
        let approval_comment = events
            .iter()
            .find_map(|event| match event {
                WorkflowEvent::ApprovalResolved { comment, .. } => comment.as_deref(),
                _ => None,
            })
            .expect("ApprovalResolved comment must be recorded");
        assert_eq!(approval_comment, "reject because password=[REDACTED]");

        let cli_reason = events
            .iter()
            .find_map(|event| match event {
                WorkflowEvent::CliMutationRequested {
                    request: CliMutationRequestRecord::Reject { reason, .. },
                    ..
                } => Some(reason.as_str()),
                _ => None,
            })
            .expect("CliMutationRequested reject reason must be recorded");
        assert_eq!(cli_reason, raw_reason);
    }

    /// [08] CLI pending 経由の SubmitOutput が engine の handle_submit_output
    /// に合流し、`OutputSubmitted` event が caller の `request_id` と `submitted_at`
    /// を保持する形で append される（spec [08] CLI 経路と in-process 経路の合流境界）。
    #[tokio::test]
    async fn dispatch_pending_submit_output_appends_event_with_caller_metadata() {
        let app = make_dispatch_app();
        let engine = Arc::new(WorkflowEngine::new_for_test());
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_run_store_data_dir(data_dir).await;
        let (session_store, handles) = make_dispatch_deps();
        let run_id = uuid::Uuid::new_v4().to_string();
        seed_running_agent(
            &engine,
            &run_id,
            "/wt/pending-dispatcher-submit",
            make_submit_output_workflow(),
        )
        .await;

        let pending = PendingCommand::new(
            run_id.clone(),
            CliRequestPayload::SubmitOutput {
                step_name: "review".to_string(),
                contract: "review-verdict".to_string(),
                structured_output: serde_json::json!({"verdict": "LGTM"}),
            },
            950.5,
        );
        let pending_id = pending.id.clone();

        let result =
            dispatch_pending_command(app.handle(), &engine, &session_store, &handles, pending)
                .await;
        assert_eq!(result, PendingCommandDispatchOutcome::Accepted);

        let events = read_dispatch_events(&app, &run_id);
        let submitted = events
            .iter()
            .find_map(|event| match event {
                WorkflowEvent::OutputSubmitted {
                    node_name,
                    contract,
                    request_id,
                    submitted_at,
                    ..
                } if node_name == "review" => {
                    Some((contract.clone(), request_id.clone(), *submitted_at))
                }
                _ => None,
            })
            .expect("OutputSubmitted event must be appended via dispatcher");
        assert_eq!(submitted.0, "review-verdict");
        assert_eq!(submitted.1.as_deref(), Some(pending_id.as_str()));
        assert_eq!(submitted.2, Some(950.5));
    }

    /// [08] CLI pending 経由の SubmitOutput が contract 不適合の場合、event は残らず、
    /// dispatcher は `RejectedFinal` を返す（spec [08] 振る舞い定義 Rule 1 適合しない場合）。
    ///
    /// 5-3 修正: `OutputSubmitted` / `CliMutationRequested` は引き続き残らないが、
    /// 観測経路用の補助履歴として `CliMutationRejected` event が追記される。
    #[tokio::test]
    async fn dispatch_pending_submit_output_rejects_invalid_contract() {
        let app = make_dispatch_app();
        let engine = Arc::new(WorkflowEngine::new_for_test());
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_run_store_data_dir(data_dir).await;
        let (session_store, handles) = make_dispatch_deps();
        let run_id = uuid::Uuid::new_v4().to_string();
        seed_running_agent(
            &engine,
            &run_id,
            "/wt/pending-dispatcher-submit-invalid",
            make_submit_output_workflow(),
        )
        .await;

        let pending = PendingCommand::new(
            run_id.clone(),
            CliRequestPayload::SubmitOutput {
                step_name: "review".to_string(),
                contract: "review-verdict".to_string(),
                structured_output: serde_json::json!({"verdict": "MAYBE"}),
            },
            960.0,
        );
        let pending_id = pending.id.clone();

        let result =
            dispatch_pending_command(app.handle(), &engine, &session_store, &handles, pending)
                .await;
        assert!(matches!(
            result,
            PendingCommandDispatchOutcome::RejectedFinal(_)
        ));

        let events = read_dispatch_events(&app, &run_id);
        // accepted のメイン履歴には出ない（spec [08] Rule 1 維持）。
        assert!(events
            .iter()
            .all(|event| !matches!(event, WorkflowEvent::OutputSubmitted { .. })));
        assert!(events
            .iter()
            .all(|event| !matches!(event, WorkflowEvent::CliMutationRequested { .. })));
        // 5-3 修正: 補助履歴として CliMutationRejected が追記される。
        let rejected = events
            .iter()
            .find_map(|event| match event {
                WorkflowEvent::CliMutationRejected {
                    request,
                    request_id,
                    requested_at,
                    ..
                } => Some((request.clone(), request_id.clone(), *requested_at)),
                _ => None,
            })
            .expect("CliMutationRejected event must be appended for rejected submit");
        assert_eq!(rejected.1, pending_id);
        assert!((rejected.2 - 960.0).abs() < f64::EPSILON);
        match rejected.0 {
            CliMutationRequestRecord::SubmitOutput {
                step_name,
                contract,
            } => {
                assert_eq!(step_name, "review");
                assert_eq!(contract, "review-verdict");
            }
            other => panic!("expected SubmitOutput request record, got: {other:?}"),
        }
    }

    /// 5-4 修正: reject rule の無い approval node への reject は engine 側で
    /// `InvalidState` として拒否される。dispatcher は `CliMutationRequested`
    /// （リクエスト受領事実）に加え、`CliMutationRejected{reason: no_reject_rule}`
    /// を補助履歴として追記する。`ApprovalResolved` は記録されない。
    #[tokio::test]
    async fn dispatch_pending_reject_without_rule_records_cli_mutation_rejected() {
        use crate::workflow::event::CliMutationRejectionReason;
        let app = make_dispatch_app();
        let engine = Arc::new(WorkflowEngine::new_for_test());
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_run_store_data_dir(data_dir).await;
        let (session_store, handles) = make_dispatch_deps();
        let run_id = uuid::Uuid::new_v4().to_string();
        // reject rule を持たない approval-only workflow を使う。
        seed_waiting_approval(
            &engine,
            &run_id,
            "/wt/pending-dispatcher-reject-without-rule",
            make_approval_only_workflow(),
        )
        .await;

        let pending = PendingCommand::new(
            run_id.clone(),
            CliRequestPayload::Reject {
                node_name: None,
                reason: "engine 側に reject rule が無い node を拒否".to_string(),
            },
            970.0,
        );

        let result =
            dispatch_pending_command(app.handle(), &engine, &session_store, &handles, pending)
                .await;
        assert!(matches!(
            result,
            PendingCommandDispatchOutcome::RejectedFinal(_)
        ));

        let events = read_dispatch_events(&app, &run_id);
        // ApprovalResolved は出ない（state 変化が起きていないため）。
        assert!(events
            .iter()
            .all(|event| !matches!(event, WorkflowEvent::ApprovalResolved { .. })));
        // CliMutationRequested は引き続き記録される（リクエスト受領事実）。
        assert!(events
            .iter()
            .any(|event| matches!(event, WorkflowEvent::CliMutationRequested { .. })));
        // 5-4 修正: CliMutationRejected{reason: no_reject_rule} が補助履歴として追記される。
        let reason = events
            .iter()
            .find_map(|event| match event {
                WorkflowEvent::CliMutationRejected { reason, .. } => Some(*reason),
                _ => None,
            })
            .expect("CliMutationRejected event must be appended for no-reject-rule path");
        assert!(matches!(reason, CliMutationRejectionReason::NoRejectRule));
    }

    /// 5-3 / 5-4 修正: `classify_rejection_reason` は engine error の典型的な
    /// メッセージから `CliMutationRejectionReason` を導出する。
    #[test]
    fn classify_rejection_reason_maps_known_errors() {
        use crate::workflow::event::CliMutationRejectionReason as R;
        let cases: Vec<(WorkflowEngineError, R)> = vec![
            (
                WorkflowEngineError::ExecutionNotFound("run".to_string()),
                R::RunNotFound,
            ),
            (
                WorkflowEngineError::UnauthorizedApprovalTarget("target".to_string()),
                R::NotWaitingApproval,
            ),
            (
                WorkflowEngineError::ValidationError(
                    "contract mismatch: step 'r' expects 'a', got 'b'".to_string(),
                ),
                R::ContractMismatch,
            ),
            (
                WorkflowEngineError::ValidationError(
                    "step 'r' is not a valid submission target".to_string(),
                ),
                R::NodeNotFound,
            ),
            (
                WorkflowEngineError::InvalidState("Step 'r' does not allow reject".to_string()),
                R::NoRejectRule,
            ),
            (
                WorkflowEngineError::InvalidState(
                    "step 'r' is not currently accepting structured output".to_string(),
                ),
                R::StepNotAccepting,
            ),
            (
                WorkflowEngineError::InvalidState("run x is already terminal".to_string()),
                R::RunNotActive,
            ),
            (
                WorkflowEngineError::InvalidState(
                    "run x is not accepting structured output (state: Completed)".to_string(),
                ),
                R::RunNotActive,
            ),
            (
                WorkflowEngineError::InvalidState("something else".to_string()),
                R::Other,
            ),
        ];
        for (err, expected) in cases {
            let got = WorkflowEngine::classify_rejection_reason(&err);
            assert_eq!(got, expected, "unexpected reason for error: {err}");
        }
    }
}
