use base64::Engine as _;

impl SessionStore {
    #[cfg(test)]
    pub fn append_session_events(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        events: &[AgentSessionEvent],
    ) -> Result<(), String> {
        #[cfg(test)]
        if let Some(hook) = self.append_event_hook.read().clone() {
            for event in events {
                hook(session_id, event)?;
            }
        }
        self.commit_agent_events(app_data_dir, session_id, events)?;
        #[cfg(test)]
        if !self.canonical_authority_active() {
            self.test_storage()
                .append_session_events(app_data_dir, session_id, events)?;
            if self.test_storage().take_event_log_recovered(session_id) {
                self.notify_event_log_recovered(session_id);
            }
        }
        Ok(())
    }

    pub(crate) fn append_session_events_from_user(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        events: &[AgentSessionEvent],
    ) -> Result<(), String> {
        #[cfg(test)]
        if let Some(hook) = self.append_event_hook.read().clone() {
            for event in events {
                hook(session_id, event)?;
            }
        }
        self.commit_agent_events_with_kind(
            app_data_dir,
            session_id,
            events,
            crate::domain::local_event::CommitOperationKind::UserMutation,
        )?;
        #[cfg(test)]
        if !self.canonical_authority_active() {
            self.test_storage()
                .append_session_events(app_data_dir, session_id, events)?;
            if self.test_storage().take_event_log_recovered(session_id) {
                self.notify_event_log_recovered(session_id);
            }
        }
        Ok(())
    }

    pub fn load_previous_human_message_before_agent(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        agent_message_id: &str,
    ) -> Result<Option<ChatMessage>, String> {
        #[cfg(test)]
        if !self.canonical_authority_active() {
            return self
                .test_storage()
                .load_previous_human_message_before_agent(
                    app_data_dir,
                    session_id,
                    agent_message_id,
                );
        }
        let Some(session) = self.load_full_session_for_restore(app_data_dir, session_id)? else {
            return Ok(None);
        };
        let Some(agent_index) = session
            .messages
            .iter()
            .position(|message| message.id == agent_message_id)
        else {
            return Ok(None);
        };
        Ok(session.messages[..agent_index]
            .iter()
            .rev()
            .find(|message| message.role == super::MessageRole::Human)
            .cloned())
    }

    /// workflow step session のセットアップ失敗時に、作成済みの子 session を
    /// 取り除くロールバック経路。storage 層へ削除を委譲する。
    pub(crate) fn remove_session_for_rollback(
        &self,
        _app_data_dir: &Path,
        session_id: &str,
    ) -> Result<(), String> {
        self.ensure_canonical_mutation_admission()?;
        self.remove_read_session_projection(session_id)?;
        #[cfg(test)]
        if !self.canonical_authority_active() {
            self.test_storage()
                .remove_session(_app_data_dir, session_id);
        }
        Ok(())
    }

    #[cfg(test)]
    pub fn list_worktree_sessions(
        &self,
        app_data_dir: &Path,
        worktree_path: &str,
    ) -> Result<Vec<ChatSession>, String> {
        Ok(self
            .read_session_metadata_inventory(app_data_dir)?
            .into_iter()
            .filter(|session| same_worktree_path(&session.worktree_path, worktree_path))
            .map(|meta| meta.to_session(Vec::new()))
            .collect())
    }

    pub fn list_worktree_sessions_full(
        &self,
        app_data_dir: &Path,
        worktree_path: &str,
    ) -> Result<Vec<ChatSession>, String> {
        let ids = self
            .read_session_metadata_inventory(app_data_dir)?
            .into_iter()
            .filter(|session| same_worktree_path(&session.worktree_path, worktree_path))
            .map(|session| session.id)
            .collect::<Vec<_>>();
        ids.into_iter()
            .filter_map(|id| {
                self.load_full_session_for_restore(app_data_dir, &id)
                    .transpose()
            })
            .collect()
    }

    /// Full-session replacement for cold paths that own a complete `ChatSession`.
    ///
    /// Do not pass shell/page sessions returned by `get_session_shell` or `get_session_page`.
    /// Normal runtime updates must use `append_message`, `persist_message_parts`, or meta-only
    /// update methods so page-external message chunks cannot be removed by partial input.
    pub fn save_full_session_for_restore(
        &self,
        app_data_dir: &Path,
        session: &ChatSession,
    ) -> Result<(), String> {
        self.save_full_session_with_kind(
            app_data_dir,
            session,
            crate::domain::local_event::CommitOperationKind::Projection,
        )
    }

    pub(crate) fn save_full_session_from_user(
        &self,
        app_data_dir: &Path,
        session: &ChatSession,
    ) -> Result<(), String> {
        self.save_full_session_with_kind(
            app_data_dir,
            session,
            crate::domain::local_event::CommitOperationKind::UserMutation,
        )
    }

    fn save_full_session_with_kind(
        &self,
        app_data_dir: &Path,
        session: &ChatSession,
        operation_kind: crate::domain::local_event::CommitOperationKind,
    ) -> Result<(), String> {
        self.ensure_canonical_mutation_admission()?;
        let permission_mode =
            crate::domain::agent_session::PermissionMode::parse(&session.permission_mode)
                .map_err(|e| e.to_string())?;
        let normalized_session;
        let session = if session.permission_mode == permission_mode.as_str() {
            session
        } else {
            normalized_session = {
                let mut session = session.clone();
                session.permission_mode = permission_mode.as_str().to_string();
                session
            };
            &normalized_session
        };

        #[cfg(test)]
        if let Some(hook) = self.save_hook.read().clone() {
            hook(session)?;
        }

        let previous_projection = self.read_session_projection(&session.id)?;
        let previous_state = previous_projection
            .as_ref()
            .map(|projection| projection.meta.state);
        let previous_title = previous_projection
            .as_ref()
            .and_then(|projection| projection.title.clone());
        let previous_queue_paused_at = previous_projection
            .as_ref()
            .and_then(|projection| projection.queue_paused_at);
        let reducer_events = previous_projection
            .as_ref()
            .map(|projection| projection.reducer_events.clone())
            .unwrap_or_default();
        let pending_send_queue = previous_projection
            .map(|projection| projection.pending_send_queue)
            .unwrap_or_default();
        let saved_meta = SessionMeta::from_session(session);
        self.commit_session_projection_snapshot_with_kind(
            CanonicalAgentSessionProjection {
                meta: saved_meta,
                title: previous_title,
                messages: session.messages.clone(),
                reducer_events,
                queue_paused_at: previous_queue_paused_at,
                latest_token_usage: None,
                pending_send_queue,
            },
            operation_kind,
        )?;
        #[cfg(test)]
        if !self.canonical_authority_active() {
            self.test_storage()
                .save_full_session_for_restore(app_data_dir, session)?;
        }
        if previous_state.as_ref() != Some(&session.state) {
            let revision = self.require_meta(app_data_dir, &session.id)?.state_revision;
            self.notify_state_change(
                &session.id,
                &session.worktree_path,
                &session.state,
                revision,
            );
        }
        Ok(())
    }

    /// `SessionState` または Error 理由 projection の変更を購読するリスナーを登録する。
    /// Error 理由だけが変わる場合は同じ `SessionState` で再通知される。
    /// 登録順に保存後に発火される。AgentStatusCenter のような中央管理が
    /// SessionStore からの状態変更を一方向に受け取るための入口。
    pub fn register_state_change_listener(&self, listener: SessionStateChangeListener) {
        self.state_change_listeners.write().push(listener);
    }

    pub fn register_event_log_recovery_listener(&self, listener: SessionEventLogRecoveryListener) {
        self.event_log_recovery_listeners.write().push(listener);
    }

    fn notify_state_change(
        &self,
        session_id: &str,
        worktree_path: &str,
        new_state: &SessionState,
        state_revision: u64,
    ) {
        let listeners = self.state_change_listeners.read().clone();
        for listener in listeners {
            listener(session_id, worktree_path, new_state, state_revision);
        }
    }

    #[cfg(test)]
    fn notify_event_log_recovered(&self, session_id: &str) {
        let listeners = self.event_log_recovery_listeners.read().clone();
        for listener in listeners {
            listener(session_id);
        }
    }

    fn require_meta(&self, app_data_dir: &Path, session_id: &str) -> Result<SessionMeta, String> {
        self.get_session_meta(app_data_dir, session_id)?
            .ok_or_else(|| format!("Session not found: {session_id}"))
    }

    fn update_meta_only(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        update: impl FnOnce(&mut SessionMeta) -> Result<(), String>,
    ) -> Result<(SessionMeta, bool), String> {
        self.update_meta_only_with_kind(
            app_data_dir,
            session_id,
            crate::domain::local_event::CommitOperationKind::Projection,
            update,
        )
    }

    fn update_meta_only_with_kind(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        operation_kind: crate::domain::local_event::CommitOperationKind,
        update: impl FnOnce(&mut SessionMeta) -> Result<(), String>,
    ) -> Result<(SessionMeta, bool), String> {
        self.ensure_canonical_mutation_admission()?;
        if self.event_authority.read().is_none() {
            #[cfg(test)]
            {
                let mut update = Some(update);
                let mut state_changed = false;
                let meta = self.test_storage().update_session_meta(
                    app_data_dir,
                    session_id,
                    &mut |meta| {
                        let previous_state = meta.state;
                        update.take().expect("legacy meta update runs once")(meta)?;
                        meta.state_revision =
                            next_sqlite_counter(meta.state_revision, "session state revision")?;
                        state_changed = previous_state != meta.state;
                        Ok(())
                    },
                )?;
                return Ok((meta, state_changed));
            }
            #[cfg(not(test))]
            unreachable!("production mutation admission rejects a missing SQLite authority");
        }
        let mut meta = self
            .get_session_meta(app_data_dir, session_id)?
            .ok_or_else(|| format!("Session not found: {session_id}"))?;
        let previous_state = meta.state;
        update(&mut meta)?;
        meta.state_revision = next_sqlite_counter(meta.state_revision, "session state revision")?;
        let state_changed = previous_state != meta.state;
        if operation_kind == crate::domain::local_event::CommitOperationKind::UserMutation {
            self.commit_user_meta_projection_snapshot(meta.clone())?;
        } else {
            self.commit_meta_projection_snapshot_with_kind(meta.clone(), operation_kind)?;
        }
        Ok((meta, state_changed))
    }

    fn update_meta_if_changed(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        update: impl FnOnce(&mut SessionMeta) -> Result<bool, String>,
    ) -> Result<Option<SessionMeta>, String> {
        self.ensure_canonical_mutation_admission()?;
        if self.event_authority.read().is_none() {
            #[cfg(test)]
            {
                let mut update = Some(update);
                let mut changed = false;
                let meta = self.test_storage().update_session_meta(
                    app_data_dir,
                    session_id,
                    &mut |meta| {
                        changed = update.take().expect("legacy meta update runs once")(meta)?;
                        Ok(())
                    },
                )?;
                return Ok(changed.then_some(meta));
            }
            #[cfg(not(test))]
            unreachable!("production mutation admission rejects a missing SQLite authority");
        }
        let mut meta = self
            .get_session_meta(app_data_dir, session_id)?
            .ok_or_else(|| format!("Session not found: {session_id}"))?;
        if !update(&mut meta)? {
            return Ok(None);
        }
        self.commit_meta_projection_snapshot(meta.clone())?;
        Ok(Some(meta))
    }

    #[cfg(test)]
    pub fn set_session_state(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        state: SessionState,
    ) -> Result<(), String> {
        self.set_session_state_with_kind(
            app_data_dir,
            session_id,
            state,
            crate::domain::local_event::CommitOperationKind::Projection,
        )
    }

    pub(crate) fn set_session_state_from_user(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        state: SessionState,
    ) -> Result<(), String> {
        self.set_session_state_with_kind(
            app_data_dir,
            session_id,
            state,
            crate::domain::local_event::CommitOperationKind::UserMutation,
        )
    }

    fn set_session_state_with_kind(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        state: SessionState,
        operation_kind: crate::domain::local_event::CommitOperationKind,
    ) -> Result<(), String> {
        #[cfg(test)]
        if let Some(hook) = self.set_state_hook.read().clone() {
            hook(session_id, &state)?;
        }
        let state_for_notify = state;
        let (meta, state_changed) =
            self.update_meta_only_with_kind(app_data_dir, session_id, operation_kind, |meta| {
                if !state.retains_error_reason() {
                    meta.error_reason = None;
                }
                meta.state = state;
                meta.updated_at = now_timestamp();
                Ok(())
            })?;
        if state_changed {
            self.notify_state_change(
                session_id,
                &meta.worktree_path,
                &state_for_notify,
                meta.state_revision,
            );
        }
        Ok(())
    }

    #[cfg(test)]
    fn set_event_projection(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        state: SessionState,
        error_reason: Option<String>,
        last_turn_interruption: Option<TurnInterruption>,
        last_turn_id: Option<u64>,
    ) -> Result<(), String> {
        #[cfg(test)]
        if let Some(hook) = self.projection_hook.read().clone() {
            hook(session_id, &state, error_reason.as_deref())?;
        }
        #[cfg(test)]
        if let Some(hook) = self.event_projection_hook.read().clone() {
            hook(session_id, last_turn_id)?;
        }
        let state_for_notify = state;
        let projected_error_reason = error_reason_for_state(&state, &error_reason);
        let mut previous_error_reason = None;
        let (meta, state_changed) = self.update_meta_only(app_data_dir, session_id, |meta| {
            previous_error_reason = Some(meta.error_reason.clone());
            meta.state = state;
            meta.error_reason = projected_error_reason.clone();
            meta.last_turn_interruption = last_turn_interruption;
            meta.last_turn_id = last_turn_id;
            meta.updated_at = now_timestamp();
            Ok(())
        })?;
        if state_changed
            || previous_error_reason
                .expect("update_session_meta must invoke closure before returning Ok")
                != projected_error_reason
        {
            self.notify_state_change(
                session_id,
                &meta.worktree_path,
                &state_for_notify,
                meta.state_revision,
            );
        }
        Ok(())
    }

    #[cfg(test)]
    pub fn update_permission_mode(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        permission_mode: &str,
    ) -> Result<(), String> {
        self.update_permission_mode_with_kind(
            app_data_dir,
            session_id,
            permission_mode,
            crate::domain::local_event::CommitOperationKind::Projection,
        )
    }

    pub(crate) fn update_permission_mode_from_user(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        permission_mode: &str,
    ) -> Result<(), String> {
        self.update_permission_mode_with_kind(
            app_data_dir,
            session_id,
            permission_mode,
            crate::domain::local_event::CommitOperationKind::UserMutation,
        )
    }

    fn update_permission_mode_with_kind(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        permission_mode: &str,
        operation_kind: crate::domain::local_event::CommitOperationKind,
    ) -> Result<(), String> {
        let permission_mode = crate::domain::agent_session::PermissionMode::parse(permission_mode)
            .map_err(|e| e.to_string())?;
        self.update_meta_only_with_kind(app_data_dir, session_id, operation_kind, |meta| {
            meta.permission_mode = permission_mode.as_str().to_string();
            Ok(())
        })?;
        Ok(())
    }

    #[cfg(test)]
    pub fn update_plan_mode(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        plan_mode: bool,
    ) -> Result<(), String> {
        self.update_plan_mode_with_kind(
            app_data_dir,
            session_id,
            plan_mode,
            crate::domain::local_event::CommitOperationKind::Projection,
        )
    }

    pub(crate) fn update_plan_mode_from_user(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        plan_mode: bool,
    ) -> Result<(), String> {
        self.update_plan_mode_with_kind(
            app_data_dir,
            session_id,
            plan_mode,
            crate::domain::local_event::CommitOperationKind::UserMutation,
        )
    }

    fn update_plan_mode_with_kind(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        plan_mode: bool,
        operation_kind: crate::domain::local_event::CommitOperationKind,
    ) -> Result<(), String> {
        self.update_meta_only_with_kind(app_data_dir, session_id, operation_kind, |meta| {
            meta.plan_mode = plan_mode;
            Ok(())
        })?;
        Ok(())
    }

    #[cfg(test)]
    pub fn update_backend_selection(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        backend_id: String,
        selected_model: Option<String>,
    ) -> Result<(), String> {
        self.update_backend_selection_with_kind(
            app_data_dir,
            session_id,
            backend_id,
            selected_model,
            crate::domain::local_event::CommitOperationKind::Projection,
        )
    }

    pub(crate) fn update_backend_selection_from_user(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        backend_id: String,
        selected_model: Option<String>,
    ) -> Result<(), String> {
        self.update_backend_selection_with_kind(
            app_data_dir,
            session_id,
            backend_id,
            selected_model,
            crate::domain::local_event::CommitOperationKind::UserMutation,
        )
    }

    fn update_backend_selection_with_kind(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        backend_id: String,
        selected_model: Option<String>,
        operation_kind: crate::domain::local_event::CommitOperationKind,
    ) -> Result<(), String> {
        self.update_meta_only_with_kind(app_data_dir, session_id, operation_kind, |meta| {
            meta.backend_id = backend_id;
            meta.selected_model = selected_model;
            meta.updated_at = now_timestamp();
            Ok(())
        })?;
        Ok(())
    }

    #[allow(dead_code)] // issues-1301 G-1: retained for permission profile settings surface; current runtime only reads the stored profile id.
    pub fn update_permission_profile_id(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        permission_profile_id: Option<&str>,
    ) -> Result<(), String> {
        let profile_id =
            crate::domain::agent_session::services::normalize_permission_profile_id(
                permission_profile_id,
            )
            .map_err(|_| {
                "Permission profile id cannot contain control characters".to_string()
            })?;
        self.update_meta_only(app_data_dir, session_id, |meta| {
            meta.permission_profile_id = profile_id;
            Ok(())
        })?;
        Ok(())
    }

    #[cfg(test)]
    pub fn update_agent_session_id(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        agent_session_id: Option<String>,
    ) -> Result<(), String> {
        self.update_meta_only(app_data_dir, session_id, |meta| {
            meta.agent_session_id = agent_session_id;
            meta.updated_at = now_timestamp();
            Ok(())
        })?;
        Ok(())
    }

    #[cfg(test)]
    pub fn update_agent_session_id_if_changed(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        agent_session_id: Option<String>,
    ) -> Result<Option<SessionMeta>, String> {
        self.update_meta_if_changed(app_data_dir, session_id, |meta| {
            if meta.agent_session_id == agent_session_id {
                return Ok(false);
            }
            meta.agent_session_id = agent_session_id;
            meta.updated_at = now_timestamp();
            Ok(true)
        })
    }

    #[cfg(test)]
    pub fn update_context_carry_if_changed(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        context_carry: Option<ContextCarryState>,
    ) -> Result<Option<SessionMeta>, String> {
        self.update_meta_if_changed(app_data_dir, session_id, |meta| {
            if meta.context_carry == context_carry {
                return Ok(false);
            }
            meta.context_carry = context_carry;
            meta.updated_at = now_timestamp();
            Ok(true)
        })
    }

    pub fn complete_context_reinjection_if_required(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        expected_provider_session_generation: u64,
        reinjected: bool,
    ) -> Result<Option<SessionMeta>, String> {
        self.complete_context_restore_after_start_if_current(
            app_data_dir,
            session_id,
            ContextRestoreCompletionRequest {
                expected_provider_session_generation,
                expected_turn_id: None,
                reinjected,
                clear_context_carry: false,
                recovery_restore_required: true,
            },
        )
    }

    pub fn complete_context_restore_after_start_if_current(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        request: ContextRestoreCompletionRequest,
    ) -> Result<Option<SessionMeta>, String> {
        if !request.requests_change() {
            return Ok(None);
        }
        let ContextRestoreCompletionRequest {
            expected_provider_session_generation,
            expected_turn_id,
            reinjected,
            clear_context_carry,
            recovery_restore_required,
        } = request;
        let at = now_timestamp();
        if !self.canonical_authority_active() {
            #[cfg(test)]
            {
                return match self.update_meta_if_changed(app_data_dir, session_id, |meta| {
                    apply_context_restore_completion_to_meta(
                        None,
                        meta,
                        ContextRestoreCompletionCommand {
                            expected_provider_session_generation,
                            expected_turn_id,
                            reinjected,
                            clear_context_carry,
                            recovery_restore_required,
                        },
                        at,
                    )
                }) {
                    Err(error)
                        if error == CONTEXT_RESTORE_COMPLETION_FENCED
                            || error == CONTEXT_RESTORE_COMPLETION_UNCHANGED =>
                    {
                        Ok(None)
                    }
                    result => result,
                };
            }
            #[cfg(not(test))]
            unreachable!("production mutation admission rejects a missing SQLite authority");
        }
        let patch = EventProjectionMetaPatch::ContextRestoreCompleted {
            expected_provider_session_generation,
            expected_turn_id,
            reinjected,
            clear_context_carry,
            recovery_restore_required,
            at,
        };
        match self.commit_agent_events_with_additional_mutations(
            session_id,
            &[],
            Vec::new(),
            None,
            Some(patch),
            None,
            crate::domain::local_event::CommitOperationKind::Projection,
        ) {
            Ok(()) => {}
            Err(error)
                if error == CONTEXT_RESTORE_COMPLETION_FENCED
                    || error == CONTEXT_RESTORE_COMPLETION_UNCHANGED =>
            {
                return Ok(None);
            }
            Err(error) => return Err(error),
        }
        let meta = self
            .get_session_meta(app_data_dir, session_id)?
            .ok_or_else(|| format!("Session not found: {session_id}"))?;
        let authority = self.event_authority.read().clone();
        let facts = match authority.as_ref() {
            Some(authority) => authority.projection_codec.context_restore_completion_facts(&meta),
            #[cfg(test)]
            None => test_context_restore_completion_facts(&meta),
            #[cfg(not(test))]
            None => unreachable!("production mutation admission requires a projection codec"),
        };
        let settled = context_restore_completion_is_settled(
            facts,
            ContextRestoreCompletionCommand {
                expected_provider_session_generation,
                expected_turn_id,
                reinjected,
                clear_context_carry,
                recovery_restore_required,
            },
        );
        Ok(settled.then_some(meta))
    }

    pub fn update_system_context_private_meta_if_changed(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        context_epoch: Option<ContextEpochMeta>,
        workflow_instructions: Vec<String>,
        agent_read_paths: Option<Vec<PathBuf>>,
    ) -> Result<Option<SessionMeta>, String> {
        self.update_meta_if_changed(app_data_dir, session_id, |meta| {
            if meta.context_epoch == context_epoch
                && meta.workflow_instructions == workflow_instructions
                && (agent_read_paths.is_none() || meta.agent_read_paths == agent_read_paths)
            {
                return Ok(false);
            }
            meta.context_epoch = context_epoch;
            meta.workflow_instructions = workflow_instructions;
            if agent_read_paths.is_some() {
                meta.agent_read_paths = agent_read_paths.clone();
            }
            meta.updated_at = now_timestamp();
            Ok(true)
        })
    }

    #[cfg(test)]
    pub fn update_resume_metadata_if_changed(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        agent_session_id: Option<String>,
        context_carry: Option<ContextCarryState>,
    ) -> Result<Option<SessionMeta>, String> {
        self.update_meta_if_changed(app_data_dir, session_id, |meta| {
            if meta.agent_session_id == agent_session_id && meta.context_carry == context_carry {
                return Ok(false);
            }
            meta.agent_session_id = agent_session_id;
            meta.context_carry = context_carry;
            meta.updated_at = now_timestamp();
            Ok(true)
        })
    }

    pub fn get_session_page(
        &self,
        _app_data_dir: &Path,
        session_id: &str,
        cursor: Option<PageCursor>,
        limit: usize,
    ) -> Result<Option<SessionPage>, String> {
        if self.canonical_authority_active() {
            return self
                .canonical_message_page(session_id, cursor, limit)
                .map(Some);
        }
        #[cfg(test)]
        return self
            .test_storage()
            .get_session_page(_app_data_dir, session_id, cursor, limit);
        #[cfg(not(test))]
        unreachable!("production always has a SQLite event authority")
    }

    #[cfg(test)]
    pub fn append_message(
        &self,
        _app_data_dir: &Path,
        session_id: &str,
        message: &ChatMessage,
    ) -> Result<SessionMeta, String> {
        self.ensure_canonical_mutation_admission()?;
        #[cfg(test)]
        if let Some(hook) = self.append_message_hook.read().clone() {
            hook(session_id, message)?;
        }
        if !self.canonical_authority_active() {
            #[cfg(test)]
            return self
                .test_storage()
                .append_message(_app_data_dir, session_id, message);
            #[cfg(not(test))]
            return Err("agent-session SQLite event authority is not configured".to_string());
        }
        let current = self
            .read_session_projection(session_id)?
            .ok_or_else(|| format!("Session projection not found: {session_id}"))?;
        let mut meta = current.meta;
        meta.message_count = add_sqlite_count(meta.message_count, 1, "session message count")?;
        if meta.first_message_preview.is_empty() {
            meta.first_message_preview =
                super::first_message_preview(std::slice::from_ref(message));
        }
        meta.updated_at = meta.updated_at.max(message.timestamp);
        meta.state_revision = next_sqlite_counter(meta.state_revision, "session state revision")?;
        self.commit_session_projection_snapshot(CanonicalAgentSessionProjection {
            meta: meta.clone(),
            title: current.title,
            messages: vec![message.clone()],
            reducer_events: current.reducer_events,
            queue_paused_at: current.queue_paused_at,
            latest_token_usage: None,
            pending_send_queue: current.pending_send_queue,
        })?;
        Ok(meta)
    }

    pub fn get_session_attachment(
        &self,
        _app_data_dir: &Path,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<Option<SessionAttachment>, String> {
        if self.canonical_authority_active() {
            if attachment_id.len() != 64
                || !attachment_id.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return Ok(None);
            }
            return self
                .canonical_content_blob(session_id, format!("attachment:{attachment_id}"))
                .and_then(|record| {
                    record
                        .map(|record| {
                            let crate::domain::local_event::AgentContentBlobRecord::Attachment {
                                id,
                                media_type,
                                bytes,
                            } = record
                            else {
                                return Err(
                                    "SQLite attachment identity is incompatible".to_string()
                                );
                            };
                            if id != attachment_id {
                                return Err(
                                    "SQLite attachment identity is incompatible".to_string()
                                );
                            }
                            Ok(SessionAttachment {
                                data: base64::engine::general_purpose::STANDARD.encode(bytes),
                                media_type,
                            })
                        })
                        .transpose()
                });
        }
        #[cfg(test)]
        return self.test_storage().get_session_attachment(
            _app_data_dir,
            session_id,
            attachment_id,
        );
        #[cfg(not(test))]
        unreachable!("production always has a SQLite event authority")
    }

    pub fn get_session_tool_output(
        &self,
        _app_data_dir: &Path,
        session_id: &str,
        tool_output_id: &str,
    ) -> Result<Option<SessionToolOutput>, String> {
        if self.canonical_authority_active() {
            return self
                .canonical_content_blob(session_id, format!("tool_output:{tool_output_id}"))
                .and_then(|record| {
                    record
                        .map(|record| {
                            let crate::domain::local_event::AgentContentBlobRecord::ToolOutput {
                                id,
                                content,
                            } = record
                            else {
                                return Err(
                                    "SQLite tool output identity is incompatible".to_string()
                                );
                            };
                            if id != tool_output_id {
                                return Err(
                                    "SQLite tool output identity is incompatible".to_string()
                                );
                            }
                            Ok(SessionToolOutput {
                                byte_size: content.len() as u64,
                                content,
                            })
                        })
                        .transpose()
                });
        }
        #[cfg(test)]
        return self.test_storage().get_session_tool_output(
            _app_data_dir,
            session_id,
            tool_output_id,
        );
        #[cfg(not(test))]
        unreachable!("production always has a SQLite event authority")
    }

    /// Atomically records streaming-domain events and the exact public message snapshot.
    ///
    /// Runtime streaming must use this boundary before publishing a delta. Otherwise an event
    /// can become durable while its message projection fails (or the inverse), leaving live and
    /// reload views with different prefixes.
    pub(crate) fn persist_streaming_parts_with_events(
        &self,
        _app_data_dir: &Path,
        session_id: &str,
        events: &[AgentSessionEvent],
        message_id: &str,
        parts: &[MessagePart],
        streaming_final_seq: u64,
    ) -> Result<Vec<MessagePart>, String> {
        #[cfg(test)]
        if let Some(hook) = self.append_event_hook.read().clone() {
            for event in events {
                hook(session_id, event)?;
            }
        }
        #[cfg(test)]
        if let Some(hook) = self.persist_parts_hook.read().clone() {
            hook(session_id, message_id, parts)?;
        }
        if !self.canonical_authority_active() {
            #[cfg(test)]
            {
                let supplied_parts = parts.to_vec();
                let (_, (), persisted_parts) = self.commit_projection_and_notify(
                    _app_data_dir,
                    session_id,
                    events,
                    |projected, projected_meta| {
                        let completed_at = projected
                            .message_for_id(message_id)
                            .filter(|message| message.role == MessageRole::Agent)
                            .ok_or_else(|| {
                                format!(
                                    "Streaming projection omitted message {message_id} for {session_id}"
                                )
                            })?
                            .timestamp;
                        Ok((
                            AgentSessionProjectionCommit {
                                meta: projected_meta,
                                message: AgentSessionProjectedMessage::PersistParts {
                                    message_id: message_id.to_string(),
                                    parts: supplied_parts.clone(),
                                    streaming_final_seq,
                                    completed_at,
                                },
                            },
                            (),
                        ))
                    },
                )?;
                return Ok(persisted_parts);
            }
            #[cfg(not(test))]
            return Err("agent-session SQLite event authority is not configured".to_string());
        }
        self.commit_agent_events_with_additional_mutations(
            session_id,
            events,
            Vec::new(),
            Some(TerminalMessageProjectionPatch {
                message_id: message_id.to_string(),
                streaming_final_seq,
                timestamp: None,
                parts: Some(parts.to_vec()),
            }),
            None,
            None,
            crate::domain::local_event::CommitOperationKind::Projection,
        )?;
        self.canonical_message_projection(session_id, message_id)?
            .and_then(|message| message.parts)
            .ok_or_else(|| {
                format!("Streaming message projection not found: {session_id}/{message_id}")
            })
    }

    pub fn persist_message_parts(
        &self,
        _app_data_dir: &Path,
        session_id: &str,
        message_id: &str,
        parts: &[MessagePart],
        streaming_final_seq: u64,
        completed_at: Option<f64>,
    ) -> Result<Vec<MessagePart>, String> {
        #[cfg(test)]
        if let Some(hook) = self.persist_parts_hook.read().clone() {
            hook(session_id, message_id, parts)?;
        }
        if !self.canonical_authority_active() {
            #[cfg(test)]
            return self.test_storage().persist_message_parts(
                _app_data_dir,
                session_id,
                message_id,
                parts,
                streaming_final_seq,
                completed_at,
            );
            #[cfg(not(test))]
            return Err("agent-session SQLite event authority is not configured".to_string());
        }
        let current = self
            .read_session_projection(session_id)?
            .ok_or_else(|| format!("Session projection not found: {session_id}"))?;
        let mut message = self
            .canonical_message_projection(session_id, message_id)?
            .ok_or_else(|| {
                format!("Message not found after projection: {session_id}/{message_id}")
            })?;
        message.parts = Some(parts.to_vec());
        message.streaming_final_seq = streaming_final_seq;
        if let Some(completed_at) = completed_at {
            message.timestamp = completed_at;
        }
        self.commit_session_projection_snapshot(CanonicalAgentSessionProjection {
            meta: current.meta,
            title: current.title,
            messages: vec![message],
            reducer_events: current.reducer_events,
            queue_paused_at: current.queue_paused_at,
            latest_token_usage: current.latest_token_usage,
            pending_send_queue: current.pending_send_queue,
        })?;
        Ok(parts.to_vec())
    }
}
