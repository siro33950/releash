use std::collections::HashMap;
use std::io::BufReader;
use std::path::Path;

use super::layout::{
    attachments_dir_in_dir, content_hash, index_file_in_dir, legacy_meta_file, message_file_in_dir,
    messages_dir_in_dir, meta_file_in_dir, session_dir, session_file, sessions_dir,
    write_json_pretty_atomic,
};
use super::FileSessionStorage;
use crate::usecase::agent_session::session::{
    first_message_preview, now_timestamp, parts_to_legacy, ChatMessage, ChatSession,
    MessageIndexEntry, MessagePageMetadata, MessagePart, PageCursor, SessionMeta, SessionPage,
    MAX_SESSION_PAGE_LIMIT, SESSION_BODY_FORMAT_VERSION,
};

impl FileSessionStorage {
    pub fn load_full_session_for_restore(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<ChatSession>, String> {
        self.ensure_loaded(app_data_dir)?;
        if let Some(err) = self.invalid_sessions.read().get(session_id) {
            return Err(err.clone());
        }
        if !self.cache.read().contains_key(session_id) {
            return Ok(None);
        }
        self.ensure_session_layout(app_data_dir, session_id)?;
        self.load_full_session_from_layout(app_data_dir, session_id)
            .map(Some)
    }

    pub fn save_full_session_for_migration_or_restore(
        &self,
        app_data_dir: &Path,
        session: &ChatSession,
    ) -> Result<(), String> {
        // 保存層を AgentChat permission_mode の正典とする。
        // cache / ファイルへの書き込み前に検証・正規化し、legacy 入力を新規保存値へ漏らさない。
        let permission_mode = crate::permission::PermissionMode::parse(&session.permission_mode)
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
        self.persist_and_update_cache(app_data_dir, session)?;
        Ok(())
    }

    pub(super) fn persist_and_update_cache(
        &self,
        app_data_dir: &Path,
        session: &ChatSession,
    ) -> Result<bool, String> {
        let _lock = self.file_lock.lock();
        let dir = sessions_dir(app_data_dir);
        std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create sessions dir: {e}"))?;
        let session_dir = session_dir(app_data_dir, &session.id)?;
        self.write_split_session_to_dir(&session_dir, session, true)?;
        if let Ok(file) = session_file(app_data_dir, &session.id) {
            let _ = std::fs::remove_file(file);
        }
        if let Ok(file) = legacy_meta_file(app_data_dir, &session.id) {
            let _ = std::fs::remove_file(file);
        }
        let meta = SessionMeta::from_session(session);
        let mut cache = self.cache.write();
        let state_changed = cache.get(&session.id).map(|p| &p.state) != Some(&session.state);
        cache.insert(session.id.clone(), meta);
        self.invalid_sessions.write().remove(&session.id);
        Ok(state_changed)
    }

    pub fn get_session_page(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        cursor: Option<PageCursor>,
        limit: usize,
    ) -> Result<Option<SessionPage>, String> {
        self.ensure_loaded(app_data_dir)?;
        if let Some(err) = self.invalid_sessions.read().get(session_id) {
            return Err(err.clone());
        }
        if !self.cache.read().contains_key(session_id) {
            return Ok(None);
        }
        self.ensure_session_layout(app_data_dir, session_id)?;
        let dir = session_dir(app_data_dir, session_id)?;
        let limit = limit.clamp(1, MAX_SESSION_PAGE_LIMIT);
        let mut index = self.read_consistent_index_from_dir(&dir, session_id)?;
        let (mut page, needs_repair) =
            self.read_page_from_index(&dir, session_id, &index, cursor.clone(), limit)?;
        if needs_repair {
            index = self.repair_index_and_meta_from_messages(&dir, session_id)?;
            (page, _) = self.read_page_from_index(&dir, session_id, &index, cursor, limit)?;
        }
        Ok(Some(page))
    }

    pub fn append_message(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        message: &ChatMessage,
    ) -> Result<(), String> {
        self.ensure_loaded(app_data_dir)?;
        if let Some(err) = self.invalid_sessions.read().get(session_id) {
            return Err(err.clone());
        }
        if !self.cache.read().contains_key(session_id) {
            return Err(format!("Session not found: {session_id}"));
        }
        self.ensure_session_layout(app_data_dir, session_id)?;
        let _lock = self.file_lock.lock();
        let dir = session_dir(app_data_dir, session_id)?;
        let mut index = self.read_consistent_index_from_dir(&dir, session_id)?;
        let seq = index
            .iter()
            .map(|entry| entry.seq)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        let (stored_message, attachment_refs) =
            self.externalize_message_attachments(&dir, message)?;
        let hash = content_hash(&stored_message)?;
        write_json_pretty_atomic(
            &message_file_in_dir(&dir, seq),
            &stored_message,
            "message chunk",
        )?;

        let mut meta = self.read_meta_from_dir(&dir, session_id)?;
        let was_empty = index.is_empty();
        index.push(MessageIndexEntry {
            id: stored_message.id.clone(),
            seq,
            role: stored_message.role.clone(),
            timestamp: stored_message.timestamp,
            content_hash: hash,
            attachment_refs,
            token_meta: None,
        });
        meta.message_count = index.len();
        meta.updated_at = message.timestamp;
        if was_empty {
            meta.first_message_preview =
                first_message_preview(std::slice::from_ref(&stored_message));
        }

        write_json_pretty_atomic(&index_file_in_dir(&dir), &index, "session index")?;
        write_json_pretty_atomic(&meta_file_in_dir(&dir), &meta, "session meta")?;
        self.cache.write().insert(session_id.to_string(), meta);
        self.invalid_sessions.write().remove(session_id);
        Ok(())
    }
    pub fn persist_message_parts(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        message_id: &str,
        parts: &[MessagePart],
        completed_at: Option<f64>,
    ) -> Result<(), String> {
        self.ensure_loaded(app_data_dir)?;
        if let Some(err) = self.invalid_sessions.read().get(session_id) {
            return Err(err.clone());
        }
        if !self.cache.read().contains_key(session_id) {
            return Err(format!("Session not found: {session_id}"));
        }
        self.ensure_session_layout(app_data_dir, session_id)?;
        let _lock = self.file_lock.lock();
        let dir = session_dir(app_data_dir, session_id)?;
        let mut index = self.read_consistent_index_from_dir(&dir, session_id)?;
        let Some(entry) = index.iter_mut().find(|entry| entry.id == message_id) else {
            return Err(format!("Message not found: {session_id}/{message_id}"));
        };
        let path = message_file_in_dir(&dir, entry.seq);
        let mut message = self.read_message_file(&path)?;
        let (content, thinking, activities) = parts_to_legacy(parts);
        message.content = content;
        message.thinking = thinking;
        message.activities = activities;
        message.parts = Some(parts.to_vec());
        let updated_at = completed_at.unwrap_or_else(now_timestamp);
        if let Some(completed_at) = completed_at {
            message.timestamp = completed_at;
        }
        let (message, attachment_refs) = self.externalize_message_attachments(&dir, &message)?;
        entry.timestamp = message.timestamp;
        entry.content_hash = content_hash(&message)?;
        entry.attachment_refs = attachment_refs;
        write_json_pretty_atomic(&path, &message, "message chunk")?;

        let mut meta = self.read_meta_from_dir(&dir, session_id)?;
        meta.updated_at = updated_at;
        if index.first().is_some_and(|first| first.id == message_id) {
            meta.first_message_preview = first_message_preview(std::slice::from_ref(&message));
        }
        write_json_pretty_atomic(&index_file_in_dir(&dir), &index, "session index")?;
        write_json_pretty_atomic(&meta_file_in_dir(&dir), &meta, "session meta")?;
        self.cache.write().insert(session_id.to_string(), meta);
        Ok(())
    }

    pub(super) fn ensure_session_layout(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<(), String> {
        let dir = session_dir(app_data_dir, session_id)?;
        if meta_file_in_dir(&dir).exists() {
            return Ok(());
        }
        let flat_file = session_file(app_data_dir, session_id)?;
        if !flat_file.exists() {
            return Ok(());
        }
        let _lock = self.file_lock.lock();
        if meta_file_in_dir(&dir).exists() {
            return Ok(());
        }
        let session = self.read_flat_session_file(&flat_file, session_id)?;
        let sessions = sessions_dir(app_data_dir);
        let tmp_dir = sessions.join(format!("{session_id}.migration.tmp"));
        let write_result = (|| -> Result<(), String> {
            let _ = std::fs::remove_dir_all(&tmp_dir);
            self.write_split_session_to_dir(&tmp_dir, &session, false)?;
            std::fs::rename(&tmp_dir, &dir)
                .map_err(|e| format!("Failed to install migrated session dir: {e}"))?;
            Ok(())
        })();
        if let Err(err) = write_result {
            let _ = std::fs::remove_dir_all(&tmp_dir);
            return Err(err);
        }
        std::fs::remove_file(&flat_file)
            .map_err(|e| format!("Failed to remove migrated session file: {e}"))?;
        if let Ok(sidecar) = legacy_meta_file(app_data_dir, session_id) {
            let _ = std::fs::remove_file(sidecar);
        }
        self.cache
            .write()
            .insert(session_id.to_string(), SessionMeta::from_session(&session));
        Ok(())
    }

    pub(super) fn read_message_file(&self, path: &Path) -> Result<ChatMessage, String> {
        #[cfg(test)]
        self.message_read_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let file =
            std::fs::File::open(path).map_err(|e| format!("Failed to read message chunk: {e}"))?;
        serde_json::from_reader(BufReader::new(file))
            .map_err(|e| format!("Failed to parse message chunk: {e}"))
    }

    #[cfg(test)]
    pub(super) fn reset_message_read_count(&self) {
        self.message_read_count
            .store(0, std::sync::atomic::Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(super) fn message_read_count(&self) -> usize {
        self.message_read_count
            .load(std::sync::atomic::Ordering::SeqCst)
    }
    pub(super) fn read_index_from_dir(&self, dir: &Path) -> Result<Vec<MessageIndexEntry>, String> {
        let path = index_file_in_dir(dir);
        let mut index: Vec<MessageIndexEntry> = match std::fs::File::open(&path) {
            Ok(file) => serde_json::from_reader(BufReader::new(file))
                .map_err(|e| format!("Failed to parse session index: {e}"))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                self.rebuild_index_from_messages(dir)?
            }
            Err(e) => return Err(format!("Failed to read session index: {e}")),
        };
        index.sort_by_key(|entry| entry.seq);
        Ok(index)
    }

    pub(super) fn read_consistent_index_from_dir(
        &self,
        dir: &Path,
        session_id: &str,
    ) -> Result<Vec<MessageIndexEntry>, String> {
        let path = index_file_in_dir(dir);
        let mut index: Vec<MessageIndexEntry> = match std::fs::File::open(&path) {
            Ok(file) => match serde_json::from_reader(BufReader::new(file)) {
                Ok(index) => index,
                Err(err) => {
                    log::warn!(
                        "Rebuilding unreadable session index for session {session_id}: {err}"
                    );
                    return self.repair_index_and_meta_from_messages(dir, session_id);
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return self.repair_index_and_meta_from_messages(dir, session_id);
            }
            Err(e) => return Err(format!("Failed to read session index: {e}")),
        };
        index.sort_by_key(|entry| entry.seq);
        if !self.index_matches_message_chunk_sequences(dir, &index)? {
            log::warn!("Rebuilding stale session index for session {session_id}");
            return self.repair_index_and_meta_from_messages(dir, session_id);
        }
        Ok(index)
    }

    fn index_matches_message_chunk_sequences(
        &self,
        dir: &Path,
        index: &[MessageIndexEntry],
    ) -> Result<bool, String> {
        let mut stored_seqs = index.iter().map(|entry| entry.seq).collect::<Vec<_>>();
        stored_seqs.sort_unstable();
        stored_seqs.dedup();
        if stored_seqs.len() != index.len() {
            return Ok(false);
        }

        let mut chunk_seqs = Vec::new();
        let messages_dir = messages_dir_in_dir(dir);
        if messages_dir.exists() {
            for entry in std::fs::read_dir(&messages_dir)
                .map_err(|e| format!("Failed to read messages dir: {e}"))?
            {
                let entry = entry.map_err(|e| format!("Failed to read messages dir entry: {e}"))?;
                let path = entry.path();
                if path.extension().is_none_or(|ext| ext != "json") {
                    continue;
                }
                let Some(seq) = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .and_then(|s| s.parse::<u64>().ok())
                else {
                    continue;
                };
                chunk_seqs.push(seq);
            }
        }
        chunk_seqs.sort_unstable();
        Ok(stored_seqs == chunk_seqs)
    }

    fn index_matches_message_chunks_full(
        &self,
        dir: &Path,
        index: &[MessageIndexEntry],
    ) -> Result<bool, String> {
        let rebuilt = self.rebuild_index_from_messages(dir)?;
        if rebuilt.len() != index.len() {
            return Ok(false);
        }
        Ok(index.iter().zip(rebuilt.iter()).all(|(stored, actual)| {
            stored.seq == actual.seq
                && stored.id == actual.id
                && stored.role == actual.role
                && stored.timestamp == actual.timestamp
                && stored.content_hash == actual.content_hash
                && stored.attachment_refs == actual.attachment_refs
        }))
    }

    pub(super) fn repair_index_and_meta_from_messages(
        &self,
        dir: &Path,
        session_id: &str,
    ) -> Result<Vec<MessageIndexEntry>, String> {
        let index = self.rebuild_index_from_messages(dir)?;
        let mut meta = self.read_meta_from_dir(dir, session_id)?;
        meta.message_count = index.len();
        meta.first_message_preview = self.first_indexed_message_preview(dir, &index)?;
        write_json_pretty_atomic(&index_file_in_dir(dir), &index, "session index")?;
        write_json_pretty_atomic(&meta_file_in_dir(dir), &meta, "session meta")?;
        self.cache
            .write()
            .insert(session_id.to_string(), meta.clone());
        Ok(index)
    }

    pub(super) fn first_indexed_message_preview(
        &self,
        dir: &Path,
        index: &[MessageIndexEntry],
    ) -> Result<String, String> {
        let Some(first) = index.first() else {
            return Ok(String::new());
        };
        let message = self.read_message_file(&message_file_in_dir(dir, first.seq))?;
        Ok(first_message_preview(std::slice::from_ref(&message)))
    }

    pub(super) fn read_page_from_index(
        &self,
        dir: &Path,
        session_id: &str,
        index: &[MessageIndexEntry],
        cursor: Option<PageCursor>,
        limit: usize,
    ) -> Result<(SessionPage, bool), String> {
        let boundary = cursor.map(|c| c.0);
        let eligible: Vec<&MessageIndexEntry> = index
            .iter()
            .filter(|entry| boundary.is_none_or(|cursor_seq| entry.seq < cursor_seq))
            .collect();
        let start = eligible.len().saturating_sub(limit);
        let selected = &eligible[start..];
        let mut messages = Vec::with_capacity(selected.len());
        let mut message_metadata = Vec::with_capacity(selected.len());
        let mut needs_repair = false;
        for entry in selected {
            let path = message_file_in_dir(dir, entry.seq);
            match self.read_message_file(&path) {
                Ok(message) => {
                    if entry.content_hash != content_hash(&message)?
                        || entry.attachment_refs != self.attachment_refs_from_message(&message)
                    {
                        needs_repair = true;
                    }
                    messages.push(message);
                    message_metadata.push(MessagePageMetadata {
                        message_id: entry.id.clone(),
                        token_meta: entry.token_meta.clone(),
                        run_meta: None,
                    });
                }
                Err(err) => {
                    needs_repair = true;
                    log::warn!(
                        "Skipping unreadable message chunk for session {session_id}, seq {}: {err}",
                        entry.seq
                    );
                }
            }
        }
        let has_more = start > 0;
        let next_cursor = if has_more {
            selected.first().map(|entry| PageCursor(entry.seq))
        } else {
            None
        };
        Ok((
            SessionPage {
                messages,
                message_metadata,
                next_cursor,
                has_more,
                total_count: index.len(),
                latest_token_usage: None,
            },
            needs_repair,
        ))
    }

    pub(super) fn rebuild_index_from_messages(
        &self,
        dir: &Path,
    ) -> Result<Vec<MessageIndexEntry>, String> {
        let messages_dir = messages_dir_in_dir(dir);
        if !messages_dir.exists() {
            return Ok(Vec::new());
        }
        let mut index = Vec::new();
        for entry in std::fs::read_dir(&messages_dir)
            .map_err(|e| format!("Failed to read messages dir: {e}"))?
        {
            let entry = entry.map_err(|e| format!("Failed to read messages dir entry: {e}"))?;
            let path = entry.path();
            if path.extension().is_none_or(|ext| ext != "json") {
                continue;
            }
            let Some(seq) = path
                .file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| s.parse::<u64>().ok())
            else {
                continue;
            };
            let message = match self.read_message_file(&path) {
                Ok(message) => message,
                Err(err) => {
                    log::warn!(
                        "Skipping unreadable message chunk while rebuilding index ({}): {err}",
                        path.display()
                    );
                    continue;
                }
            };
            index.push(MessageIndexEntry {
                id: message.id.clone(),
                seq,
                role: message.role.clone(),
                timestamp: message.timestamp,
                content_hash: content_hash(&message)?,
                attachment_refs: self.attachment_refs_from_message(&message),
                token_meta: None,
            });
        }
        index.sort_by_key(|entry| entry.seq);
        Ok(index)
    }

    pub(super) fn load_full_session_from_layout(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<ChatSession, String> {
        let dir = session_dir(app_data_dir, session_id)?;
        if !meta_file_in_dir(&dir).exists() {
            let flat = session_file(app_data_dir, session_id)?;
            if flat.exists() {
                return self.read_flat_session_file(&flat, session_id);
            }
            return Err(format!("Session not found: {session_id}"));
        }
        let meta = self.read_meta_from_dir(&dir, session_id)?;
        let mut index = self.read_consistent_index_from_dir(&dir, session_id)?;
        if !self.index_matches_message_chunks_full(&dir, &index)? {
            log::warn!("Rebuilding stale session index for session {session_id}");
            index = self.repair_index_and_meta_from_messages(&dir, session_id)?;
        }
        let mut messages = Vec::with_capacity(index.len());
        for entry in index {
            let path = message_file_in_dir(&dir, entry.seq);
            match self.read_message_file(&path) {
                Ok(message) => messages.push(self.hydrate_message_attachments(&dir, message)?),
                Err(err) => {
                    log::warn!(
                        "Skipping unreadable message chunk for session {session_id}, seq {}: {err}",
                        entry.seq
                    );
                }
            }
        }
        Ok(meta.to_session(messages))
    }

    pub(super) fn write_split_session_to_dir(
        &self,
        dir: &Path,
        session: &ChatSession,
        reuse_existing_index: bool,
    ) -> Result<(), String> {
        std::fs::create_dir_all(messages_dir_in_dir(dir))
            .map_err(|e| format!("Failed to create messages dir: {e}"))?;
        std::fs::create_dir_all(attachments_dir_in_dir(dir))
            .map_err(|e| format!("Failed to create attachments dir: {e}"))?;
        let old_index = if reuse_existing_index {
            self.read_index_from_dir(dir).unwrap_or_default()
        } else {
            Vec::new()
        };
        let mut old_by_id: HashMap<String, MessageIndexEntry> = old_index
            .into_iter()
            .map(|entry| (entry.id.clone(), entry))
            .collect();
        let mut next_seq = old_by_id
            .values()
            .map(|entry| entry.seq)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        let mut used_seq = std::collections::HashSet::new();
        let mut index = Vec::with_capacity(session.messages.len());
        for message in &session.messages {
            let (stored_message, attachment_refs) =
                self.externalize_message_attachments(dir, message)?;
            let old_entry = old_by_id.remove(&message.id);
            let mut seq = old_entry
                .as_ref()
                .map(|entry| entry.seq)
                .unwrap_or_else(|| {
                    let seq = next_seq;
                    next_seq = next_seq.saturating_add(1);
                    seq
                });
            if !used_seq.insert(seq) {
                seq = next_seq;
                next_seq = next_seq.saturating_add(1);
                used_seq.insert(seq);
            }
            let hash = content_hash(&stored_message)?;
            let chunk_path = message_file_in_dir(dir, seq);
            if old_entry
                .as_ref()
                .is_none_or(|entry| entry.content_hash != hash)
                || !chunk_path.exists()
            {
                write_json_pretty_atomic(&chunk_path, &stored_message, "message chunk")?;
            }
            index.push(MessageIndexEntry {
                id: message.id.clone(),
                seq,
                role: message.role.clone(),
                timestamp: message.timestamp,
                content_hash: hash,
                attachment_refs,
                token_meta: None,
            });
        }
        index.sort_by_key(|entry| entry.seq);
        let mut meta = SessionMeta::from_session(session);
        meta.body_format_version = SESSION_BODY_FORMAT_VERSION;
        write_json_pretty_atomic(&meta_file_in_dir(dir), &meta, "session meta")?;
        write_json_pretty_atomic(&index_file_in_dir(dir), &index, "session index")?;
        self.remove_stale_message_chunks(dir, &used_seq)
    }

    pub(super) fn remove_stale_message_chunks(
        &self,
        dir: &Path,
        used_seq: &std::collections::HashSet<u64>,
    ) -> Result<(), String> {
        let messages_dir = messages_dir_in_dir(dir);
        if !messages_dir.exists() {
            return Ok(());
        }
        for entry in std::fs::read_dir(&messages_dir)
            .map_err(|e| format!("Failed to read messages dir: {e}"))?
        {
            let entry = entry.map_err(|e| format!("Failed to read messages dir entry: {e}"))?;
            let path = entry.path();
            if path.extension().is_none_or(|ext| ext != "json") {
                continue;
            }
            let Some(seq) = path
                .file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| s.parse::<u64>().ok())
            else {
                continue;
            };
            if !used_seq.contains(&seq) {
                std::fs::remove_file(&path)
                    .map_err(|e| format!("Failed to remove stale message chunk: {e}"))?;
            }
        }
        Ok(())
    }
}
