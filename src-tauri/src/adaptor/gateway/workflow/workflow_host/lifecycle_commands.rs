//! Abort orchestration.

use super::*;

enum AbortCommit {
    Aborted,
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

/// Abort の execution ライフサイクル typed command。
impl WorkflowRuntimeHost {
    pub(crate) async fn stop_execution_tree_processes(
        &self,
        app: &WorkflowRuntimeDependencies,
        execution_id: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        self.shutdown_active_commands_for_execution(execution_id)
            .await;
        let store = app.store.clone().ok_or_else(|| {
            WorkflowRuntimeError::SessionStore("fact store unavailable".to_string())
        })?;
        let folded = workflow_fact_log::fold_tree_from(
            &workflow_fact_log::FactLogReadBackend::Live(store),
            execution_id,
        )
        .map_err(WorkflowRuntimeError::SessionStore)?
        .ok_or_else(|| WorkflowRuntimeError::ExecutionNotFound(execution_id.to_string()))?;
        for node in &folded.aggregate.node_executions {
            if let Some(session_id) = &node.session_id {
                self.workflow_agent_sessions
                    .stop_agent_session_for_terminal_node_preserving_checkpoint(
                        session_id, &node.id,
                    )
                    .await?;
            }
        }
        Ok(())
    }

    pub(crate) async fn abort_workflow_execution(
        &self,
        app: &WorkflowRuntimeDependencies,
        execution_id: &str,
        expected_node_name: Option<&str>,
    ) -> Result<(), WorkflowRuntimeError> {
        let (current_node, status) = self
            .validate_execution_command_target(app, execution_id)
            .await?;
        if let Some(expected_node_name) = expected_node_name {
            if current_node.as_deref() != Some(expected_node_name) {
                return Err(WorkflowRuntimeError::UnauthorizedApprovalTarget(
                    "node does not match".to_string(),
                ));
            }
        }
        if !status.is_active() {
            return Err(WorkflowRuntimeError::InvalidState(format!(
                "execution {execution_id} cannot be aborted from status {}",
                status.as_str()
            )));
        }
        let interruption_reservation = if self
            .executions
            .lock()
            .await
            .get(execution_id)
            .is_some_and(|execution| execution.launched_as == ExecutionTreeLaunch::Session)
        {
            None
        } else {
            Some(
                self.execution_store
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
                    })?,
            )
        };
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
            Ok(AbortCommit::Aborted) => {
                if activation_was_paused {
                    activation_gate.commit_cancel();
                    activation_guard = Some(activation_gate.lock.lock().await);
                }
                let _activation_guard = activation_guard;
                self.finish_committed_abort(app, execution_id).await;
                self.finish_abort_interruption(interruption_reservation)
                    .await
                    .map_err(|error| {
                        WorkflowRuntimeError::SessionStore(format!(
                            "ExecutionStore abort reservation cleanup failed: {error}"
                        ))
                    })?;
                abort_outcome_to_command_result(AbortOutcome::Aborted, execution_id)
            }
            Ok(AbortCommit::NotFound) => {
                self.finish_abort_interruption(interruption_reservation)
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
                self.finish_abort_interruption(interruption_reservation)
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
                    .finish_abort_interruption(interruption_reservation)
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

    async fn finish_abort_interruption(
        &self,
        reservation: Option<
            crate::adaptor::gateway::workflow::execution_store::ActiveInterruptionReservation,
        >,
    ) -> Result<(), ExecutionStoreError> {
        if let Some(reservation) = reservation {
            self.execution_store
                .finish_active_interruption(reservation)
                .await?;
        }
        Ok(())
    }

    async fn validate_execution_command_target(
        &self,
        app: &WorkflowRuntimeDependencies,
        execution_id: &str,
    ) -> Result<(Option<String>, ExecutionStatus), WorkflowRuntimeError> {
        if self
            .execution_store
            .interrupted_transition_pending(execution_id)
            .await
        {
            return Err(WorkflowRuntimeError::InvalidState(format!(
                "execution {execution_id} already has a transition in progress"
            )));
        }
        let (worktree_path, current_node, status) = if let Some(store) = &app.store {
            let tree = workflow_fact_log::fold_tree_from(
                &workflow_fact_log::FactLogReadBackend::Live(store.clone()),
                execution_id,
            )
            .map_err(WorkflowRuntimeError::SessionStore)?
            .ok_or_else(|| WorkflowRuntimeError::ExecutionNotFound(execution_id.into()))?;
            let model = crate::domain::workflow::services::fact_replay::derive_read_model(&tree);
            (model.worktree_path, model.current_node, model.status)
        } else {
            let metadata = self
                .execution_store
                .get_execution_record(execution_id)
                .await
                .map_err(|error| {
                    WorkflowRuntimeError::SessionStore(format!(
                        "canonical workflow execution read failed: {error}"
                    ))
                })?
                .ok_or_else(|| WorkflowRuntimeError::ExecutionNotFound(execution_id.into()))?;
            (
                metadata.worktree_path,
                metadata.current_node,
                metadata.status,
            )
        };
        if let Some(in_memory) = self.executions.lock().await.get(execution_id) {
            if in_memory.worktree_path != worktree_path {
                return Err(WorkflowRuntimeError::UnauthorizedWorktree(format!(
                    "execution {execution_id} worktree does not match persisted metadata"
                )));
            }
        }
        Ok((current_node, status))
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
    /// mutation 直前 snapshot で `DomainExecutionTree` 全体を一括復元する
    /// （Spec atomic mutation 境界）。
    ///
    /// 外部から直接呼ばれることはなく、`abort_workflow_execution*` runtime primitive 経路のみが
    /// 利用する（Spec [04]: 内部呼び出し元も driver の private method を直接叩かない）。
    async fn commit_abort_workflow_by_execution_id(
        &self,
        app: &WorkflowRuntimeDependencies,
        execution_id: &str,
        expected_node_name: Option<&str>,
    ) -> Result<AbortCommit, WorkflowRuntimeError> {
        // 1. 対象 execution の存在 + active 性を判定。
        //    非受理経路 (NotFound / AlreadyTerminal) ではどんな外部副作用も発生させない。
        let lookup = self.abort_target_lookup(execution_id).await?;
        match lookup {
            AbortTargetLookup::NotFound => {
                return Ok(AbortCommit::NotFound);
            }
            AbortTargetLookup::AlreadyTerminal => {
                return Ok(AbortCommit::AlreadyTerminal);
            }
            AbortTargetLookup::Active => {}
        }
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
            exec.abort_active_node_executions(timestamp);
            exec.clear_node_stalls(timestamp);
            let snapshot_state = RuntimeCommitSnapshot::from_execution(exec)?;
            (snapshot_before, snapshot_state, aborted_node_for_event)
        };

        // 3. [04] commit point: ExecutionAborted を必須 append。失敗時は
        //    DomainExecutionTree / Execution Store / ChatSession を snapshot で一括復元する。
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

        Ok(AbortCommit::Aborted)
    }

    async fn finish_committed_abort(&self, app: &WorkflowRuntimeDependencies, execution_id: &str) {
        self.cancel_startup_retries(execution_id).await;
        // ExecutionAborted is durable before this method is called. Runtime activation must be
        // quiesced before entering this terminal cleanup so it cannot recreate a closed runtime.
        self.shutdown_active_commands_for_execution(execution_id)
            .await;
        self.finalize_terminal_transition_after_required_append(app, execution_id)
            .await;
    }

    /// `abort_workflow_by_execution_id` の post-commit 区間。state は呼出し前に Aborted に
    /// 遷移済みで、`ExecutionAborted` event は必須 append 済み、かつ Execution Store sync も
    /// 完了済みである前提。broadcast / in-memory runtime releaseを
    /// 実行する。終端した Session の AgentSession は required append 後の effect で
    /// checkpoint を保持したまま停止済みである。
    ///
    /// [04] post-commit 失敗は warn ログのみで command 結果に伝播させない。観測可能な
    /// 事実は既に ExecutionAborted で確定しており、ここでの副作用失敗を command failure に
    /// 射影すると spec [04] の「post-commit 失敗は command failure として返さない」に
    /// 違反するため。
    pub(super) async fn finalize_terminal_transition_after_required_append(
        &self,
        app: &WorkflowRuntimeDependencies,
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
                return Ok(AbortTargetLookup::Active);
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
