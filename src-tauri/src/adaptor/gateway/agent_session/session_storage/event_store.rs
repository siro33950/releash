use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::layout::{
    event_batches_dir_in_dir, event_log_file_in_dir, event_tail_file_in_dir,
    queue_pause_checkpoint_file_in_dir, session_dir, write_json_pretty_atomic,
    write_json_pretty_atomic_durable,
};
use super::transaction::TransactionApplyError;
use super::FileSessionStorage;
use crate::usecase::agent_session::event_log::{
    apply_event_to_queue_pause, AgentSessionEvent, TurnEventLog,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct AppendOutcome {
    pub(super) recovered: bool,
}

impl FileSessionStorage {
    pub fn load_queue_paused_at(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Option<f64>, String> {
        if !self.reconcile_session_transaction(app_data_dir, session_id)? {
            return Err(format!("Session not found: {session_id}"));
        }
        let _lock = self.file_lock.lock();
        let dir = session_dir(app_data_dir, session_id)?;
        load_queue_pause_projection_from_dir(&dir)
    }

    pub fn load_session_events(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Vec<AgentSessionEvent>, String> {
        if !self.reconcile_session_transaction(app_data_dir, session_id)? {
            return Err(format!("Session not found: {session_id}"));
        }
        let _lock = self.file_lock.lock();
        let dir = session_dir(app_data_dir, session_id)?;
        self.read_session_events_from_dir(&dir)
    }

    pub fn append_session_event(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        event: &AgentSessionEvent,
    ) -> Result<Vec<AgentSessionEvent>, String> {
        if !self.reconcile_session_transaction(app_data_dir, session_id)? {
            return Err(format!("Session not found: {session_id}"));
        }
        let _lock = self.file_lock.lock();
        let dir = session_dir(app_data_dir, session_id)?;
        self.apply_pending_session_transaction(&dir, session_id)?;
        let outcome = self.append_session_event_to_dir(&dir, event)?;
        if outcome.recovered {
            self.record_event_log_recovery(session_id);
        }
        self.read_session_events_from_dir(&dir)
    }

    pub fn append_session_event_without_projection(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        event: &AgentSessionEvent,
    ) -> Result<(), String> {
        if !self.reconcile_session_transaction(app_data_dir, session_id)? {
            return Err(format!("Session not found: {session_id}"));
        }
        let _lock = self.file_lock.lock();
        let dir = session_dir(app_data_dir, session_id)?;
        self.apply_pending_session_transaction(&dir, session_id)?;
        let outcome = self.append_session_event_to_dir(&dir, event)?;
        if outcome.recovered {
            self.record_event_log_recovery(session_id);
        }
        Ok(())
    }

    pub(super) fn record_event_log_recovery(&self, session_id: &str) {
        self.recovered_event_logs
            .write()
            .insert(session_id.to_string());
        log::warn!(
            "agent_session_event_log_recovered session_id={session_id} recovery=tail_truncation"
        );
    }

    pub fn append_session_events(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        events: &[AgentSessionEvent],
    ) -> Result<(), String> {
        if events.is_empty() {
            return Ok(());
        }
        if !self.reconcile_session_transaction(app_data_dir, session_id)? {
            return Err(format!("Session not found: {session_id}"));
        }
        let _lock = self.file_lock.lock();
        let dir = session_dir(app_data_dir, session_id)?;
        self.apply_pending_session_transaction(&dir, session_id)?;
        let outcome = self.append_session_events_to_dir(&dir, events)?;
        if outcome.recovered {
            self.record_event_log_recovery(session_id);
        }
        Ok(())
    }

    pub(super) fn read_session_events_from_dir(
        &self,
        dir: &Path,
    ) -> Result<Vec<AgentSessionEvent>, String> {
        #[cfg(test)]
        self.event_read_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let path = event_log_file_in_dir(dir);
        let mut events = match std::fs::read_to_string(&path) {
            Ok(content) => parse_session_events_content(&content),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(format!("Failed to read session event log: {e}")),
        }?;
        for batch_path in self.committed_event_batch_paths(dir)? {
            let content = std::fs::read_to_string(&batch_path)
                .map_err(|e| format!("Failed to read session event batch: {e}"))?;
            let mut batch = serde_json::from_str::<Vec<AgentSessionEvent>>(&content)
                .map_err(|e| format!("Failed to parse session event batch: {e}"))?;
            events.append(&mut batch);
        }
        let tail_path = event_tail_file_in_dir(dir);
        match std::fs::read_to_string(&tail_path) {
            Ok(content) => events.extend(parse_session_events_content(&content)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("Failed to read session event tail: {error}")),
        }
        Ok(events)
    }

    #[cfg(test)]
    pub(crate) fn reset_event_read_count(&self) {
        self.event_read_count
            .store(0, std::sync::atomic::Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(crate) fn event_read_count(&self) -> usize {
        self.event_read_count
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    #[cfg(test)]
    pub(crate) fn reset_event_batch_directory_scan_count(&self) {
        self.event_batch_directory_scan_count
            .store(0, std::sync::atomic::Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(crate) fn event_batch_directory_scan_count(&self) -> usize {
        self.event_batch_directory_scan_count
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    pub(super) fn canonicalize_session_events_from_dir(
        &self,
        dir: &Path,
    ) -> Result<Vec<AgentSessionEvent>, TransactionApplyError> {
        let mut events = canonicalize_event_array(&event_log_file_in_dir(dir))?;
        for batch_path in self
            .committed_event_batch_paths(dir)
            .map_err(TransactionApplyError::retryable)?
        {
            let content = std::fs::read_to_string(&batch_path).map_err(|error| {
                TransactionApplyError::retryable(format!(
                    "Failed to read session event batch: {error}"
                ))
            })?;
            let mut batch =
                serde_json::from_str::<Vec<AgentSessionEvent>>(&content).map_err(|error| {
                    TransactionApplyError::corrupt(format!(
                        "Failed to parse session event batch: {error}"
                    ))
                })?;
            events.append(&mut batch);
        }
        events.extend(canonicalize_event_array(&event_tail_file_in_dir(dir))?);
        Ok(events)
    }

    pub(super) fn event_append_file_in_dir(dir: &Path) -> std::path::PathBuf {
        if event_batches_dir_in_dir(dir).exists() || event_tail_file_in_dir(dir).exists() {
            event_tail_file_in_dir(dir)
        } else {
            event_log_file_in_dir(dir)
        }
    }

    pub(super) fn append_session_event_to_dir(
        &self,
        dir: &Path,
        event: &AgentSessionEvent,
    ) -> Result<AppendOutcome, String> {
        append_event_log_record(dir, EventLogRecordRef::Event(event), "event")
    }

    pub(super) fn append_session_events_to_dir(
        &self,
        dir: &Path,
        events: &[AgentSessionEvent],
    ) -> Result<AppendOutcome, String> {
        if events.is_empty() {
            return Ok(AppendOutcome::default());
        }
        append_event_log_record(dir, EventLogRecordRef::Batch { events }, "event batch")
    }

    pub(super) fn committed_event_batch_paths(
        &self,
        dir: &Path,
    ) -> Result<Vec<std::path::PathBuf>, String> {
        #[cfg(test)]
        self.event_batch_directory_scan_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        committed_event_batch_paths(dir)
    }
}

fn canonicalize_event_array(path: &Path) -> Result<Vec<AgentSessionEvent>, TransactionApplyError> {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(TransactionApplyError::retryable(format!(
                "Failed to read session event log: {error}"
            )))
        }
    };
    let events = parse_session_events_content(&content).map_err(TransactionApplyError::corrupt)?;
    if serde_json::from_str::<Vec<AgentSessionEvent>>(&content).is_err() {
        write_json_pretty_atomic_durable(path, &events, "session event log")
            .map_err(TransactionApplyError::retryable)?;
    }
    Ok(events)
}

fn committed_event_batch_paths(dir: &Path) -> Result<Vec<std::path::PathBuf>, String> {
    let batches_dir = event_batches_dir_in_dir(dir);
    let entries = match std::fs::read_dir(&batches_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("Failed to read session event batch dir: {error}")),
    };
    let mut paths = Vec::new();
    for entry in entries {
        let path = entry
            .map_err(|error| format!("Failed to read session event batch entry: {error}"))?
            .path();
        if path.extension().and_then(|extension| extension.to_str()) == Some("json")
            && path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .is_some_and(|stem| stem.parse::<u64>().is_ok())
        {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

#[derive(Serialize)]
#[serde(untagged)]
enum EventLogRecordRef<'a> {
    Event(&'a AgentSessionEvent),
    Batch { events: &'a [AgentSessionEvent] },
}

#[derive(Deserialize)]
#[serde(untagged)]
enum EventLogRecord {
    Event(Box<AgentSessionEvent>),
    Batch { events: Vec<AgentSessionEvent> },
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct QueuePauseCheckpoint {
    event_log_len: u64,
    queue_paused_at: Option<f64>,
}

fn append_event_log_record(
    dir: &Path,
    record: EventLogRecordRef<'_>,
    description: &str,
) -> Result<AppendOutcome, String> {
    append_event_log_record_with_fault(dir, record, description, None)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EventLogWriteFault {
    RepairWrite,
    RepairSync,
    RepairBeforeRename,
    AppendAfterPayload,
    AppendAfterClosing,
    AppendSync,
}

fn append_event_log_record_with_fault(
    dir: &Path,
    record: EventLogRecordRef<'_>,
    description: &str,
    fault: Option<EventLogWriteFault>,
) -> Result<AppendOutcome, String> {
    let path = event_log_file_in_dir(dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create session event log dir: {e}"))?;
    }
    let payload = serde_json::to_string_pretty(&record)
        .map_err(|e| format!("Failed to serialize session {description}: {e}"))?;
    let mut content = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(format!("Failed to read session event log: {error}")),
    };
    let len = content.len() as u64;
    let checkpoint = matching_queue_pause_checkpoint(dir, len);
    let mut projection_is_current = checkpoint.is_some();
    let mut queue_paused_at = checkpoint.and_then(|checkpoint| checkpoint.queue_paused_at);
    let mut recovered = false;
    if content.is_empty() {
        content = "[]".to_string();
        projection_is_current = true;
    } else if !projection_is_current {
        match parse_event_log_records(&content) {
            Ok(events) => {
                queue_paused_at = project_queue_pause(events.iter());
            }
            Err(_) => {
                let (events, recovered_content) = recover_unclosed_event_log(&content).map_err(
                    |_| {
                        "Failed to append session event: event log does not end with a JSON array; tail could not be recovered".to_string()
                    },
                )?;
                queue_paused_at = project_queue_pause(events.iter());
                invalidate_queue_pause_checkpoint(dir)?;
                replace_recovered_event_log_atomic(&path, &recovered_content, fault)?;
                content = recovered_content;
                recovered = true;
            }
        }
        projection_is_current = true;
    }
    debug_assert!(projection_is_current);
    invalidate_queue_pause_checkpoint(dir)?;
    let closing_pos = content
        .char_indices()
        .rev()
        .find(|(_, ch)| !ch.is_whitespace())
        .filter(|(_, ch)| *ch == ']')
        .map(|(position, _)| position)
        .ok_or_else(|| {
            "Failed to append session event: event log does not end with a JSON array".to_string()
        })?;
    let prefix = &content[..closing_pos];
    let is_empty_array = prefix.chars().rev().find(|ch| !ch.is_whitespace()) == Some('[');
    let payload = if is_empty_array {
        format!("\n{}", indent_json_payload(&payload))
    } else {
        format!(",\n{}", indent_json_payload(&payload))
    };
    replace_appended_event_log_atomic(&path, prefix, &payload, "\n]\n", fault)?;
    let queue_paused_at = apply_record_to_queue_pause(queue_paused_at, &record);
    let committed_len = std::fs::metadata(&path)
        .map_err(|error| format!("Failed to stat session event log: {error}"))?
        .len();
    persist_queue_pause_checkpoint(dir, committed_len, queue_paused_at);
    Ok(AppendOutcome { recovered })
}

fn replace_recovered_event_log_atomic(
    path: &Path,
    recovered: &str,
    fault: Option<EventLogWriteFault>,
) -> Result<(), String> {
    let temp_path = path.with_extension("json.tmp");
    let result = (|| -> Result<(), String> {
        let mut file = std::fs::File::create(&temp_path)
            .map_err(|error| format!("Failed to create repaired session event log: {error}"))?;
        if fault == Some(EventLogWriteFault::RepairWrite) {
            let split = recovered.len() / 2;
            file.write_all(&recovered.as_bytes()[..split])
                .map_err(|error| format!("Failed to repair session event log: {error}"))?;
            return Err("injected repaired session event log write failure".to_string());
        }
        file.write_all(recovered.as_bytes())
            .map_err(|error| format!("Failed to repair session event log: {error}"))?;
        if fault == Some(EventLogWriteFault::RepairSync) {
            return Err("injected repaired session event log sync failure".to_string());
        }
        file.sync_all()
            .map_err(|error| format!("Failed to sync repaired session event log: {error}"))?;
        if fault == Some(EventLogWriteFault::RepairBeforeRename) {
            return Err("injected crash before repaired session event log rename".to_string());
        }
        std::fs::rename(&temp_path, path)
            .map_err(|error| format!("Failed to rename repaired session event log: {error}"))
    })();
    if result.is_err() && fault != Some(EventLogWriteFault::RepairBeforeRename) {
        let _ = std::fs::remove_file(&temp_path);
    }
    result
}

fn replace_appended_event_log_atomic(
    path: &Path,
    prefix: &str,
    payload: &str,
    closing: &str,
    fault: Option<EventLogWriteFault>,
) -> Result<(), String> {
    let temp_path = path.with_extension("json.tmp");
    let result = (|| -> Result<(), String> {
        let mut file = std::fs::File::create(&temp_path)
            .map_err(|error| format!("Failed to create session event log temp file: {error}"))?;
        file.write_all(prefix.as_bytes())
            .map_err(|error| format!("Failed to write session event log prefix: {error}"))?;
        file.write_all(payload.as_bytes())
            .map_err(|error| format!("Failed to append session event log payload: {error}"))?;
        if fault == Some(EventLogWriteFault::AppendAfterPayload) {
            return Err("injected failure after session event log payload write".to_string());
        }
        file.write_all(closing.as_bytes())
            .map_err(|error| format!("Failed to close session event log: {error}"))?;
        if fault == Some(EventLogWriteFault::AppendAfterClosing) {
            return Err("injected failure after session event log closing write".to_string());
        }
        if fault == Some(EventLogWriteFault::AppendSync) {
            return Err("injected session event log sync failure".to_string());
        }
        file.sync_all()
            .map_err(|error| format!("Failed to sync session event log: {error}"))?;
        std::fs::rename(&temp_path, path)
            .map_err(|error| format!("Failed to rename session event log temp file: {error}"))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp_path);
    }
    result
}

fn load_queue_pause_projection_from_dir(dir: &Path) -> Result<Option<f64>, String> {
    let event_log_path = event_log_file_in_dir(dir);
    let event_log_len = match std::fs::metadata(&event_log_path) {
        Ok(metadata) => metadata.len(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
        Err(error) => return Err(format!("Failed to stat session event log: {error}")),
    };
    if let Some(checkpoint) = matching_queue_pause_checkpoint(dir, event_log_len) {
        return Ok(checkpoint.queue_paused_at);
    }
    let (queue_paused_at, projection_is_checkpointable) =
        match std::fs::read_to_string(&event_log_path) {
            Ok(content) => match parse_event_log_records(&content) {
                Ok(events) => (project_queue_pause(events.iter()), true),
                Err(error) => {
                    let events = recover_unclosed_session_events(&content)
                        .map_err(|_| format!("Failed to parse session event log: {error}"))?;
                    // A recovered in-memory projection must not attest that the damaged file is
                    // current. Leave the checkpoint stale so the next append repairs the log and
                    // publishes the recovery signal.
                    (project_queue_pause(events.iter()), false)
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (None, true),
            Err(error) => return Err(format!("Failed to read session event log: {error}")),
        };
    if projection_is_checkpointable {
        persist_queue_pause_checkpoint(dir, event_log_len, queue_paused_at);
    }
    Ok(queue_paused_at)
}

fn matching_queue_pause_checkpoint(dir: &Path, event_log_len: u64) -> Option<QueuePauseCheckpoint> {
    let content = std::fs::read_to_string(queue_pause_checkpoint_file_in_dir(dir)).ok()?;
    let checkpoint = serde_json::from_str::<QueuePauseCheckpoint>(&content).ok()?;
    (checkpoint.event_log_len == event_log_len).then_some(checkpoint)
}

fn persist_queue_pause_checkpoint(dir: &Path, event_log_len: u64, queue_paused_at: Option<f64>) {
    let checkpoint = QueuePauseCheckpoint {
        event_log_len,
        queue_paused_at,
    };
    if let Err(error) = write_json_pretty_atomic(
        &queue_pause_checkpoint_file_in_dir(dir),
        &checkpoint,
        "queue pause checkpoint",
    ) {
        // event log remains the durable source of truth. A missing/stale checkpoint is rebuilt
        // by the next narrow read instead of turning a committed append into a false failure.
        log::warn!("failed to persist queue pause checkpoint: {error}");
    }
}

fn invalidate_queue_pause_checkpoint(dir: &Path) -> Result<(), String> {
    match std::fs::remove_file(queue_pause_checkpoint_file_in_dir(dir)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "Failed to invalidate queue pause checkpoint: {error}"
        )),
    }
}

fn apply_record_to_queue_pause(
    queue_paused_at: Option<f64>,
    record: &EventLogRecordRef<'_>,
) -> Option<f64> {
    match record {
        EventLogRecordRef::Event(event) => apply_event_to_queue_pause(queue_paused_at, event),
        EventLogRecordRef::Batch { events } => events
            .iter()
            .fold(queue_paused_at, apply_event_to_queue_pause),
    }
}

fn project_queue_pause<'a>(events: impl IntoIterator<Item = &'a AgentSessionEvent>) -> Option<f64> {
    TurnEventLog::from_events(events.into_iter().cloned().collect()).queue_paused_at()
}

fn parse_session_events_content(content: &str) -> Result<Vec<AgentSessionEvent>, String> {
    match parse_event_log_records(content) {
        Ok(events) => Ok(events),
        Err(error) => recover_unclosed_session_events(content)
            .map_err(|_| format!("Failed to parse session event log: {error}")),
    }
}

fn recover_unclosed_session_events(content: &str) -> Result<Vec<AgentSessionEvent>, ()> {
    recover_unclosed_event_log(content).map(|(events, _)| events)
}

fn recover_unclosed_event_log(content: &str) -> Result<(Vec<AgentSessionEvent>, String), ()> {
    if !content.trim_start().starts_with('[') {
        return Err(());
    }

    let mut end = content.len();
    loop {
        let mut prefix = content[..end].trim_end();
        if let Some(stripped) = prefix.strip_suffix(',') {
            prefix = stripped.trim_end();
        }
        if let Some((candidate, closed_nested_container)) = close_unclosed_json_containers(prefix) {
            if let Ok(records) = parse_event_log_record_envelopes(&candidate) {
                let incomplete_batch = closed_nested_container
                    && matches!(records.last(), Some(EventLogRecord::Batch { .. }));
                if !incomplete_batch {
                    return Ok((flatten_event_log_records(records), candidate));
                }
            }
        }
        if end == 0 {
            return Err(());
        }
        end = previous_char_boundary(content, end);
    }
}

fn close_unclosed_json_containers(prefix: &str) -> Option<(String, bool)> {
    let mut stack = Vec::new();
    let mut in_string = false;
    let mut escaped = false;
    for ch in prefix.chars() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '[' | '{' => stack.push(ch),
            ']' if stack.pop() != Some('[') => return None,
            '}' if stack.pop() != Some('{') => return None,
            ']' | '}' => {}
            _ => {}
        }
    }
    if in_string {
        return None;
    }
    let closed_nested_container = stack.len() > 1;
    let mut candidate = prefix.to_string();
    for opening in stack.into_iter().rev() {
        candidate.push(if opening == '[' { ']' } else { '}' });
    }
    Some((candidate, closed_nested_container))
}

fn parse_event_log_record_envelopes(
    content: &str,
) -> Result<Vec<EventLogRecord>, serde_json::Error> {
    serde_json::from_str(content)
}

fn flatten_event_log_records(records: Vec<EventLogRecord>) -> Vec<AgentSessionEvent> {
    records
        .into_iter()
        .flat_map(|record| match record {
            EventLogRecord::Event(event) => vec![*event],
            EventLogRecord::Batch { events } => events,
        })
        .collect()
}

fn parse_event_log_records(content: &str) -> Result<Vec<AgentSessionEvent>, serde_json::Error> {
    parse_event_log_record_envelopes(content).map(flatten_event_log_records)
}

fn previous_char_boundary(value: &str, end: usize) -> usize {
    let mut previous = end.saturating_sub(1);
    while previous > 0 && !value.is_char_boundary(previous) {
        previous -= 1;
    }
    previous
}

fn indent_json_payload(payload: &str) -> String {
    payload
        .lines()
        .map(|line| format!("  {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usecase::agent_session::event_log::PromptInput;
    use crate::usecase::agent_session::session::{
        ChatSession, MessagePart, SessionEventLogRecoverySignal, SessionState,
    };

    const SESSION_ID: &str = "a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d";

    fn event(turn_id: u64) -> AgentSessionEvent {
        AgentSessionEvent::TurnStarted {
            turn_id,
            message_id: format!("m-{turn_id}"),
            assistant_message_id: None,
            prompt: PromptInput::default(),
            at: turn_id as f64,
        }
    }

    fn final_parts_event() -> AgentSessionEvent {
        AgentSessionEvent::FinalPartsRecorded {
            turn_id: 1,
            message_id: "agent-1".to_string(),
            parts: vec![MessagePart::Text {
                content: "completed response".to_string(),
                parent_tool_use_id: None,
            }],
        }
    }

    fn session(session_id: &str, worktree_path: &Path) -> ChatSession {
        ChatSession {
            id: session_id.to_string(),
            worktree_path: worktree_path.to_string_lossy().to_string(),
            messages: Vec::new(),
            state: SessionState::Active,
            error_reason: None,
            created_at: 1.0,
            updated_at: 1.0,
            agent_session_id: None,
            context_carry: None,
            permission_mode: "edit".to_string(),
            plan_mode: false,
            selected_model: None,
            permission_profile_id: None,
            backend_id: Some("claude".to_string()),
            workflow_node_session: false,
            workflow_node_context: None,
            context_epoch: None,
        }
    }

    fn torn_final_parts_fixture() -> String {
        let content = serde_json::to_string_pretty(&vec![final_parts_event()]).unwrap();
        let parts_start = content.find("\"parts\"").unwrap();
        let parts_closing = parts_start + content[parts_start..].find(']').unwrap();
        content[..=parts_closing].to_string()
    }

    fn storage_with_session(tmp: &tempfile::TempDir, session_id: &str) -> FileSessionStorage {
        let storage = FileSessionStorage::default();
        storage
            .save_full_session_for_migration_or_restore(
                tmp.path(),
                &session(session_id, tmp.path()),
            )
            .unwrap();
        storage
    }

    fn read_events(dir: &Path) -> Vec<AgentSessionEvent> {
        let content = std::fs::read_to_string(event_log_file_in_dir(dir)).unwrap();
        parse_session_events_content(&content).unwrap()
    }

    #[test]
    fn append_session_event_to_empty_file_writes_valid_array() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = FileSessionStorage::default();
        std::fs::create_dir_all(tmp.path()).unwrap();
        std::fs::write(event_log_file_in_dir(tmp.path()), "").unwrap();

        let outcome = storage
            .append_session_event_to_dir(tmp.path(), &event(1))
            .unwrap();

        assert!(!outcome.recovered);
        let content = std::fs::read_to_string(event_log_file_in_dir(tmp.path())).unwrap();
        assert!(content.starts_with("[\n"));
        assert!(content.ends_with("\n]\n"));
        assert_eq!(read_events(tmp.path()), vec![event(1)]);
    }

    #[test]
    fn append_session_event_to_empty_array_omits_comma() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = FileSessionStorage::default();
        std::fs::write(event_log_file_in_dir(tmp.path()), "[]").unwrap();

        storage
            .append_session_event_to_dir(tmp.path(), &event(1))
            .unwrap();

        let content = std::fs::read_to_string(event_log_file_in_dir(tmp.path())).unwrap();
        assert!(!content.contains("[],"));
        assert!(!content.contains("[,\n"));
        assert_eq!(read_events(tmp.path()), vec![event(1)]);
    }

    #[test]
    fn append_session_event_to_existing_array_preserves_order() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = FileSessionStorage::default();

        storage
            .append_session_event_to_dir(tmp.path(), &event(1))
            .unwrap();
        storage
            .append_session_event_to_dir(tmp.path(), &event(2))
            .unwrap();

        assert_eq!(read_events(tmp.path()), vec![event(1), event(2)]);
    }

    #[test]
    fn committed_batches_and_following_events_preserve_order() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = FileSessionStorage::default();

        storage
            .append_session_event_to_dir(tmp.path(), &event(1))
            .unwrap();
        storage
            .append_session_events_to_dir(tmp.path(), &[event(2), event(3)])
            .unwrap();
        storage
            .append_session_event_to_dir(tmp.path(), &event(4))
            .unwrap();

        assert_eq!(
            storage.read_session_events_from_dir(tmp.path()).unwrap(),
            vec![event(1), event(2), event(3), event(4)]
        );
    }

    #[test]
    fn incomplete_batch_segments_are_invisible_and_retry_commits_each_event_once() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = FileSessionStorage::default();
        let batch = vec![event(2), event(3), event(4)];
        storage
            .append_session_event_to_dir(tmp.path(), &event(1))
            .unwrap();
        let batches_dir = event_batches_dir_in_dir(tmp.path());
        std::fs::create_dir_all(&batches_dir).unwrap();
        let temp_path = batches_dir.join("00000000000000000001.json.tmp");

        for completed_event_count in 0..batch.len() {
            let mut partial =
                serde_json::to_string_pretty(&batch[..completed_event_count]).unwrap();
            partial.truncate(partial.rfind(']').unwrap());
            std::fs::write(&temp_path, partial).unwrap();

            assert_eq!(
                storage.read_session_events_from_dir(tmp.path()).unwrap(),
                vec![event(1)]
            );
        }

        storage
            .append_session_events_to_dir(tmp.path(), &batch)
            .unwrap();

        assert_eq!(
            storage.read_session_events_from_dir(tmp.path()).unwrap(),
            vec![event(1), event(2), event(3), event(4)]
        );
        assert_eq!(committed_event_batch_paths(tmp.path()).unwrap().len(), 1);
    }

    #[test]
    fn batch_append_repairs_event_arrays_before_tail_rotation_and_records_recovery() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = storage_with_session(&tmp, SESSION_ID);
        let dir = session_dir(tmp.path(), SESSION_ID).unwrap();
        storage
            .append_session_event_to_dir(&dir, &event(1))
            .unwrap();
        storage
            .append_session_events_to_dir(&dir, &[event(2)])
            .unwrap();
        storage
            .append_session_event_to_dir(&dir, &event(3))
            .unwrap();
        for path in [event_log_file_in_dir(&dir), event_tail_file_in_dir(&dir)] {
            let content = std::fs::read_to_string(&path).unwrap();
            let closing_pos = content.rfind(']').unwrap();
            std::fs::write(path, &content[..closing_pos]).unwrap();
        }

        storage
            .append_session_events(tmp.path(), SESSION_ID, &[event(4), event(5)])
            .unwrap();

        let legacy_content = std::fs::read_to_string(event_log_file_in_dir(&dir)).unwrap();
        assert_eq!(
            serde_json::from_str::<Vec<AgentSessionEvent>>(&legacy_content).unwrap(),
            vec![event(1)]
        );
        let batch_paths = committed_event_batch_paths(&dir).unwrap();
        assert_eq!(batch_paths.len(), 3);
        for path in &batch_paths {
            let content = std::fs::read_to_string(path).unwrap();
            serde_json::from_str::<Vec<AgentSessionEvent>>(&content).unwrap();
        }
        assert_eq!(
            serde_json::from_str::<Vec<AgentSessionEvent>>(
                &std::fs::read_to_string(&batch_paths[1]).unwrap()
            )
            .unwrap(),
            vec![event(3)]
        );
        assert_eq!(
            storage.read_session_events_from_dir(&dir).unwrap(),
            (1..=5).map(event).collect::<Vec<_>>()
        );
        assert!(storage.take_event_log_recovered(SESSION_ID));
    }

    #[test]
    fn append_session_event_rejects_unrecoverable_log() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = FileSessionStorage::default();
        std::fs::write(event_log_file_in_dir(tmp.path()), "not a JSON array").unwrap();

        let error = storage
            .append_session_event_to_dir(tmp.path(), &event(1))
            .unwrap_err();

        assert!(error.contains("could not be recovered"));
    }

    #[test]
    fn append_session_event_recovers_unclosed_log_then_appends() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = FileSessionStorage::default();
        storage
            .append_session_event_to_dir(tmp.path(), &event(1))
            .unwrap();
        storage
            .append_session_event_to_dir(tmp.path(), &event(2))
            .unwrap();
        let path = event_log_file_in_dir(tmp.path());
        let content = std::fs::read_to_string(&path).unwrap();
        let closing_pos = content.rfind(']').unwrap();
        std::fs::write(&path, &content[..closing_pos]).unwrap();

        let outcome = storage
            .append_session_event_to_dir(tmp.path(), &event(3))
            .unwrap();

        assert!(outcome.recovered);
        assert_eq!(read_events(tmp.path()), vec![event(1), event(2), event(3)]);
    }

    #[test]
    fn append_session_event_recovers_trailing_partial_event() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = FileSessionStorage::default();
        storage
            .append_session_event_to_dir(tmp.path(), &event(1))
            .unwrap();
        storage
            .append_session_event_to_dir(tmp.path(), &event(2))
            .unwrap();
        let path = event_log_file_in_dir(tmp.path());
        let content = std::fs::read_to_string(&path).unwrap();
        let partial_event_pos = content.find("\"m-2\"").unwrap();
        std::fs::write(&path, &content[..partial_event_pos]).unwrap();

        let outcome = storage
            .append_session_event_to_dir(tmp.path(), &event(3))
            .unwrap();

        assert!(outcome.recovered);
        assert_eq!(read_events(tmp.path()), vec![event(1), event(3)]);
    }

    #[test]
    fn append_session_event_recovers_final_parts_torn_after_inner_array() {
        let tmp = tempfile::tempdir().unwrap();
        let session_id = SESSION_ID;
        let storage = storage_with_session(&tmp, session_id);
        let dir = session_dir(tmp.path(), session_id).unwrap();
        std::fs::write(event_log_file_in_dir(&dir), torn_final_parts_fixture()).unwrap();

        let events = storage
            .append_session_event(tmp.path(), session_id, &event(2))
            .unwrap();

        assert_eq!(
            events
                .iter()
                .filter(|candidate| **candidate == event(2))
                .count(),
            1
        );
        assert!(events.contains(&final_parts_event()));
        let content = std::fs::read_to_string(event_log_file_in_dir(&dir)).unwrap();
        serde_json::from_str::<Vec<AgentSessionEvent>>(&content).unwrap();
    }

    #[test]
    fn read_session_events_recovers_final_parts_torn_after_inner_array() {
        let tmp = tempfile::tempdir().unwrap();
        let session_id = SESSION_ID;
        let storage = storage_with_session(&tmp, session_id);
        let dir = session_dir(tmp.path(), session_id).unwrap();
        std::fs::write(event_log_file_in_dir(&dir), torn_final_parts_fixture()).unwrap();

        let events = storage.load_session_events(tmp.path(), session_id).unwrap();

        assert_eq!(events, vec![final_parts_event()]);
    }

    #[test]
    fn append_without_projection_repairs_final_parts_torn_after_inner_array() {
        let tmp = tempfile::tempdir().unwrap();
        let session_id = SESSION_ID;
        let storage = storage_with_session(&tmp, session_id);
        let dir = session_dir(tmp.path(), session_id).unwrap();
        std::fs::write(event_log_file_in_dir(&dir), torn_final_parts_fixture()).unwrap();

        storage
            .append_session_event_without_projection(tmp.path(), session_id, &event(2))
            .unwrap();

        let content = std::fs::read_to_string(event_log_file_in_dir(&dir)).unwrap();
        let events = serde_json::from_str::<Vec<AgentSessionEvent>>(&content).unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|candidate| **candidate == event(2))
                .count(),
            1
        );
        assert!(events.contains(&final_parts_event()));
    }

    #[test]
    fn read_session_events_recovers_missing_closing_array_bracket() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = FileSessionStorage::default();
        storage
            .append_session_event_to_dir(tmp.path(), &event(1))
            .unwrap();
        storage
            .append_session_event_to_dir(tmp.path(), &event(2))
            .unwrap();
        let path = event_log_file_in_dir(tmp.path());
        let content = std::fs::read_to_string(&path).unwrap();
        let closing_pos = content.rfind(']').unwrap();
        std::fs::write(&path, &content[..closing_pos]).unwrap();

        let events = storage.read_session_events_from_dir(tmp.path()).unwrap();

        assert_eq!(events, vec![event(1), event(2)]);
    }

    #[test]
    fn append_repairs_missing_closing_array_bracket_before_writing() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = FileSessionStorage::default();
        storage
            .append_session_event_to_dir(tmp.path(), &event(1))
            .unwrap();
        storage
            .append_session_event_to_dir(tmp.path(), &event(2))
            .unwrap();
        let path = event_log_file_in_dir(tmp.path());
        let content = std::fs::read_to_string(&path).unwrap();
        let closing_pos = content.rfind(']').unwrap();
        std::fs::write(&path, &content[..closing_pos]).unwrap();

        storage
            .append_session_event_to_dir(tmp.path(), &event(3))
            .unwrap();

        assert_eq!(read_events(tmp.path()), vec![event(1), event(2), event(3)]);
    }

    #[test]
    fn read_session_events_ignores_incomplete_trailing_event() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = FileSessionStorage::default();
        storage
            .append_session_event_to_dir(tmp.path(), &event(1))
            .unwrap();
        storage
            .append_session_event_to_dir(tmp.path(), &event(2))
            .unwrap();
        let path = event_log_file_in_dir(tmp.path());
        let content = std::fs::read_to_string(&path).unwrap();
        let partial_event_pos = content.find("\"m-2\"").unwrap();
        std::fs::write(&path, &content[..partial_event_pos]).unwrap();

        let events = storage.read_session_events_from_dir(tmp.path()).unwrap();

        assert_eq!(events, vec![event(1)]);
    }

    #[test]
    fn append_session_event_handles_trailing_whitespace_after_array() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = FileSessionStorage::default();
        storage
            .append_session_event_to_dir(tmp.path(), &event(1))
            .unwrap();
        std::fs::OpenOptions::new()
            .append(true)
            .open(event_log_file_in_dir(tmp.path()))
            .unwrap()
            .write_all(b" \n\t")
            .unwrap();

        storage
            .append_session_event_to_dir(tmp.path(), &event(2))
            .unwrap();

        assert_eq!(read_events(tmp.path()), vec![event(1), event(2)]);
    }

    #[test]
    fn append_session_events_commits_the_batch_in_order() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = FileSessionStorage::default();
        storage
            .append_session_event_to_dir(tmp.path(), &event(1))
            .unwrap();

        storage
            .append_session_events_to_dir(tmp.path(), &[event(2), event(3)])
            .unwrap();

        assert_eq!(read_events(tmp.path()), vec![event(1), event(2), event(3)]);
        let temp_files = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .count();
        assert_eq!(temp_files, 0);
        let raw = std::fs::read_to_string(event_log_file_in_dir(tmp.path())).unwrap();
        assert!(raw.contains("\"events\""));
    }

    #[test]
    fn incomplete_batch_envelope_recovers_without_partial_batch_visibility() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = FileSessionStorage::default();
        storage
            .append_session_event_to_dir(tmp.path(), &event(1))
            .unwrap();
        let path = event_log_file_in_dir(tmp.path());
        let mut content = std::fs::read_to_string(&path).unwrap();
        let closing = content.rfind(']').unwrap();
        content.truncate(closing);
        content.push_str(",\n  {\"events\":[{\"type\":\"queue_paused\",\"at\":2.0}");
        std::fs::write(&path, content).unwrap();

        assert_eq!(
            storage.read_session_events_from_dir(tmp.path()).unwrap(),
            vec![event(1)]
        );
    }

    #[test]
    fn append_discards_an_incomplete_batch_and_remains_writable() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = FileSessionStorage::default();
        storage
            .append_session_event_to_dir(tmp.path(), &event(1))
            .unwrap();
        let path = event_log_file_in_dir(tmp.path());
        let mut content = std::fs::read_to_string(&path).unwrap();
        let closing = content.rfind(']').unwrap();
        content.truncate(closing);
        content.push_str(",\n  {\"events\":[{\"type\":\"queue_paused\",\"at\":2.0}");
        std::fs::write(&path, content).unwrap();

        storage
            .append_session_event_to_dir(tmp.path(), &event(3))
            .unwrap();

        assert_eq!(read_events(tmp.path()), vec![event(1), event(3)]);
    }

    #[test]
    fn queue_pause_checkpoint_avoids_reloading_a_long_event_history() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = FileSessionStorage::default();
        for turn_id in 1..=100 {
            storage
                .append_session_event_to_dir(tmp.path(), &event(turn_id))
                .unwrap();
        }
        storage
            .append_session_event_to_dir(tmp.path(), &AgentSessionEvent::QueuePaused { at: 101.0 })
            .unwrap();
        assert_eq!(
            load_queue_pause_projection_from_dir(tmp.path()).unwrap(),
            Some(101.0)
        );

        let event_log_path = event_log_file_in_dir(tmp.path());
        let len = std::fs::metadata(&event_log_path).unwrap().len() as usize;
        std::fs::write(&event_log_path, vec![b'x'; len]).unwrap();

        assert_eq!(
            load_queue_pause_projection_from_dir(tmp.path()).unwrap(),
            Some(101.0)
        );
        assert!(
            std::fs::metadata(queue_pause_checkpoint_file_in_dir(tmp.path()))
                .unwrap()
                .len()
                < 128
        );
    }

    #[test]
    fn append_session_events_failure_leaves_the_existing_log_unchanged() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = FileSessionStorage::default();
        let path = event_log_file_in_dir(tmp.path());
        std::fs::write(&path, "{\n").unwrap();

        let error = storage
            .append_session_events_to_dir(tmp.path(), &[event(1), event(2)])
            .unwrap_err();

        assert!(error.contains("does not end with a JSON array"));
        assert_eq!(std::fs::read_to_string(path).unwrap(), "{\n");
    }

    #[test]
    fn recovery_failures_preserve_the_complete_prefix_and_allow_retry() {
        for fault in [
            EventLogWriteFault::RepairWrite,
            EventLogWriteFault::RepairSync,
            EventLogWriteFault::RepairBeforeRename,
        ] {
            let tmp = tempfile::tempdir().unwrap();
            let storage = FileSessionStorage::default();
            storage
                .append_session_event_to_dir(tmp.path(), &event(1))
                .unwrap();
            storage
                .append_session_event_to_dir(tmp.path(), &event(2))
                .unwrap();
            let path = event_log_file_in_dir(tmp.path());
            let content = std::fs::read_to_string(&path).unwrap();
            let closing_pos = content.rfind(']').unwrap();
            std::fs::write(&path, &content[..closing_pos]).unwrap();

            let error = append_event_log_record_with_fault(
                tmp.path(),
                EventLogRecordRef::Event(&event(3)),
                "event",
                Some(fault),
            )
            .unwrap_err();

            assert!(error.contains("injected"), "unexpected {fault:?}: {error}");
            let reopened = FileSessionStorage::default();
            assert_eq!(
                reopened.read_session_events_from_dir(tmp.path()).unwrap(),
                vec![event(1), event(2)],
                "complete prefix changed after {fault:?}"
            );
            reopened
                .append_session_event_to_dir(tmp.path(), &event(3))
                .unwrap();
            assert_eq!(
                reopened.read_session_events_from_dir(tmp.path()).unwrap(),
                vec![event(1), event(2), event(3)]
            );
        }
    }

    #[test]
    fn append_faults_leave_the_previous_replay_and_bytes_unchanged() {
        for fault in [
            EventLogWriteFault::AppendAfterPayload,
            EventLogWriteFault::AppendAfterClosing,
            EventLogWriteFault::AppendSync,
        ] {
            let tmp = tempfile::tempdir().unwrap();
            let storage = FileSessionStorage::default();
            storage
                .append_session_event_to_dir(tmp.path(), &event(1))
                .unwrap();
            let path = event_log_file_in_dir(tmp.path());
            let bytes_before = std::fs::read(&path).unwrap();
            let events_before = storage.read_session_events_from_dir(tmp.path()).unwrap();
            let batch = [AgentSessionEvent::QueuePaused { at: 2.0 }, event(2)];

            let error = append_event_log_record_with_fault(
                tmp.path(),
                EventLogRecordRef::Batch { events: &batch },
                "event batch",
                Some(fault),
            )
            .unwrap_err();

            assert!(error.contains("injected"), "unexpected {fault:?}: {error}");
            assert_eq!(std::fs::read(&path).unwrap(), bytes_before);
            let reopened = FileSessionStorage::default();
            assert_eq!(
                reopened.read_session_events_from_dir(tmp.path()).unwrap(),
                events_before
            );
            assert!(!reopened
                .read_session_events_from_dir(tmp.path())
                .unwrap()
                .iter()
                .any(|event| matches!(event, AgentSessionEvent::QueuePaused { .. })));
        }
    }
}
