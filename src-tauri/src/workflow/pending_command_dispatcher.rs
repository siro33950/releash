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

    let request = payload_to_cli_request(&payload);
    let metadata = WorkflowMutationContext::new(
        run_id.clone(),
        WorkflowMutationSource::CliPendingCommand { request_id: id },
        request,
        requested_at,
    );

    let command = payload_to_workflow_command(payload, run_id.clone());
    let request_id = metadata.request_id().to_string();
    match engine.cli_mutation_already_recorded(app, &run_id, &request_id) {
        Ok(true) => return PendingCommandDispatchOutcome::Accepted,
        Ok(false) => {}
        Err(e) => return classify_dispatch_error(e),
    }
    let commit_context = CommandCommitContext::cli_pending(metadata);

    if let Err(e) = engine
        .ensure_execution_loaded_for_external(app, session_store, &run_id)
        .await
    {
        return handle_rejected_dispatch(app, engine, e, commit_context).await;
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
        Err(e) => handle_rejected_dispatch(app, engine, e, commit_context).await,
    }
}

async fn handle_rejected_dispatch<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    engine: &WorkflowEngine,
    error: WorkflowEngineError,
    commit_context: CommandCommitContext,
) -> PendingCommandDispatchOutcome {
    let reason = error.to_string();
    if WorkflowEngine::should_commit_rejected_external_request(&error) {
        if let Err(record_error) = engine
            .append_command_commit_context(app, commit_context)
            .await
        {
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

    fn create_parent_session(
        app: &DispatchTestApp,
        session_store: &crate::session::SessionStore,
        worktree_path: &str,
    ) -> crate::session::ChatSession {
        let data_dir = dispatch_data_dir(app.handle());
        crate::session::create_session_internal_with_permission(
            session_store,
            &data_dir,
            worktree_path,
            None,
            crate::permission::PermissionMode::Edit,
        )
        .unwrap()
    }

    async fn seed_waiting_approval(
        app: &DispatchTestApp,
        engine: &WorkflowEngine,
        session_store: &crate::session::SessionStore,
        run_id: &str,
        worktree_path: &str,
        workflow: Workflow,
    ) {
        let parent = create_parent_session(app, session_store, worktree_path);
        engine
            .seed_active_execution_for_test(
                run_id.to_string(),
                workflow,
                WorkflowExecutionState::WaitingApproval,
                worktree_path.to_string(),
                parent.id,
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
            &app,
            &engine,
            &session_store,
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
            &app,
            &engine,
            &session_store,
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
            &app,
            &engine,
            &session_store,
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
            &app,
            &engine,
            &session_store,
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
}
