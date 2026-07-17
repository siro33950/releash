use super::*;

enum AbortCommit {
    Aborted { session_ids: Vec<String> },
    NotFound,
    AlreadyTerminal,
}

fn abort_outcome_to_command_result(
    outcome: AbortOutcome,
    execution_id: &str,
) -> Result<(), WorkflowEngineError> {
    match outcome {
        AbortOutcome::Aborted => Ok(()),
        AbortOutcome::NotFound => Err(WorkflowEngineError::ExecutionNotFound(
            execution_id.to_string(),
        )),
        AbortOutcome::AlreadyTerminal => Err(WorkflowEngineError::InvalidState(format!(
            "execution {execution_id} is already terminal"
        ))),
    }
}

/// abort / stop / resume の execution ライフサイクル typed command 群。
impl WorkflowRuntimeService {
    pub(crate) async fn abort_workflow_execution<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        execution_id: &str,
        expected_node_name: Option<&str>,
    ) -> Result<(), WorkflowEngineError> {
        let metadata = self.validate_execution_command_target(execution_id).await?;
        if let Some(expected_node_name) = expected_node_name {
            let target_node = if metadata.status == ExecutionStatus::Interrupted {
                metadata.resume_from_node.as_deref()
            } else {
                metadata.current_node.as_deref()
            };
            if target_node != Some(expected_node_name) {
                return Err(WorkflowEngineError::UnauthorizedApprovalTarget(
                    "node does not match".to_string(),
                ));
            }
        }
        if metadata.status == ExecutionStatus::Interrupted {
            let timestamp = current_timestamp();
            let data_dir = match self.execution_store.configured_data_dir().await {
                Some(data_dir) => data_dir,
                None => crate::infrastructure::platform::app_data_dir::resolve_data_dir(app)
                    .map_err(|error| {
                        WorkflowEngineError::SessionStore(format!("resolve_data_dir: {error}"))
                    })?,
            };
            let events = WorkflowEventLog::new(&data_dir)
                .read_log(execution_id)
                .map_err(WorkflowEngineError::SessionStore)?;
            let projected =
                crate::adaptor::gateway::workflow::event_projection::project_workflow_execution(
                    execution_id,
                    &events,
                )
                .map_err(WorkflowEngineError::InvalidState)?
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(execution_id.to_string()))?;
            if projected.status != ExecutionStatus::Interrupted {
                return Err(WorkflowEngineError::InvalidState(format!(
                    "execution {execution_id} event log has status {} instead of interrupted",
                    projected.status.as_str()
                )));
            }
            if projected.worktree_path != metadata.worktree_path
                || projected.resume_from_node != metadata.resume_from_node
            {
                return Err(WorkflowEngineError::UnauthorizedWorktree(format!(
                    "execution {execution_id} metadata does not match its event-log checkpoint"
                )));
            }
            let reservation = self
                .execution_store
                .reserve_interrupted_for_abort(execution_id, timestamp)
                .await
                .map_err(|error| match error {
                    ExecutionStoreError::ExecutionNotFound { .. } => {
                        WorkflowEngineError::ExecutionNotFound(execution_id.to_string())
                    }
                    ExecutionStoreError::InvalidStatusTransition { .. }
                    | ExecutionStoreError::TransitionInProgress { .. } => {
                        WorkflowEngineError::InvalidState(error.to_string())
                    }
                    other => WorkflowEngineError::SessionStore(format!(
                        "ExecutionStore interrupted abort reservation failed: {other}"
                    )),
                })?;
            let append_result = self.write_log_required_batch(
                app,
                &[WorkflowEvent::ExecutionAborted {
                    execution_id: execution_id.to_string(),
                    aborted_node: metadata.resume_from_node.clone(),
                    timestamp,
                }],
            );
            if let Err(error) = append_result {
                self.execution_store
                    .rollback_interrupted_abort(reservation)
                    .await
                    .map_err(|rollback_error| {
                        WorkflowEngineError::SessionStore(format!(
                            "ExecutionAborted log failed: {error}; interrupted metadata rollback failed: {rollback_error}"
                        ))
                    })?;
                return Err(WorkflowEngineError::SessionStore(format!(
                    "ExecutionAborted log failed: {error}"
                )));
            }
            if let Err(error) = self
                .execution_store
                .commit_interrupted_abort(reservation)
                .await
            {
                // Event log と persisted metadata は既に Aborted で一致している。reservation
                // cleanup の失敗を accepted command の失敗へ射影しない。
                log::warn!(
                    "ExecutionStore interrupted abort reservation cleanup failed for {execution_id}: {error}"
                );
            }
            return Ok(());
        }
        if !metadata.status.is_active() {
            return Err(WorkflowEngineError::InvalidState(format!(
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
                    WorkflowEngineError::ExecutionNotFound(execution_id.to_string())
                }
                ExecutionStoreError::InvalidStatusTransition { .. }
                | ExecutionStoreError::TransitionInProgress { .. } => {
                    WorkflowEngineError::InvalidState(error.to_string())
                }
                other => WorkflowEngineError::SessionStore(format!(
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
            .commit_abort_workflow_by_execution_id(
                app,
                session_store,
                execution_id,
                expected_node_name,
            )
            .await;
        match abort_result {
            Ok(AbortCommit::Aborted { session_ids }) => {
                if activation_was_paused {
                    activation_gate.commit_cancel();
                    activation_guard = Some(activation_gate.lock.lock().await);
                }
                let _activation_guard = activation_guard;
                self.finish_committed_abort(app, agent_runtime, execution_id, &session_ids)
                    .await;
                self.execution_store
                    .finish_active_interruption(interruption_reservation)
                    .await
                    .map_err(|error| {
                        WorkflowEngineError::SessionStore(format!(
                            "ExecutionStore abort reservation cleanup failed: {error}"
                        ))
                    })?;
                abort_outcome_to_command_result(AbortOutcome::Aborted, execution_id)
            }
            Ok(AbortCommit::NotFound) => {
                self.execution_store
                    .finish_active_interruption(interruption_reservation)
                    .await
                    .map_err(|error| {
                        WorkflowEngineError::SessionStore(format!(
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
                        WorkflowEngineError::SessionStore(format!(
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
                    WorkflowEngineError::SessionStore(format!(
                        "abort failed: {error}; abort reservation rollback failed: {reservation_error}"
                    ))
                })?;
                Err(error)
            }
        }
    }

    /// Explicit stop command. The accepted fact is `ExecutionInterrupted(Stop)`; process and
    /// session cancellation happen only after that fact and its metadata projection commit.
    pub(crate) async fn stop_workflow_execution<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        execution_id: &str,
    ) -> Result<(), WorkflowEngineError> {
        let metadata = self.validate_execution_command_target(execution_id).await?;
        if !matches!(
            metadata.status,
            ExecutionStatus::Running | ExecutionStatus::WaitingApproval
        ) {
            return Err(WorkflowEngineError::InvalidState(format!(
                "execution {execution_id} cannot be stopped from status {}",
                metadata.status.as_str()
            )));
        }
        if !self
            .interrupt_active_execution(
                app,
                agent_runtime,
                execution_id,
                ExecutionInterruptionReason::Stop,
            )
            .await?
        {
            return Err(WorkflowEngineError::InvalidState(format!(
                "execution {execution_id} is not active"
            )));
        }
        Ok(())
    }

    /// Rebuilds an interrupted execution exclusively from its append-only event stream and starts
    /// a fresh runtime for the first unconfirmed node attempt.
    pub(crate) async fn resume_workflow_execution<R: tauri::Runtime + 'static>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        execution_id: &str,
    ) -> Result<(), WorkflowEngineError> {
        resume_orchestration::resume_workflow_execution(
            self,
            app,
            session_store,
            agent_runtime,
            execution_id,
        )
        .await
    }
    pub(super) async fn validate_execution_command_target(
        &self,
        execution_id: &str,
    ) -> Result<WorkflowExecutionMetadata, WorkflowEngineError> {
        if self
            .execution_store
            .interrupted_transition_pending(execution_id)
            .await
        {
            return Err(WorkflowEngineError::InvalidState(format!(
                "execution {execution_id} already has a transition in progress"
            )));
        }
        let metadata = self
            .execution_store
            .get_execution_record(execution_id)
            .await
            .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(execution_id.to_string()))?;
        let resolved = self
            .worktree_resolver
            .resolve(metadata.worktree_path.clone())
            .await
            .map_err(|error| WorkflowEngineError::UnauthorizedWorktree(error.to_string()))?;
        if resolved != metadata.worktree_path {
            return Err(WorkflowEngineError::UnauthorizedWorktree(format!(
                "execution {execution_id} targets '{}' but managed worktree resolves to '{resolved}'",
                metadata.worktree_path
            )));
        }
        if let Some(in_memory) = self.executions.lock().await.get(execution_id) {
            if in_memory.worktree_path != metadata.worktree_path {
                return Err(WorkflowEngineError::UnauthorizedWorktree(format!(
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
    /// mutation 直前 snapshot で `WorkflowExecution` 全体を一括復元する
    /// （Spec atomic mutation 境界）。
    ///
    /// 外部から直接呼ばれることはなく、`abort_workflow_execution*` runtime primitive 経路のみが
    /// 利用する（Spec [04]: 内部呼び出し元も engine の private method を直接叩かない）。
    #[cfg(test)]
    pub(super) async fn abort_workflow_by_execution_id<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        execution_id: &str,
        expected_node_name: Option<&str>,
    ) -> Result<AbortOutcome, WorkflowEngineError> {
        let commit = self
            .commit_abort_workflow_by_execution_id(
                app,
                session_store,
                execution_id,
                expected_node_name,
            )
            .await?;
        match commit {
            AbortCommit::Aborted { session_ids } => {
                self.finish_committed_abort(app, agent_runtime, execution_id, &session_ids)
                    .await;
                Ok(AbortOutcome::Aborted)
            }
            AbortCommit::NotFound => Ok(AbortOutcome::NotFound),
            AbortCommit::AlreadyTerminal => Ok(AbortOutcome::AlreadyTerminal),
        }
    }

    async fn commit_abort_workflow_by_execution_id<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        execution_id: &str,
        expected_node_name: Option<&str>,
    ) -> Result<AbortCommit, WorkflowEngineError> {
        // 1. 対象 execution の存在 + active 性を判定。
        //    非受理経路 (NotFound / AlreadyTerminal) ではどんな外部副作用も発生させない。
        let lookup = self.abort_target_lookup(execution_id).await;
        let (current_node_session_id, fanout_session_ids) = match lookup {
            AbortTargetLookup::NotFound => {
                return Ok(AbortCommit::NotFound);
            }
            AbortTargetLookup::AlreadyTerminal => {
                return Ok(AbortCommit::AlreadyTerminal);
            }
            AbortTargetLookup::Active {
                current_node_session_id,
                fanout_session_ids,
            } => (current_node_session_id, fanout_session_ids),
        };
        let mut session_ids = current_node_session_id.into_iter().collect::<Vec<_>>();
        session_ids.extend(fanout_session_ids.into_iter().flatten());
        session_ids.sort();
        session_ids.dedup();
        #[cfg(test)]
        self.wait_abort_after_lookup_for_test().await;

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
                return Ok(if self.has_terminal_execution_record(execution_id).await {
                    AbortCommit::AlreadyTerminal
                } else {
                    AbortCommit::NotFound
                });
            };
            if !exec.is_active() {
                return Ok(AbortCommit::AlreadyTerminal);
            }
            if let Some(expected_node_name) = expected_node_name {
                let current_node = exec
                    .workflow
                    .nodes
                    .get(exec.current_node_index)
                    .map(|node| node.name.as_str())
                    .ok_or_else(|| {
                        WorkflowEngineError::InvalidState(format!(
                            "execution {execution_id} has invalid current node"
                        ))
                    })?;
                if expected_node_name != current_node {
                    return Err(WorkflowEngineError::UnauthorizedApprovalTarget(
                        "node does not match".to_string(),
                    ));
                }
            }
            let snapshot_before = exec.clone();
            let aborted_node_for_event = exec
                .fanout_runtime
                .as_ref()
                .map(|fanout| fanout.parent_node_name.clone())
                .or_else(|| {
                    exec.workflow
                        .nodes
                        .get(exec.current_node_index)
                        .map(|node| node.name.clone())
                });

            // spec issues-1023: state を Aborted にする前に、中断時の current node /
            // fanout children を `node_history` に "aborted" entry として記録する。
            // これにより UI 側は既存 history 描画経路 + session_id を使って中断 node の
            // session log にアクセスできるようになる。`exec.fanout_runtime = None` を
            // 明示クリアして `to_commit_snapshot()` 経由の二重表示を防ぐ。
            if exec.fanout_runtime.is_some() {
                if let Some(entry) = exec.make_aborted_fanout_history_entry(timestamp) {
                    exec.node_history.push(entry);
                }
                exec.fanout_runtime = None;
            } else {
                let current_node_name = exec.workflow.nodes[exec.current_node_index].name.clone();
                let current_attempt = exec
                    .node_execution_counts
                    .get(&current_node_name)
                    .copied()
                    .unwrap_or(1);
                let already_in_history = exec.node_history.last().is_some_and(|e| {
                    e.node_name == current_node_name && e.attempt == current_attempt
                });
                if !already_in_history {
                    let entry = exec.make_aborted_history_entry(timestamp);
                    exec.node_history.push(entry);
                }
            }

            exec.state = RuntimeExecutionState::Aborted;
            exec.current_stall_observations.clear();
            exec.updated_at = timestamp;
            let snapshot_state = exec.to_commit_snapshot();
            (snapshot_before, snapshot_state, aborted_node_for_event)
        };

        // 3. [04] commit point: ExecutionAborted を必須 append。失敗時は
        //    WorkflowExecution / Execution Store / ChatSession を snapshot で一括復元する。
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
                session_store,
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
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        execution_id: &str,
        session_ids: &[String],
    ) {
        // ExecutionAborted is durable before this method is called. Runtime activation must be
        // quiesced before entering this terminal cleanup so it cannot recreate a closed runtime.
        self.shutdown_active_commands_for_execution(execution_id)
            .await;
        for session_id in session_ids {
            workflow_runtime_session::interrupt_agent(agent_runtime, session_id).await;
        }
        self.finalize_terminal_transition_after_required_append(app, agent_runtime, execution_id)
            .await;
    }

    /// `abort_workflow_by_execution_id` の post-commit 区間。state は呼出し前に Aborted に
    /// 遷移済みで、`ExecutionAborted` event は必須 append 済み、かつ Execution Store sync も
    /// 完了済みである前提。ChatSession persist / node session release / refs cleanup /
    /// broadcast を実行する。
    ///
    /// [04] post-commit 失敗は warn ログのみで command 結果に伝播させない。観測可能な
    /// 事実は既に ExecutionAborted で確定しており、ここでの副作用失敗を command failure に
    /// 射影すると spec [04] の「post-commit 失敗は command failure として返さない」に
    /// 違反するため。
    pub(super) async fn finalize_terminal_transition_after_required_append<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        execution_id: &str,
    ) {
        let (snapshot, worktree_path) = {
            let execs = self.executions.lock().await;
            let Some(exec) = execs.get(execution_id) else {
                return;
            };
            (exec.to_commit_snapshot(), exec.worktree_path.clone())
        };

        // terminal session の release と refs cleanup。
        let terminal_session_ids = workflow_runtime_commit::terminal_node_session_ids(&snapshot);
        workflow_runtime_session::release_completed_node_sessions(
            agent_runtime,
            &terminal_session_ids,
        )
        .await;
        self.cleanup_session_workflow_refs_by_execution_id(execution_id)
            .await;
        workflow_runtime_session::broadcast_state(app, &worktree_path, snapshot).await;
        self.release_terminal_execution(execution_id).await;
    }

    pub(super) async fn abort_target_lookup(&self, execution_id: &str) -> AbortTargetLookup {
        {
            let execs = self.executions.lock().await;
            if let Some(exec) = execs.get(execution_id) {
                if !exec.is_active() {
                    return AbortTargetLookup::AlreadyTerminal;
                }
                let current_node_session_id = exec.current_session_id.clone();
                let fanout_session_ids = exec.fanout_runtime.as_ref().map(|pr| {
                    pr.children
                        .iter()
                        .filter(|c| c.state == FanoutChildRuntimeState::Running)
                        .map(|c| c.session_id.clone())
                        .collect::<Vec<_>>()
                });
                return AbortTargetLookup::Active {
                    current_node_session_id,
                    fanout_session_ids,
                };
            }
        }
        if self.has_terminal_execution_record(execution_id).await {
            AbortTargetLookup::AlreadyTerminal
        } else {
            AbortTargetLookup::NotFound
        }
    }

    pub(super) async fn has_terminal_execution_record(&self, execution_id: &str) -> bool {
        self.execution_store
            .get_execution_record(execution_id)
            .await
            .is_some_and(|execution| execution.status.is_terminal())
    }
}
