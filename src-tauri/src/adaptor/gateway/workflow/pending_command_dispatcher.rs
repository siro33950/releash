//! [06] CLI pending command の dispatcher gateway。
//!
//! file-direct payload の解釈を engine から分離し、engine には runtime primitive と
//! mutation route context だけを渡す。

use std::sync::Arc;

use tokio::sync::Mutex;

use crate::adaptor::gateway::workflow::engine_error::{
    classify_cli_mutation_rejection_reason, should_commit_rejected_external_request,
    WorkflowEngineError,
};
use crate::adaptor::gateway::workflow::event::CliMutationRequestRecord;
use crate::adaptor::gateway::workflow::pending_command::{
    PendingCommand, PendingCommandEntry, PendingCommandPayload, PendingCommandStore,
};
use crate::adaptor::gateway::workflow::route_context::{
    CommandCommitContext, WorkflowMutationContext, WorkflowMutationSource,
};
use crate::adaptor::gateway::workflow::runtime_state::ApprovalDecision as RuntimeApprovalDecision;
use crate::infrastructure::agent_session::runtime::AgentProcessMap;
use crate::usecase::agent_session::session::SessionStore;

use super::pending_runtime::PendingCommandRuntime;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PendingCommandDispatchOutcome {
    Accepted,
    RejectedFinal(String),
    RetryableFailure(String),
}

pub(crate) async fn process_pending_command_entry<R, E>(
    app: &tauri::AppHandle<R>,
    engine: &E,
    session_store: &Arc<SessionStore>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    store: &PendingCommandStore,
    entry: PendingCommandEntry,
) where
    R: tauri::Runtime,
    E: PendingCommandRuntime<R> + ?Sized,
{
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

pub(crate) async fn dispatch_pending_command<R, E>(
    app: &tauri::AppHandle<R>,
    engine: &E,
    session_store: &Arc<SessionStore>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    pending: PendingCommand,
) -> PendingCommandDispatchOutcome
where
    R: tauri::Runtime,
    E: PendingCommandRuntime<R> + ?Sized,
{
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

    // [08] CLI 提出経路と in-process 経路は engine の submit-output primitive で合流する。
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
    let dispatch_payload = payload_to_runtime_dispatch(payload);

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

    match dispatch_runtime_payload(
        app,
        engine,
        session_store,
        handles,
        &run_id,
        dispatch_payload,
        commit_context.clone(),
        &request_id,
        requested_at,
    )
    .await
    {
        Ok(()) => PendingCommandDispatchOutcome::Accepted,
        Err(e) => {
            handle_rejected_dispatch(app, engine, e, commit_context, &run_id, is_submit_output)
                .await
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn dispatch_runtime_payload<R, E>(
    app: &tauri::AppHandle<R>,
    engine: &E,
    session_store: &Arc<SessionStore>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    run_id: &str,
    payload: PendingRuntimeDispatchPayload,
    commit_context: CommandCommitContext,
    request_id: &str,
    requested_at: f64,
) -> Result<(), WorkflowEngineError>
where
    R: tauri::Runtime,
    E: PendingCommandRuntime<R> + ?Sized,
{
    match payload {
        PendingRuntimeDispatchPayload::Approval {
            node_name,
            decision,
            approval_comment,
        } => {
            engine
                .resolve_workflow_approval_with_commit_context(
                    app,
                    session_store,
                    handles,
                    run_id,
                    decision,
                    approval_comment,
                    node_name.as_deref(),
                    Some(commit_context),
                )
                .await
        }
        PendingRuntimeDispatchPayload::Abort { expected_node_name } => {
            engine
                .abort_workflow_run_with_commit_context(
                    app,
                    session_store,
                    handles,
                    run_id,
                    expected_node_name.as_deref(),
                    Some(commit_context),
                )
                .await
        }
        PendingRuntimeDispatchPayload::SubmitOutput {
            step_name,
            contract,
            structured_output,
        } => {
            engine
                .submit_workflow_output(
                    app,
                    session_store,
                    handles,
                    run_id,
                    step_name,
                    contract,
                    structured_output,
                    Some(request_id.to_string()),
                    Some(requested_at),
                )
                .await
        }
    }
}

async fn handle_rejected_dispatch<R, E>(
    app: &tauri::AppHandle<R>,
    engine: &E,
    error: WorkflowEngineError,
    commit_context: CommandCommitContext,
    run_id: &str,
    is_submit_output: bool,
) -> PendingCommandDispatchOutcome
where
    R: tauri::Runtime,
    E: PendingCommandRuntime<R> + ?Sized,
{
    let reason = error.to_string();
    let should_commit = should_commit_rejected_external_request(&error);
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

/// pending payload → runtime primitive 入力への純変換。
///
/// 自由記述テキストの境界バリデーション（reject reason 非空 / 文字数上限）は
/// engine 受理時の `validate_approval_decision` / `validate_approve_comment_length`
/// に委ね、本 adapter では事前検証を行わない（review R2-01: ドメイン pure helper
/// 集約に伴う dispatcher 重複検証の削除）。
#[derive(Debug, Clone, PartialEq)]
enum PendingRuntimeDispatchPayload {
    Approval {
        node_name: Option<String>,
        decision: RuntimeApprovalDecision,
        approval_comment: Option<String>,
    },
    Abort {
        expected_node_name: Option<String>,
    },
    SubmitOutput {
        step_name: String,
        contract: String,
        structured_output: serde_json::Value,
    },
}

fn payload_to_runtime_dispatch(payload: PendingCommandPayload) -> PendingRuntimeDispatchPayload {
    match payload {
        PendingCommandPayload::Approve { node_name, comment } => {
            PendingRuntimeDispatchPayload::Approval {
                node_name,
                decision: RuntimeApprovalDecision::Approve,
                approval_comment: comment,
            }
        }
        PendingCommandPayload::Reject { node_name, reason } => {
            PendingRuntimeDispatchPayload::Approval {
                node_name,
                decision: RuntimeApprovalDecision::Reject {
                    comment: reason.clone(),
                },
                approval_comment: Some(reason),
            }
        }
        PendingCommandPayload::Abort { node_name } => PendingRuntimeDispatchPayload::Abort {
            expected_node_name: node_name,
        },
        PendingCommandPayload::SubmitOutput {
            step_name,
            contract,
            structured_output,
        } => PendingRuntimeDispatchPayload::SubmitOutput {
            step_name,
            contract,
            structured_output,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptor::gateway::workflow::pending_command::CliRequestPayload;
    use std::sync::Mutex as StdMutex;
    use tempfile::TempDir;

    type DispatchTestApp = tauri::App<tauri::test::MockRuntime>;

    #[derive(Default)]
    struct FakePendingRuntime {
        approval_calls: StdMutex<Vec<ApprovalRuntimeCall>>,
        abort_calls: StdMutex<Vec<AbortRuntimeCall>>,
        submit_calls: StdMutex<Vec<SubmitRuntimeCall>>,
        append_contexts: StdMutex<Vec<CommandCommitContext>>,
        rejected_submit_contexts: StdMutex<Vec<(String, CommandCommitContext, String)>>,
        rejected_mutation_contexts: StdMutex<Vec<(CommandCommitContext, String)>>,
        next_approval_error: StdMutex<Option<WorkflowEngineError>>,
        next_abort_error: StdMutex<Option<WorkflowEngineError>>,
        next_submit_error: StdMutex<Option<WorkflowEngineError>>,
        next_append_context_error: StdMutex<Option<WorkflowEngineError>>,
        cli_already_recorded: StdMutex<bool>,
    }

    #[derive(Debug)]
    struct ApprovalRuntimeCall {
        run_id: String,
        decision: RuntimeApprovalDecision,
        approval_comment: Option<String>,
        node_name: Option<String>,
        commit_context: Option<CommandCommitContext>,
    }

    #[derive(Debug)]
    struct AbortRuntimeCall {
        run_id: String,
        expected_node_name: Option<String>,
        commit_context: Option<CommandCommitContext>,
    }

    #[derive(Debug)]
    struct SubmitRuntimeCall {
        run_id: String,
        step_name: String,
        contract: String,
        structured_output: serde_json::Value,
        request_id: Option<String>,
        submitted_at: Option<f64>,
    }

    impl FakePendingRuntime {
        fn reject_next_approval(&self, error: WorkflowEngineError) {
            *self.next_approval_error.lock().unwrap() = Some(error);
        }

        fn reject_next_submit(&self, error: WorkflowEngineError) {
            *self.next_submit_error.lock().unwrap() = Some(error);
        }

        fn fail_next_append_context(&self, error: WorkflowEngineError) {
            *self.next_append_context_error.lock().unwrap() = Some(error);
        }
    }

    #[async_trait::async_trait]
    impl PendingCommandRuntime<tauri::test::MockRuntime> for FakePendingRuntime {
        fn output_submitted_already_recorded(
            &self,
            _app: &tauri::AppHandle<tauri::test::MockRuntime>,
            _run_id: &str,
            _request_id: &str,
        ) -> Result<bool, WorkflowEngineError> {
            Ok(false)
        }

        fn cli_mutation_already_recorded(
            &self,
            _app: &tauri::AppHandle<tauri::test::MockRuntime>,
            _run_id: &str,
            _request_id: &str,
        ) -> Result<bool, WorkflowEngineError> {
            Ok(*self.cli_already_recorded.lock().unwrap())
        }

        async fn ensure_execution_loaded_for_external(
            &self,
            _app: &tauri::AppHandle<tauri::test::MockRuntime>,
            _session_store: &Arc<SessionStore>,
            _run_id: &str,
        ) -> Result<(), WorkflowEngineError> {
            Ok(())
        }

        async fn resolve_workflow_approval_with_commit_context(
            &self,
            _app: &tauri::AppHandle<tauri::test::MockRuntime>,
            _session_store: &Arc<SessionStore>,
            _handles: &Arc<Mutex<AgentProcessMap>>,
            run_id: &str,
            decision: RuntimeApprovalDecision,
            approval_comment: Option<String>,
            node_name: Option<&str>,
            commit_context: Option<CommandCommitContext>,
        ) -> Result<(), WorkflowEngineError> {
            self.approval_calls
                .lock()
                .unwrap()
                .push(ApprovalRuntimeCall {
                    run_id: run_id.to_string(),
                    decision,
                    approval_comment,
                    node_name: node_name.map(ToOwned::to_owned),
                    commit_context,
                });
            match self.next_approval_error.lock().unwrap().take() {
                Some(error) => Err(error),
                None => Ok(()),
            }
        }

        async fn abort_workflow_run_with_commit_context(
            &self,
            _app: &tauri::AppHandle<tauri::test::MockRuntime>,
            _session_store: &Arc<SessionStore>,
            _handles: &Arc<Mutex<AgentProcessMap>>,
            run_id: &str,
            expected_node_name: Option<&str>,
            commit_context: Option<CommandCommitContext>,
        ) -> Result<(), WorkflowEngineError> {
            self.abort_calls.lock().unwrap().push(AbortRuntimeCall {
                run_id: run_id.to_string(),
                expected_node_name: expected_node_name.map(ToOwned::to_owned),
                commit_context,
            });
            match self.next_abort_error.lock().unwrap().take() {
                Some(error) => Err(error),
                None => Ok(()),
            }
        }

        async fn submit_workflow_output(
            &self,
            _app: &tauri::AppHandle<tauri::test::MockRuntime>,
            _session_store: &Arc<SessionStore>,
            _handles: &Arc<Mutex<AgentProcessMap>>,
            run_id: &str,
            step_name: String,
            contract: String,
            structured_output: serde_json::Value,
            request_id: Option<String>,
            submitted_at: Option<f64>,
        ) -> Result<(), WorkflowEngineError> {
            self.submit_calls.lock().unwrap().push(SubmitRuntimeCall {
                run_id: run_id.to_string(),
                step_name,
                contract,
                structured_output,
                request_id,
                submitted_at,
            });
            match self.next_submit_error.lock().unwrap().take() {
                Some(error) => Err(error),
                None => Ok(()),
            }
        }

        async fn append_command_commit_context(
            &self,
            _app: &tauri::AppHandle<tauri::test::MockRuntime>,
            commit_context: CommandCommitContext,
        ) -> Result<(), WorkflowEngineError> {
            self.append_contexts.lock().unwrap().push(commit_context);
            match self.next_append_context_error.lock().unwrap().take() {
                Some(error) => Err(error),
                None => Ok(()),
            }
        }

        async fn append_cli_mutation_rejected_for_submit_output(
            &self,
            _app: &tauri::AppHandle<tauri::test::MockRuntime>,
            run_id: &str,
            commit_context: &CommandCommitContext,
            error: &WorkflowEngineError,
        ) -> Result<(), WorkflowEngineError> {
            self.rejected_submit_contexts.lock().unwrap().push((
                run_id.to_string(),
                commit_context.clone(),
                error.to_string(),
            ));
            Ok(())
        }

        async fn append_cli_mutation_rejected(
            &self,
            _app: &tauri::AppHandle<tauri::test::MockRuntime>,
            commit_context: &CommandCommitContext,
            error: &WorkflowEngineError,
        ) -> Result<(), WorkflowEngineError> {
            self.rejected_mutation_contexts
                .lock()
                .unwrap()
                .push((commit_context.clone(), error.to_string()));
            Ok(())
        }
    }

    fn make_dispatch_app() -> DispatchTestApp {
        tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("tauri mock test app must build")
    }

    fn make_dispatch_deps() -> (
        Arc<crate::usecase::agent_session::session::SessionStore>,
        Arc<Mutex<AgentProcessMap>>,
    ) {
        (
            Arc::new(crate::test_support::build_session_store()),
            Arc::new(Mutex::new(AgentProcessMap::new())),
        )
    }

    fn make_pending_store_dir() -> TempDir {
        TempDir::new().expect("pending store temp dir must be created")
    }

    fn assert_cli_pending_context(
        context: &CommandCommitContext,
        expected_run_id: &str,
        expected_request: CliMutationRequestRecord,
        expected_requested_at: f64,
    ) {
        let mutation = context
            .cli_pending_mutation()
            .expect("context must be CliPending");
        let (run_id, request, requested_at, _request_id) = mutation.clone().into_event_parts();
        assert_eq!(run_id, expected_run_id);
        assert_eq!(request, expected_request);
        assert!((requested_at - expected_requested_at).abs() < f64::EPSILON);
    }

    #[test]
    fn payload_to_runtime_dispatch_preserves_omitted_approve_node_for_engine_resolution() {
        let payload = CliRequestPayload::Approve {
            node_name: None,
            comment: None,
        };
        let dispatch_payload = payload_to_runtime_dispatch(payload.clone());
        let request = payload_to_cli_request(&payload);
        match dispatch_payload {
            PendingRuntimeDispatchPayload::Approval {
                node_name,
                decision,
                approval_comment,
            } => {
                assert!(node_name.is_none());
                assert_eq!(decision, RuntimeApprovalDecision::Approve);
                assert!(approval_comment.is_none());
            }
            other => panic!("expected approval dispatch payload, got: {other:?}"),
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
    // comment 文字数上限）を行っていた `payload_to_runtime_dispatch_rejects_*`
    // テストは、review R2-01（ドメインルールの 3 層重複解消）に伴い削除済み。
    // 同等の境界バリデーションは `engine::validate_approval_decision` /
    // `engine::validate_approve_comment_length` で担保されており、engine 側
    // テスト（`validate_approval_decision_reject_*` / `approve_comment_length_*`）
    // が引き続きカバーする。dispatch 全体の RejectedFinal 経路は
    // `process_pending_entry_marks_final_reject_processed_once` でカバー済み。

    #[tokio::test]
    async fn dispatch_pending_approve_records_cli_request_after_engine_acceptance() {
        let app = make_dispatch_app();
        let runtime = FakePendingRuntime::default();
        let (session_store, handles) = make_dispatch_deps();
        let run_id = uuid::Uuid::new_v4().to_string();

        let pending = PendingCommand::new(
            run_id.clone(),
            CliRequestPayload::Approve {
                node_name: None,
                comment: Some("cli-lgtm".to_string()),
            },
            900.0,
        );

        let result =
            dispatch_pending_command(app.handle(), &runtime, &session_store, &handles, pending)
                .await;
        assert_eq!(result, PendingCommandDispatchOutcome::Accepted);

        let calls = runtime.approval_calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        let call = &calls[0];
        assert_eq!(call.run_id, run_id);
        assert_eq!(call.decision, RuntimeApprovalDecision::Approve);
        assert_eq!(call.approval_comment.as_deref(), Some("cli-lgtm"));
        assert!(call.node_name.is_none());
        assert_cli_pending_context(
            call.commit_context
                .as_ref()
                .expect("approval call must carry commit context"),
            &run_id,
            CliMutationRequestRecord::Approve {
                node_name: None,
                comment: Some("cli-lgtm".to_string()),
            },
            900.0,
        );
    }

    #[tokio::test]
    async fn process_pending_entry_marks_final_reject_processed_once() {
        let app = make_dispatch_app();
        let runtime = FakePendingRuntime::default();
        runtime.reject_next_approval(WorkflowEngineError::UnauthorizedApprovalTarget(
            "stale-review".to_string(),
        ));
        let temp_dir = make_pending_store_dir();
        let data_dir = temp_dir.path().to_path_buf();
        let (session_store, handles) = make_dispatch_deps();
        let run_id = uuid::Uuid::new_v4().to_string();

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
            &runtime,
            &session_store,
            &handles,
            &store,
            entry.clone(),
        )
        .await;

        assert!(store.list_pending().unwrap().is_empty());
        assert_eq!(runtime.append_contexts.lock().unwrap().len(), 1);
        assert_eq!(runtime.rejected_mutation_contexts.lock().unwrap().len(), 1);

        process_pending_command_entry(
            app.handle(),
            &runtime,
            &session_store,
            &handles,
            &store,
            entry,
        )
        .await;
        assert_eq!(runtime.append_contexts.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn process_pending_entry_releases_retryable_failure_back_to_pending() {
        let app = make_dispatch_app();
        let runtime = FakePendingRuntime::default();
        runtime.reject_next_approval(WorkflowEngineError::SessionStore(
            "temporary io".to_string(),
        ));
        let temp_dir = make_pending_store_dir();
        let data_dir = temp_dir.path().to_path_buf();
        let (session_store, handles) = make_dispatch_deps();
        let run_id = uuid::Uuid::new_v4().to_string();

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

        process_pending_command_entry(
            app.handle(),
            &runtime,
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
        let runtime = FakePendingRuntime::default();
        let (session_store, handles) = make_dispatch_deps();
        let run_id = uuid::Uuid::new_v4().to_string();

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
            dispatch_pending_command(app.handle(), &runtime, &session_store, &handles, pending)
                .await;
        assert_eq!(result, PendingCommandDispatchOutcome::Accepted);

        let calls = runtime.approval_calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        let call = &calls[0];
        assert!(matches!(
            call.decision,
            RuntimeApprovalDecision::Reject { .. }
        ));
        assert_eq!(call.approval_comment.as_deref(), Some(raw_reason.as_str()));
        assert_cli_pending_context(
            call.commit_context
                .as_ref()
                .expect("reject call must carry commit context"),
            &run_id,
            CliMutationRequestRecord::Reject {
                node_name: None,
                reason: raw_reason,
            },
            905.0,
        );
    }

    /// [08] CLI pending 経由の SubmitOutput が engine の handle_submit_output
    /// に合流し、`OutputSubmitted` event が caller の `request_id` と `submitted_at`
    /// を保持する形で append される（spec [08] CLI 経路と in-process 経路の合流境界）。
    #[tokio::test]
    async fn dispatch_pending_submit_output_appends_event_with_caller_metadata() {
        let app = make_dispatch_app();
        let runtime = FakePendingRuntime::default();
        let (session_store, handles) = make_dispatch_deps();
        let run_id = uuid::Uuid::new_v4().to_string();

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
            dispatch_pending_command(app.handle(), &runtime, &session_store, &handles, pending)
                .await;
        assert_eq!(result, PendingCommandDispatchOutcome::Accepted);

        let calls = runtime.submit_calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        let call = &calls[0];
        assert_eq!(call.run_id, run_id);
        assert_eq!(call.step_name, "review");
        assert_eq!(call.contract, "review-verdict");
        assert_eq!(
            call.structured_output,
            serde_json::json!({"verdict": "LGTM"})
        );
        assert_eq!(call.request_id.as_deref(), Some(pending_id.as_str()));
        assert_eq!(call.submitted_at, Some(950.5));
    }

    /// [08] CLI pending 経由の SubmitOutput が contract 不適合の場合、event は残らず、
    /// dispatcher は `RejectedFinal` を返す（spec [08] 振る舞い定義 Rule 1 適合しない場合）。
    ///
    /// 5-3 修正: `OutputSubmitted` / `CliMutationRequested` は引き続き残らないが、
    /// 観測経路用の補助履歴として `CliMutationRejected` event が追記される。
    #[tokio::test]
    async fn dispatch_pending_submit_output_rejects_invalid_contract() {
        let app = make_dispatch_app();
        let runtime = FakePendingRuntime::default();
        runtime.reject_next_submit(WorkflowEngineError::ValidationError(
            "contract mismatch: step 'review' expects 'review-verdict', got 'spec-directory'"
                .to_string(),
        ));
        let (session_store, handles) = make_dispatch_deps();
        let run_id = uuid::Uuid::new_v4().to_string();

        let pending = PendingCommand::new(
            run_id.clone(),
            CliRequestPayload::SubmitOutput {
                step_name: "review".to_string(),
                contract: "spec-directory".to_string(),
                structured_output: serde_json::json!({"spec_dir": "/not/relative"}),
            },
            960.0,
        );
        let pending_id = pending.id.clone();

        let result =
            dispatch_pending_command(app.handle(), &runtime, &session_store, &handles, pending)
                .await;
        assert!(matches!(
            result,
            PendingCommandDispatchOutcome::RejectedFinal(_)
        ));

        assert!(runtime.append_contexts.lock().unwrap().is_empty());
        let rejected = runtime.rejected_submit_contexts.lock().unwrap();
        assert_eq!(rejected.len(), 1);
        assert_eq!(rejected[0].0, run_id);
        let (request_id, requested_at, step_name, contract) = rejected[0]
            .1
            .submit_output_rejection_parts()
            .expect("submit rejection context must carry submit metadata");
        assert_eq!(request_id, pending_id);
        assert!((requested_at - 960.0).abs() < f64::EPSILON);
        assert_eq!(step_name, "review");
        assert_eq!(contract, "spec-directory");
        assert!(
            rejected[0].2.contains("contract mismatch"),
            "unexpected rejection: {}",
            rejected[0].2
        );
    }

    /// 5-4 修正: reject rule の無い approval node への reject は runtime 側で
    /// `InvalidState` として拒否される。dispatcher は request context と rejected
    /// context の追記を runtime に委譲する。
    #[tokio::test]
    async fn dispatch_pending_reject_without_rule_records_cli_mutation_rejected() {
        let app = make_dispatch_app();
        let runtime = FakePendingRuntime::default();
        runtime.reject_next_approval(WorkflowEngineError::InvalidState(
            "Step 'review' does not allow reject".to_string(),
        ));
        let (session_store, handles) = make_dispatch_deps();
        let run_id = uuid::Uuid::new_v4().to_string();

        let pending = PendingCommand::new(
            run_id.clone(),
            CliRequestPayload::Reject {
                node_name: None,
                reason: "engine 側に reject rule が無い node を拒否".to_string(),
            },
            970.0,
        );

        let result =
            dispatch_pending_command(app.handle(), &runtime, &session_store, &handles, pending)
                .await;
        assert!(matches!(
            result,
            PendingCommandDispatchOutcome::RejectedFinal(_)
        ));

        assert_eq!(runtime.append_contexts.lock().unwrap().len(), 1);
        let rejected = runtime.rejected_mutation_contexts.lock().unwrap();
        assert_eq!(rejected.len(), 1);
        assert_cli_pending_context(
            &rejected[0].0,
            &run_id,
            CliMutationRequestRecord::Reject {
                node_name: None,
                reason: "engine 側に reject rule が無い node を拒否".to_string(),
            },
            970.0,
        );
        assert!(rejected[0].1.contains("does not allow reject"));
    }

    /// 5-3 / 5-4 修正: `classify_cli_mutation_rejection_reason` は engine error の典型的な
    /// メッセージから `CliMutationRejectionReason` を導出する。
    #[test]
    fn classify_rejection_reason_maps_known_errors() {
        use crate::adaptor::gateway::workflow::event::CliMutationRejectionReason as R;
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
            let got = classify_cli_mutation_rejection_reason(&err);
            assert_eq!(got, expected, "unexpected reason for error: {err}");
        }
    }
}
