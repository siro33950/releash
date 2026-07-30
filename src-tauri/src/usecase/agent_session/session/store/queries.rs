impl SessionStore {
    #[cfg(test)]
    pub fn list_sessions(
        &self,
        app_data_dir: &Path,
        worktree_path: &str,
    ) -> Result<Vec<SessionSummary>, String> {
        self.list_sessions_filtered(app_data_dir, worktree_path, |s| s.state.is_open())
    }

    #[cfg(test)]
    pub fn list_closed_sessions(
        &self,
        app_data_dir: &Path,
        worktree_path: &str,
    ) -> Result<Vec<SessionSummary>, String> {
        self.list_sessions_filtered(app_data_dir, worktree_path, |s| {
            s.state.is_closed_history()
        })
    }

    /// Fixed application-shutdown inventory. Workflow-owned child sessions
    /// are represented by their workflow owner target and therefore are not
    /// emitted as a second shutdown target here.
    pub(crate) fn application_shutdown_target_session_ids(
        &self,
        _app_data_dir: &Path,
    ) -> Result<Vec<String>, String> {
        if !self.canonical_authority_active() {
            #[cfg(test)]
            {
                let mut ids = self
                    .read_session_metadata_inventory(_app_data_dir)?
                    .into_iter()
                    .filter(|meta| {
                        !meta.workflow_node_session && meta.state.is_open()
                    })
                    .map(|meta| meta.id)
                    .collect::<Vec<_>>();
                ids.sort();
                ids.dedup();
                return Ok(ids);
            }
            #[cfg(not(test))]
            unreachable!("production always has a SQLite event authority");
        }
        let authority = self
            .event_authority
            .read()
            .clone()
            .expect("canonical authority checked");
        let result = authority
            .repository
            .query_blocking(
                crate::domain::local_event::LocalEventQuery::CanonicalRuntimeOwnerSnapshot {
                    limit: 8_192,
                },
            )
            .map_err(|error| format!("runtime owner inventory read failed: {error}"))?;
        let crate::domain::local_event::LocalEventQueryResult::CanonicalRuntimeOwnerSnapshot(
            owners,
        ) = result
        else {
            return Err("runtime owner inventory returned the wrong shape".to_string());
        };
        let mut ids = owners
            .into_iter()
            .filter_map(|owner| match owner {
                crate::domain::local_event::CanonicalRuntimeOwnerView::AgentSession {
                    session_id,
                    shutdown_target: true,
                    workflow_node_session: false,
                    ..
                } => Some(session_id),
                _ => None,
            })
            .collect::<Vec<_>>();
        ids.sort();
        ids.dedup();
        Ok(ids)
    }

    #[cfg(test)]
    fn list_sessions_filtered(
        &self,
        app_data_dir: &Path,
        worktree_path: &str,
        predicate: impl Fn(&SessionMeta) -> bool,
    ) -> Result<Vec<SessionSummary>, String> {
        let mut summaries = self
            .read_session_metadata_inventory(app_data_dir)?
            .into_iter()
            .filter(|s| same_worktree_path(&s.worktree_path, worktree_path) && predicate(s))
            .map(|meta| meta.to_summary())
            .collect::<Vec<_>>();
        summaries.sort_by(|a, b| {
            b.updated_at
                .partial_cmp(&a.updated_at)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        for summary in &mut summaries {
            if let Some(title) = self.session_title(app_data_dir, &summary.id)? {
                summary.first_message = title;
            }
        }
        Ok(summaries)
    }

    #[cfg(test)]
    pub fn archive_session(&self, app_data_dir: &Path, session_id: &str) -> Result<(), String> {
        let meta = self.require_meta(app_data_dir, session_id)?;
        if !meta.state.is_closed_history() {
            return Err("Only closed sessions can be archived".to_string());
        }
        self.set_session_state(app_data_dir, session_id, SessionState::Archived)
    }

    #[cfg(test)]
    pub fn archive_open_session(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<(), String> {
        let meta = self.require_meta(app_data_dir, session_id)?;
        if crate::domain::agent_session::services::admit_user_session_metadata_action(
            meta.workflow_node_session,
            UserSessionMetadataAction::ArchiveOpen,
        )
        .is_err()
        {
            return Err("Workflow node sessions cannot be archived".to_string());
        }
        self.set_session_state(app_data_dir, session_id, SessionState::Archived)
    }

    pub fn session_title(
        &self,
        _app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<String>, String> {
        if let Some(projection) = self.read_session_projection(session_id)? {
            return Ok(projection.title);
        }
        if self.canonical_authority_active() {
            return Ok(None);
        }
        #[cfg(test)]
        return self.test_storage().session_title(_app_data_dir, session_id);
        #[cfg(not(test))]
        unreachable!("production always has a SQLite event authority")
    }

    pub fn session_titles(&self, app_data_dir: &Path) -> Result<HashMap<String, String>, String> {
        if self.canonical_authority_active() {
            let mut titles = HashMap::new();
            for meta in self.read_canonical_metadata_inventory(app_data_dir)? {
                if let Some(title) = self.session_title(app_data_dir, &meta.id)? {
                    titles.insert(meta.id, title);
                }
            }
            return Ok(titles);
        }
        #[cfg(test)]
        return self.test_storage().session_titles(app_data_dir);
        #[cfg(not(test))]
        unreachable!("production always has a SQLite event authority")
    }

    pub fn set_session_title(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        title: Option<&str>,
    ) -> Result<SessionSummary, String> {
        let meta = self.require_meta(app_data_dir, session_id)?;
        if crate::domain::agent_session::services::admit_user_session_metadata_action(
            meta.workflow_node_session,
            UserSessionMetadataAction::Rename,
        )
        .is_err()
        {
            return Err("Workflow node sessions cannot be renamed".to_string());
        }

        let title_for_summary = title
            .map(crate::domain::agent_session::services::compact_session_title)
            .filter(|title| !title.is_empty());
        if !self.canonical_authority_active() {
            #[cfg(test)]
            {
                self.test_storage().write_session_title(
                    app_data_dir,
                    session_id,
                    title_for_summary.as_deref(),
                )?;
                let mut summary = meta.to_summary();
                if let Some(title) = title_for_summary {
                    summary.first_message = title;
                }
                return Ok(summary);
            }
            #[cfg(not(test))]
            return Err("agent-session SQLite event authority is not configured".to_string());
        }
        let current = self
            .read_session_projection(session_id)?
            .ok_or_else(|| format!("Session projection not found: {session_id}"))?;
        self.commit_user_session_projection_snapshot(CanonicalAgentSessionProjection {
            meta: current.meta.clone(),
            title: title_for_summary.clone(),
            messages: Vec::new(),
            reducer_events: current.reducer_events,
            queue_paused_at: current.queue_paused_at,
            latest_token_usage: current.latest_token_usage,
            pending_send_queue: current.pending_send_queue,
        })?;

        let mut summary = meta.to_summary();
        if let Some(title) = title_for_summary {
            summary.first_message = title;
        }
        Ok(summary)
    }

    pub fn fork_session(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<ChatSession, String> {
        self.ensure_canonical_mutation_admission()?;
        let parent_meta = self.require_meta(app_data_dir, session_id)?;
        let fork = decide_session_fork(
            parent_meta.workflow_node_session,
        )
        .map_err(|_| "Workflow node sessions cannot be forked".to_string())?;

        let now = now_timestamp();
        let mut forked_meta = parent_meta.clone();
        forked_meta.id = uuid::Uuid::new_v4().to_string();
        forked_meta.state = fork.state;
        forked_meta.error_reason = fork.error_reason;
        forked_meta.created_at = now;
        forked_meta.updated_at = now;
        forked_meta.agent_session_id = fork.provider_session_id;
        forked_meta.provider_session_generation = fork.provider_session_generation;
        forked_meta.provider_session_observation_id = fork.provider_session_observation_id;
        forked_meta.context_reinjection_generation = fork.context_reinjection_generation;
        forked_meta.context_carry = fork.context_carry;
        if fork.clear_recovery_publication {
            forked_meta.recovery_publication_snapshot = None;
        }
        if fork.clear_last_turn_interruption {
            forked_meta.last_turn_interruption = None;
        }
        forked_meta.last_turn_id = fork.last_turn_id;
        forked_meta.workflow_node_session = fork.workflow_node_session;

        if !self.canonical_authority_active() {
            #[cfg(test)]
            {
                let title = self
                    .test_storage()
                    .session_title(app_data_dir, session_id)?;
                self.test_storage()
                    .fork_session_layout(app_data_dir, session_id, &forked_meta)?;
                if let Some(title) = title.as_deref() {
                    self.test_storage().write_session_title(
                        app_data_dir,
                        &forked_meta.id,
                        Some(title),
                    )?;
                }
                return Ok(forked_meta.to_session(Vec::new()));
            }
            #[cfg(not(test))]
            return Err("agent-session SQLite event authority is not configured".to_string());
        }

        let title = self.session_title(app_data_dir, session_id)?;
        let messages = self.canonical_all_messages(session_id)?;
        self.commit_user_session_projection_snapshot(CanonicalAgentSessionProjection {
            meta: forked_meta.clone(),
            title,
            messages,
            reducer_events: Vec::new(),
            queue_paused_at: None,
            latest_token_usage: None,
            pending_send_queue: Vec::new(),
        })?;
        Ok(forked_meta.to_session(Vec::new()))
    }

    pub fn get_session_shell(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<ChatSession>, String> {
        Ok(self
            .get_session_meta(app_data_dir, session_id)?
            .map(|meta| meta.to_session(Vec::new())))
    }

    pub fn get_session_with_latest_page(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        limit: usize,
    ) -> Result<Option<(ChatSession, SessionPage, Option<TurnInterruption>)>, String> {
        let Some(meta) = self.get_session_meta(app_data_dir, session_id)? else {
            return Ok(None);
        };
        let page = self
            .get_session_page(app_data_dir, session_id, None, limit)?
            .unwrap_or(SessionPage {
                messages: Vec::new(),
                message_metadata: Vec::new(),
                next_cursor: None,
                has_more: false,
                total_count: meta.message_count,
                latest_token_usage: None,
            });
        let session = meta.to_session(page.messages.clone());
        Ok(Some((session, page, meta.last_turn_interruption)))
    }

    pub fn get_session_meta(
        &self,
        _app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<SessionMeta>, String> {
        if let Some(projection) = self.read_session_projection(session_id)? {
            return Ok(Some(projection.meta));
        }
        if self.canonical_authority_active() {
            return Ok(None);
        }
        #[cfg(test)]
        return self
            .test_storage()
            .get_session_meta(_app_data_dir, session_id);
        #[cfg(not(test))]
        unreachable!("production always has a SQLite event authority")
    }

    pub fn get_session_review_context(
        &self,
        _app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<SessionReviewContext>, String> {
        if let Some(projection) = self.read_session_projection(session_id)? {
            return Ok(Some(projection.meta.into()));
        }
        if self.canonical_authority_active() {
            return Ok(None);
        }
        #[cfg(test)]
        return self
            .test_storage()
            .get_session_review_context(_app_data_dir, session_id);
        #[cfg(not(test))]
        unreachable!("production always has a SQLite event authority")
    }

    pub fn load_full_session_for_restore(
        &self,
        _app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<ChatSession>, String> {
        if let Some(projection) = self.read_session_projection(session_id)? {
            let messages = self.canonical_all_messages(session_id)?;
            return Ok(Some(projection.meta.to_session(messages)));
        }
        if self.canonical_authority_active() {
            return Ok(None);
        }
        #[cfg(test)]
        return self
            .test_storage()
            .load_full_session_for_restore(_app_data_dir, session_id);
        #[cfg(not(test))]
        unreachable!("production always has a SQLite event authority")
    }

    pub fn load_session_events(
        &self,
        _app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Vec<AgentSessionEvent>, String> {
        if let Some(projection) = self.read_session_projection(session_id)? {
            return Ok(projection.reducer_events);
        }
        if self.canonical_authority_active() {
            return Ok(Vec::new());
        }
        #[cfg(test)]
        return self
            .test_storage()
            .load_session_events(_app_data_dir, session_id);
        #[cfg(not(test))]
        unreachable!("production always has a SQLite event authority")
    }

    /// Read only the bounded reducer input owned by the current session
    /// projection. Terminal arbitration and operation CAS paths must use this
    /// instead of replaying historical turns.
    pub(crate) fn load_current_reducer_events(
        &self,
        _app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Vec<AgentSessionEvent>, String> {
        if let Some(projection) = self.read_session_projection(session_id)? {
            return Ok(projection.reducer_events);
        }
        if self.canonical_authority_active() {
            return Ok(Vec::new());
        }
        #[cfg(test)]
        {
            let events = self
                .test_storage()
                .load_session_events(_app_data_dir, session_id)?;
            Ok(bounded_reducer_events(Vec::new(), &events))
        }
        #[cfg(not(test))]
        unreachable!("production always has a SQLite event authority")
    }

    pub fn load_queue_paused_at(
        &self,
        _app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<f64>, String> {
        if let Some(projection) = self.read_session_projection(session_id)? {
            return Ok(projection.queue_paused_at);
        }
        if self.canonical_authority_active() {
            return Ok(None);
        }
        #[cfg(test)]
        return self
            .test_storage()
            .load_queue_paused_at(_app_data_dir, session_id);
        #[cfg(not(test))]
        unreachable!("production always has a SQLite event authority")
    }

}
