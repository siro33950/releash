//! Stop, resume, abort, and approval orchestration.

use super::*;

enum AbortCommit {
    Aborted { session_ids: Vec<String> },
    NotFound,
    AlreadyTerminal,
}

fn abort_outcome_to_command_result(
    outcome: AbortOutcome,
    execution_id: &str,
) -> Result<(), WorkflowRuntimeError> {
    match outcome {
        AbortOutcome::Aborted => Ok(()),
        AbortOutcome::NotFound => Err(WorkflowRuntimeError::ExecutionNotFound(
            execution_id.to_string(),
        )),
        AbortOutcome::AlreadyTerminal => Err(WorkflowRuntimeError::InvalidState(format!(
            "execution {execution_id} is already terminal"
        ))),
    }
}

/// abort / stop / resume の execution ライフサイクル typed command 群。
impl WorkflowRuntimeHost {
    pub(crate) async fn abort_workflow_execution<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        execution_id: &str,
        expected_node_name: Option<&str>,
    ) -> Result<(), WorkflowRuntimeError> {
        let metadata = self.validate_execution_command_target(execution_id).await?;
        if let Some(expected_node_name) = expected_node_name {
            if metadata.current_node.as_deref() != Some(expected_node_name) {
                return Err(WorkflowRuntimeError::UnauthorizedApprovalTarget(
                    "node does not match".to_string(),
                ));
            }
        }
        if !metadata.status.is_active() {
            return Err(WorkflowRuntimeError::InvalidState(format!(
                "execution {execution_id} cannot be aborted from status {}",
                metadata.status.as_str()
            )));
        }
        let interruption_reservation = self
            .execution_store
            .reserve_active_interruption(execution_id)
            .await
            .map_err(|error| match error {
                ExecutionStoreError::ExecutionNotFound { .. } => {
                    WorkflowRuntimeError::ExecutionNotFound(execution_id.to_string())
                }
                ExecutionStoreError::InvalidStatusTransition { .. }
                | ExecutionStoreError::TransitionInProgress { .. } => {
                    WorkflowRuntimeError::InvalidState(error.to_string())
                }
                other => WorkflowRuntimeError::SessionStore(format!(
                    "ExecutionStore abort reservation failed: {other}"
                )),
            })?;
        let activation_gate = self.runtime_activation_gate(execution_id).await;
        activation_gate.request_cancel();
        let mut activation_guard = None;
        tokio::select! {
            biased;
            _ = activation_gate.cancellation_acknowledged() => {}
            guard = activation_gate.lock.lock() => {
                activation_guard = Some(guard);
            }
        }
        let activation_was_paused = activation_guard.is_none();
        // execution 全体の Abort: NotFound / AlreadyTerminal は非受理として typed error
        // に射影する（Spec [04] Rule「対象不在 / 既に終了した command は受理されない」）。
        let abort_result = self
            .commit_abort_workflow_by_execution_id(app, execution_id, expected_node_name)
            .await;
        match abort_result {
            Ok(AbortCommit::Aborted { session_ids }) => {
                if activation_was_paused {
                    activation_gate.commit_cancel();
                    activation_guard = Some(activation_gate.lock.lock().await);
                }
                let _activation_guard = activation_guard;
                let terminal_cleanup_result = self
                    .finish_committed_abort(app, execution_id, &session_ids)
                    .await;
                self.execution_store
                    .finish_active_interruption(interruption_reservation)
                    .await
                    .map_err(|error| {
                        WorkflowRuntimeError::SessionStore(format!(
                            "ExecutionStore abort reservation cleanup failed: {error}"
                        ))
                    })?;
                terminal_cleanup_result?;
                abort_outcome_to_command_result(AbortOutcome::Aborted, execution_id)
            }
            Ok(AbortCommit::NotFound) => {
                self.execution_store
                    .finish_active_interruption(interruption_reservation)
                    .await
                    .map_err(|error| {
                        WorkflowRuntimeError::SessionStore(format!(
                            "ExecutionStore abort reservation rollback failed: {error}"
                        ))
                    })?;
                if activation_was_paused {
                    activation_gate.rollback_cancel();
                } else {
                    activation_gate.reset_cancel();
                }
                abort_outcome_to_command_result(AbortOutcome::NotFound, execution_id)
            }
            Ok(AbortCommit::AlreadyTerminal) => {
                self.execution_store
                    .finish_active_interruption(interruption_reservation)
                    .await
                    .map_err(|error| {
                        WorkflowRuntimeError::SessionStore(format!(
                            "ExecutionStore abort reservation rollback failed: {error}"
                        ))
                    })?;
                if activation_was_paused {
                    activation_gate.rollback_cancel();
                } else {
                    activation_gate.reset_cancel();
                }
                abort_outcome_to_command_result(AbortOutcome::AlreadyTerminal, execution_id)
            }
            Err(error) => {
                let reservation_result = self
                    .execution_store
                    .finish_active_interruption(interruption_reservation)
                    .await;
                if activation_was_paused {
                    activation_gate.rollback_cancel();
                } else {
                    activation_gate.reset_cancel();
                }
                reservation_result.map_err(|reservation_error| {
                    WorkflowRuntimeError::SessionStore(format!(
                        "abort failed: {error}; abort reservation rollback failed: {reservation_error}"
                    ))
                })?;
                Err(error)
            }
        }
    }

    /// Pause all currently running Node Attempts without changing the Workflow lifecycle.
    /// NodePaused is published only after every targeted runtime has accepted cancellation.
    pub(crate) async fn stop_workflow_execution<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        execution_id: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        let metadata = self.validate_execution_command_target(execution_id).await?;
        if !metadata.status.is_active() {
            return Err(WorkflowRuntimeError::InvalidState(format!(
                "execution {execution_id} cannot be stopped from status {}",
                metadata.status.as_str()
            )));
        }
        let timestamp = current_timestamp();
        let (snapshot_before, candidate, events, session_targets, command_ids, worktree_path) = {
            let executions = self.executions.lock().await;
            let snapshot_before = executions
                .get(execution_id)
                .cloned()
                .ok_or_else(|| WorkflowRuntimeError::ExecutionNotFound(execution_id.to_string()))?;
            let mut candidate = snapshot_before.clone();
            let targets = candidate
                .node_executions
                .iter()
                .filter(|node| node.status == NodeExecutionStatus::Running)
                .map(|node| (node.id.clone(), node.kind, node.session_id.clone()))
                .collect::<Vec<_>>();
            let mut events = Vec::new();
            let mut session_targets = Vec::new();
            let mut command_ids = Vec::new();
            for (node_execution_id, kind, session_id) in targets {
                if candidate.pause_node_execution(&node_execution_id, timestamp)
                    != crate::domain::workflow::entities::workflow_execution::TransitionOutcome::Applied
                {
                    continue;
                }
                events.push(WorkflowEvent::NodePaused {
                    execution_id: execution_id.to_string(),
                    node_execution_id: node_execution_id.clone(),
                    timestamp,
                });
                match kind {
                    NodeKindName::Session => {
                        if let Some(session_id) = session_id {
                            session_targets.push((node_execution_id.clone(), session_id));
                        }
                    }
                    NodeKindName::Command => command_ids.push(node_execution_id),
                    NodeKindName::Fanout | NodeKindName::Sequence => {}
                }
            }
            let worktree_path = candidate.worktree_path.clone();
            (
                snapshot_before,
                candidate,
                events,
                session_targets,
                command_ids,
                worktree_path,
            )
        };
        if events.is_empty() {
            return Ok(());
        }
        let snapshot = self
            .commit_control_plane_candidate(
                app,
                ControlPlaneCommitCandidate {
                    execution_id,
                    snapshot_before,
                    candidate,
                    events: &events,
                    provider_events: Vec::new(),
                },
            )
            .await?;
        workflow_runtime_session::broadcast_state(app, &worktree_path, snapshot).await;
        let mut unstopped_node_execution_ids = HashSet::new();
        let mut stop_failures = Vec::new();
        for (node_execution_id, session_id) in session_targets {
            if let Err(error) = self
                .workflow_agent_sessions
                .interrupt_workflow_agent_session(&session_id)
                .await
            {
                unstopped_node_execution_ids.insert(node_execution_id);
                stop_failures.push(error.to_string());
            }
        }
        if !command_ids.is_empty() {
            let handles = self.active_commands.lock().await;
            for node_execution_id in &command_ids {
                if let Some(handle) = handles.get(node_execution_id) {
                    handle.request_shutdown();
                }
            }
        }
        if !unstopped_node_execution_ids.is_empty() {
            self.restore_unstopped_pauses_to_running(
                app,
                execution_id,
                &unstopped_node_execution_ids,
            )
            .await
            .map_err(|compensation_error| {
                WorkflowRuntimeError::InvalidState(format!(
                    "{}; failed to restore unstopped paused nodes: {compensation_error}",
                    stop_failures.join("; ")
                ))
            })?;
            return Err(WorkflowRuntimeError::AgentSession(stop_failures.join("; ")));
        }
        Ok(())
    }

    async fn restore_unstopped_pauses_to_running<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        execution_id: &str,
        node_execution_ids: &HashSet<String>,
    ) -> Result<(), WorkflowRuntimeError> {
        let timestamp = current_timestamp();
        let (snapshot_before, candidate, events, worktree_path) = {
            let executions = self.executions.lock().await;
            let snapshot_before = executions
                .get(execution_id)
                .cloned()
                .ok_or_else(|| WorkflowRuntimeError::ExecutionNotFound(execution_id.to_string()))?;
            let mut candidate = snapshot_before.clone();
            let mut events = Vec::new();
            for node_execution_id in node_execution_ids {
                if candidate.resume_node_execution(node_execution_id, timestamp)
                    == crate::domain::workflow::entities::workflow_execution::TransitionOutcome::Applied
                {
                    events.push(WorkflowEvent::NodeResumed {
                        execution_id: execution_id.to_string(),
                        node_execution_id: node_execution_id.clone(),
                        timestamp,
                    });
                }
            }
            let worktree_path = candidate.worktree_path.clone();
            (snapshot_before, candidate, events, worktree_path)
        };
        if events.is_empty() {
            return Ok(());
        }
        let snapshot = self
            .commit_control_plane_candidate(
                app,
                ControlPlaneCommitCandidate {
                    execution_id,
                    snapshot_before,
                    candidate,
                    events: &events,
                    provider_events: Vec::new(),
                },
            )
            .await?;
        workflow_runtime_session::broadcast_state(app, &worktree_path, snapshot).await;
        Ok(())
    }

    /// Restore Agent Node Attempts that failed to activate after an in-place Resume.
    async fn restore_unactivated_resumes_to_paused<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        execution_id: &str,
        node_execution_ids: &HashSet<String>,
    ) -> Result<(), WorkflowRuntimeError> {
        if node_execution_ids.is_empty() {
            return Ok(());
        }
        let timestamp = current_timestamp();
        let (snapshot_before, candidate, events, worktree_path) = {
            let executions = self.executions.lock().await;
            let snapshot_before = executions
                .get(execution_id)
                .cloned()
                .ok_or_else(|| WorkflowRuntimeError::ExecutionNotFound(execution_id.to_string()))?;
            let mut candidate = snapshot_before.clone();
            let mut events = Vec::new();
            for node_execution_id in node_execution_ids {
                if candidate.pause_node_execution(node_execution_id, timestamp)
                    == crate::domain::workflow::entities::workflow_execution::TransitionOutcome::Applied
                {
                    events.push(WorkflowEvent::NodePaused {
                        execution_id: execution_id.to_string(),
                        node_execution_id: node_execution_id.clone(),
                        timestamp,
                    });
                }
            }
            let worktree_path = candidate.worktree_path.clone();
            (snapshot_before, candidate, events, worktree_path)
        };
        if events.is_empty() {
            return Ok(());
        }
        let snapshot = self
            .commit_control_plane_candidate(
                app,
                ControlPlaneCommitCandidate {
                    execution_id,
                    snapshot_before,
                    candidate,
                    events: &events,
                    provider_events: Vec::new(),
                },
            )
            .await?;
        workflow_runtime_session::broadcast_state(app, &worktree_path, snapshot).await;
        Ok(())
    }

    pub(crate) async fn resume_workflow_execution<R: tauri::Runtime + 'static>(
        &self,
        app: &tauri::AppHandle<R>,
        execution_id: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        let metadata = self.validate_execution_command_target(execution_id).await?;
        let ledger = self
            .worktree_ledger
            .snapshot_for_tree(execution_id)
            .map_err(|error| WorkflowRuntimeError::SessionStore(error.to_string()))?;
        if let Some(cause) = ledger.recovery_cause_for_tree(execution_id) {
            return Err(WorkflowRuntimeError::InvalidState(cause.to_string()));
        }
        if metadata.status != ExecutionStatus::Running {
            return Err(WorkflowRuntimeError::InvalidState(format!(
                "execution {execution_id} cannot be resumed from status {}",
                metadata.status.as_str()
            )));
        }
        let timestamp = current_timestamp();
        let (snapshot_before, candidate, events, resumed_sessions, paused_commands, worktree_path) = {
            let executions = self.executions.lock().await;
            let snapshot_before = executions
                .get(execution_id)
                .cloned()
                .ok_or_else(|| WorkflowRuntimeError::ExecutionNotFound(execution_id.to_string()))?;
            let mut candidate = snapshot_before.clone();
            let targets = candidate
                .node_executions
                .iter()
                .filter(|node| node.status == NodeExecutionStatus::Paused)
                .map(|node| (node.id.clone(), node.kind, node.session_id.clone()))
                .collect::<Vec<_>>();
            let mut events = Vec::new();
            let mut resumed_sessions = Vec::new();
            let mut paused_commands = Vec::new();
            for (node_execution_id, kind, session_id) in targets {
                if kind == NodeKindName::Command {
                    paused_commands.push(node_execution_id);
                    continue;
                }
                if candidate.resume_node_execution(&node_execution_id, timestamp)
                    != crate::domain::workflow::entities::workflow_execution::TransitionOutcome::Applied
                {
                    continue;
                }
                events.push(WorkflowEvent::NodeResumed {
                    execution_id: execution_id.to_string(),
                    node_execution_id: node_execution_id.clone(),
                    timestamp,
                });
                if kind == NodeKindName::Session {
                    let session_id = session_id.ok_or_else(|| {
                        WorkflowRuntimeError::InvalidState(format!(
                            "paused Session NodeExecution '{node_execution_id}' has no AgentSession"
                        ))
                    })?;
                    resumed_sessions.push((node_execution_id, session_id));
                }
            }
            let worktree_path = candidate.worktree_path.clone();
            (
                snapshot_before,
                candidate,
                events,
                resumed_sessions,
                paused_commands,
                worktree_path,
            )
        };
        if events.is_empty() && paused_commands.is_empty() {
            return Ok(());
        }
        let snapshot = if events.is_empty() {
            None
        } else {
            Some(
                self.commit_control_plane_candidate(
                    app,
                    ControlPlaneCommitCandidate {
                        execution_id,
                        snapshot_before,
                        candidate,
                        events: &events,
                        provider_events: Vec::new(),
                    },
                )
                .await?,
            )
        };
        let mut unactivated = resumed_sessions
            .iter()
            .map(|(node_execution_id, _)| node_execution_id.clone())
            .collect::<HashSet<_>>();
        for (node_execution_id, session_id) in resumed_sessions {
            let activation = self
                .workflow_agent_sessions
                .dispatch_initial_instruction(
                    &session_id,
                    &node_execution_id,
                    "Continue the paused workflow node from the existing conversation context.",
                )
                .await;
            if let Err(error) = activation {
                if let Err(compensation_error) = self
                    .restore_unactivated_resumes_to_paused(app, execution_id, &unactivated)
                    .await
                {
                    return Err(WorkflowRuntimeError::InvalidState(format!(
                        "{error}; failed to restore unactivated resumed nodes: {compensation_error}"
                    )));
                }
                return Err(error);
            }
            unactivated.remove(&node_execution_id);
        }
        for node_execution_id in paused_commands {
            self.restart_paused_command_node(app, execution_id, &node_execution_id)
                .await?;
        }
        if let Some(snapshot) = snapshot {
            workflow_runtime_session::broadcast_state(app, &worktree_path, snapshot).await;
        }
        Ok(())
    }
    pub(super) async fn validate_execution_command_target(
        &self,
        execution_id: &str,
    ) -> Result<WorkflowExecutionMetadata, WorkflowRuntimeError> {
        if self
            .execution_store
            .interrupted_transition_pending(execution_id)
            .await
        {
            return Err(WorkflowRuntimeError::InvalidState(format!(
                "execution {execution_id} already has a transition in progress"
            )));
        }
        let metadata = self
            .execution_store
            .get_execution_record(execution_id)
            .await
            .map_err(|error| {
                WorkflowRuntimeError::SessionStore(format!(
                    "canonical workflow execution read failed: {error}"
                ))
            })?
            .ok_or_else(|| WorkflowRuntimeError::ExecutionNotFound(execution_id.to_string()))?;
        let resolved = self
            .worktree_resolver
            .resolve(metadata.worktree_path.clone())
            .await
            .map_err(|error| WorkflowRuntimeError::UnauthorizedWorktree(error.to_string()))?;
        if resolved != metadata.worktree_path {
            return Err(WorkflowRuntimeError::UnauthorizedWorktree(format!(
                "execution {execution_id} targets '{}' but managed worktree resolves to '{resolved}'",
                metadata.worktree_path
            )));
        }
        if let Some(in_memory) = self.executions.lock().await.get(execution_id) {
            if in_memory.worktree_path != metadata.worktree_path {
                return Err(WorkflowRuntimeError::UnauthorizedWorktree(format!(
                    "execution {execution_id} worktree does not match persisted metadata"
                )));
            }
        }
        Ok(metadata)
    }

    /// ワークフローを中断する。
    /// `execution_id` を主語に workflow を中断する。
    ///
    /// Spec issues-1011 finding 2/10: 全経路で `executions.get_mut(execution_id)` を使い、
    /// worktree_path 経由の委譲を排除する。これにより、同一 worktree に terminal execution と
    /// active execution が共存しても誤って別 execution を中断する TOCTOU を構造的に排除する。
    ///
    /// Spec [04]: `AbortExecution` command handler の境界。
    /// - 対象 execution が存在しない場合は `AbortOutcome::NotFound` を返す（非受理）。
    /// - 既に terminal な execution の場合は `AbortOutcome::AlreadyTerminal` を返す（非受理）。
    /// - 実際に Aborted に遷移し ExecutionAborted event を必須 append できた場合のみ
    ///   `AbortOutcome::Aborted` を返す。
    ///
    /// ExecutionAborted event は `write_log_required` 経由で必須 append し、append 失敗時は
    /// mutation 直前 snapshot で `DomainWorkflowExecution` 全体を一括復元する
    /// （Spec atomic mutation 境界）。
    ///
    /// 外部から直接呼ばれることはなく、`abort_workflow_execution*` runtime primitive 経路のみが
    /// 利用する（Spec [04]: 内部呼び出し元も driver の private method を直接叩かない）。
    async fn commit_abort_workflow_by_execution_id<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        execution_id: &str,
        expected_node_name: Option<&str>,
    ) -> Result<AbortCommit, WorkflowRuntimeError> {
        // 1. 対象 execution の存在 + active 性を判定。
        //    非受理経路 (NotFound / AlreadyTerminal) ではどんな外部副作用も発生させない。
        let lookup = self.abort_target_lookup(execution_id).await?;
        let (current_node_session_id, active_node_session_ids) = match lookup {
            AbortTargetLookup::NotFound => {
                return Ok(AbortCommit::NotFound);
            }
            AbortTargetLookup::AlreadyTerminal => {
                return Ok(AbortCommit::AlreadyTerminal);
            }
            AbortTargetLookup::Active {
                current_node_session_id,
                active_node_session_ids,
            } => (current_node_session_id, active_node_session_ids),
        };
        let mut session_ids = current_node_session_id.into_iter().collect::<Vec<_>>();
        session_ids.extend(active_node_session_ids.into_iter().flatten());
        session_ids.sort();
        session_ids.dedup();
        // 2. [04] pre-commit (rollback 可能): mutation 直前 snapshot を取得し、
        //    state を Aborted に遷移させる。競合で terminal 化していた場合は
        //    AlreadyTerminal で返す。
        let timestamp = current_timestamp();
        let execution_store_snapshot_before = self
            .execution_store
            .active_execution_snapshot(execution_id)
            .await;
        let (snapshot_before, snapshot_state, aborted_node_for_event) = {
            let mut execs = self.executions.lock().await;
            let Some(exec) = execs.get_mut(execution_id) else {
                drop(execs);
                return Ok(if self.has_terminal_execution_record(execution_id).await? {
                    AbortCommit::AlreadyTerminal
                } else {
                    AbortCommit::NotFound
                });
            };
            if !exec.is_active() {
                return Ok(AbortCommit::AlreadyTerminal);
            }
            if let Some(expected_node_name) = expected_node_name {
                let current_node = exec.display_current_node();
                if current_node.as_deref() != Some(expected_node_name) {
                    return Err(WorkflowRuntimeError::UnauthorizedApprovalTarget(
                        "node does not match".to_string(),
                    ));
                }
            }
            let snapshot_before = exec.clone();
            let aborted_node_for_event = exec.display_current_node();

            // spec issues-1023: state を Aborted にする前に、中断時のアクティブ leaf を
            // `node_history` に "aborted" entry として記録する。UI 側は既存 history
            // 描画経路 + session_id で中断 node の session log にアクセスできる。
            exec.record_aborted_history_for_active_leaves(timestamp);

            let _ = exec.transition_aborted();
            exec.clear_node_stalls(timestamp);
            let snapshot_state = RuntimeCommitSnapshot::from_execution(exec)?;
            (snapshot_before, snapshot_state, aborted_node_for_event)
        };

        // 3. [04] commit point: ExecutionAborted を必須 append。失敗時は
        //    DomainWorkflowExecution / Execution Store / ChatSession を snapshot で一括復元する。
        //    interrupt_agent はこの時点ではまだ実行していないため、append 失敗時には
        //    rollback 不能な外部副作用が残らない。
        let aborted_event = WorkflowEvent::ExecutionAborted {
            execution_id: execution_id.to_string(),
            aborted_node: aborted_node_for_event,
            timestamp,
        };
        let commit_result = self
            .commit_required_events(
                app,
                RequiredEventCommit {
                    execution_id,
                    snapshot_for_commit: &snapshot_state,
                    snapshot_before,
                    execution_store_snapshot_before,
                    required_events: vec![aborted_event],
                    append_error_context: "ExecutionAborted log failed",
                },
            )
            .await;
        if let Err(error) = commit_result {
            // `commit_required_events` restores `snapshot_before` when the append itself fails.
            // If the exact Aborted snapshot is still current, the append succeeded and only the
            // post-commit ExecutionStore projection failed. ExecutionAborted is authoritative in
            // that case: keep cancelling runtime activation and run terminal cleanup instead of
            // rolling the paused activation back against an already-Aborted execution.
            let aborted_event_is_committed = self
                .executions
                .lock()
                .await
                .get(execution_id)
                .is_some_and(|current| commit_snapshot_is_current(current, &snapshot_state));
            if !aborted_event_is_committed {
                return Err(error);
            }
            log::warn!(
                "ExecutionAborted metadata projection failed for {execution_id} after durable append; continuing terminal cleanup: {error}"
            );
        }
        crate::other::telemetry::record_workflow_node_failure(
            FailureClassification::new(NodeExecutionFailureKind::UserAbort),
            None,
        );

        Ok(AbortCommit::Aborted { session_ids })
    }

    async fn finish_committed_abort<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        execution_id: &str,
        session_ids: &[String],
    ) -> Result<(), WorkflowRuntimeError> {
        // ExecutionAborted is durable before this method is called. Runtime activation must be
        // quiesced before entering this terminal cleanup so it cannot recreate a closed runtime.
        self.shutdown_active_commands_for_execution(execution_id)
            .await;
        let mut interrupt_failures = Vec::new();
        for session_id in session_ids {
            if let Err(error) = self
                .workflow_agent_sessions
                .interrupt_workflow_agent_session(session_id)
                .await
            {
                interrupt_failures.push(error.to_string());
            }
        }
        self.finalize_terminal_transition_after_required_append(app, execution_id)
            .await;
        if interrupt_failures.is_empty() {
            Ok(())
        } else {
            Err(WorkflowRuntimeError::AgentSession(
                interrupt_failures.join("; "),
            ))
        }
    }

    /// `abort_workflow_by_execution_id` の post-commit 区間。state は呼出し前に Aborted に
    /// 遷移済みで、`ExecutionAborted` event は必須 append 済み、かつ Execution Store sync も
    /// 完了済みである前提。Workflow refs cleanup / broadcast / in-memory runtime releaseを
    /// 実行する。AgentSessionとPTYはWorkflow終端後もユーザー操作のため保持する。
    ///
    /// [04] post-commit 失敗は warn ログのみで command 結果に伝播させない。観測可能な
    /// 事実は既に ExecutionAborted で確定しており、ここでの副作用失敗を command failure に
    /// 射影すると spec [04] の「post-commit 失敗は command failure として返さない」に
    /// 違反するため。
    pub(super) async fn finalize_terminal_transition_after_required_append<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        execution_id: &str,
    ) {
        let (snapshot, worktree_path) = {
            let execs = self.executions.lock().await;
            let Some(exec) = execs.get(execution_id) else {
                return;
            };
            let snapshot = match RuntimeCommitSnapshot::from_execution(exec) {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    log::warn!("terminal transition cleanup skipped for '{execution_id}': {error}");
                    return;
                }
            };
            (snapshot, exec.worktree_path.clone())
        };

        self.cleanup_session_workflow_refs_by_execution_id(execution_id)
            .await;
        workflow_runtime_session::broadcast_state(app, &worktree_path, snapshot).await;
        self.release_terminal_execution(execution_id).await;
    }

    pub(super) async fn abort_target_lookup(
        &self,
        execution_id: &str,
    ) -> Result<AbortTargetLookup, WorkflowRuntimeError> {
        {
            let execs = self.executions.lock().await;
            if let Some(exec) = execs.get(execution_id) {
                if !exec.is_active() {
                    return Ok(AbortTargetLookup::AlreadyTerminal);
                }
                let current_node_session_id = exec.current_session_id.clone();
                let active_node_session_ids = Some(
                    exec.node_executions
                        .iter()
                        .filter(|node| node.status.is_active())
                        .filter_map(|node| node.session_id.clone())
                        .collect::<Vec<_>>(),
                );
                return Ok(AbortTargetLookup::Active {
                    current_node_session_id,
                    active_node_session_ids,
                });
            }
        }
        if self.has_terminal_execution_record(execution_id).await? {
            Ok(AbortTargetLookup::AlreadyTerminal)
        } else {
            Ok(AbortTargetLookup::NotFound)
        }
    }

    pub(super) async fn has_terminal_execution_record(
        &self,
        execution_id: &str,
    ) -> Result<bool, WorkflowRuntimeError> {
        Ok(self
            .execution_store
            .get_execution_record(execution_id)
            .await
            .map_err(|error| {
                WorkflowRuntimeError::SessionStore(format!(
                    "canonical workflow execution read failed: {error}"
                ))
            })?
            .is_some_and(|execution| execution.status.is_terminal()))
    }
}
