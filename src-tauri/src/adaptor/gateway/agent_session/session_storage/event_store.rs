use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use super::layout::{event_log_file_in_dir, session_dir, write_json_pretty_atomic_durable};
use super::FileSessionStorage;
use crate::usecase::agent_session::event_log::AgentSessionEvent;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct AppendOutcome {
    recovered: bool,
}

impl FileSessionStorage {
    pub fn load_session_events(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Vec<AgentSessionEvent>, String> {
        if !self.reconcile_session_transaction(app_data_dir, session_id)? {
            return Err(format!("Session not found: {session_id}"));
        }
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

    fn record_event_log_recovery(&self, session_id: &str) {
        self.recovered_event_logs
            .write()
            .insert(session_id.to_string());
        log::warn!(
            "agent_session_event_log_recovered session_id={session_id} recovery=tail_truncation"
        );
    }

    pub(super) fn read_session_events_from_dir(
        &self,
        dir: &Path,
    ) -> Result<Vec<AgentSessionEvent>, String> {
        let path = event_log_file_in_dir(dir);
        match std::fs::read_to_string(&path) {
            Ok(content) => parse_session_events_content(&content),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(format!("Failed to read session event log: {e}")),
        }
    }

    pub(super) fn canonicalize_session_events_from_dir(
        &self,
        dir: &Path,
    ) -> Result<Vec<AgentSessionEvent>, String> {
        let path = event_log_file_in_dir(dir);
        let content = match std::fs::read_to_string(&path) {
            Ok(content) => content,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(format!("Failed to read session event log: {error}")),
        };
        let events = parse_session_events_content(&content)?;
        if serde_json::from_str::<Vec<AgentSessionEvent>>(&content).is_err() {
            write_json_pretty_atomic_durable(&path, &events, "session event log")?;
        }
        Ok(events)
    }

    pub(super) fn append_session_event_to_dir(
        &self,
        dir: &Path,
        event: &AgentSessionEvent,
    ) -> Result<AppendOutcome, String> {
        let path = event_log_file_in_dir(dir);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create session event log dir: {e}"))?;
        }
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|e| format!("Failed to open session event log: {e}"))?;
        let payload = serde_json::to_string_pretty(event)
            .map_err(|e| format!("Failed to serialize session event: {e}"))?;
        let len = file
            .metadata()
            .map_err(|e| format!("Failed to stat session event log: {e}"))?
            .len();
        if len == 0 {
            write!(file, "[\n{}\n]\n", indent_json_payload(&payload))
                .map_err(|e| format!("Failed to append session event: {e}"))?;
            file.flush()
                .map_err(|e| format!("Failed to flush session event log: {e}"))?;
            return Ok(AppendOutcome::default());
        }

        let mut recovered = false;
        let mut closing = last_non_whitespace_byte(&mut file, len)?;
        let needs_recovery = match closing {
            Some((_, b']')) => !event_log_is_valid_json(&mut file)?,
            _ => true,
        };
        if needs_recovery {
            let events = recover_session_events_from_file(&mut file)?;
            rewrite_session_events(&mut file, &events)?;
            let repaired_len = file
                .metadata()
                .map_err(|e| format!("Failed to stat repaired session event log: {e}"))?
                .len();
            closing = last_non_whitespace_byte(&mut file, repaired_len)?;
            recovered = true;
        }
        let Some((closing_pos, b']')) = closing else {
            return Err(
                "Failed to append session event: repaired event log is not a JSON array"
                    .to_string(),
            );
        };
        let previous = last_non_whitespace_byte(&mut file, closing_pos)?;
        let is_empty_array = matches!(previous, Some((_, b'[')));
        file.set_len(closing_pos)
            .map_err(|e| format!("Failed to truncate session event log: {e}"))?;
        file.seek(SeekFrom::End(0))
            .map_err(|e| format!("Failed to seek session event log: {e}"))?;
        if is_empty_array {
            write!(file, "\n{}", indent_json_payload(&payload))
        } else {
            write!(file, ",\n{}", indent_json_payload(&payload))
        }
        .map_err(|e| format!("Failed to append session event: {e}"))?;
        write!(file, "\n]\n").map_err(|e| format!("Failed to close session event log: {e}"))?;
        file.flush()
            .map_err(|e| format!("Failed to flush session event log: {e}"))?;
        Ok(AppendOutcome { recovered })
    }
}

fn recover_session_events_from_file(
    file: &mut std::fs::File,
) -> Result<Vec<AgentSessionEvent>, String> {
    file.seek(SeekFrom::Start(0))
        .map_err(|e| format!("Failed to seek damaged session event log: {e}"))?;
    let mut content = String::new();
    file.read_to_string(&mut content)
        .map_err(|e| format!("Failed to read damaged session event log: {e}"))?;
    recover_unclosed_session_events(&content).map_err(|_| {
        "Failed to append session event: event log tail could not be recovered".to_string()
    })
}

fn rewrite_session_events(
    file: &mut std::fs::File,
    events: &[AgentSessionEvent],
) -> Result<(), String> {
    let payload = serde_json::to_string_pretty(events)
        .map_err(|e| format!("Failed to serialize recovered session event log: {e}"))?;
    file.set_len(0)
        .map_err(|e| format!("Failed to truncate damaged session event log: {e}"))?;
    file.seek(SeekFrom::Start(0))
        .map_err(|e| format!("Failed to seek repaired session event log: {e}"))?;
    writeln!(file, "{payload}")
        .map_err(|e| format!("Failed to rewrite recovered session event log: {e}"))?;
    file.flush()
        .map_err(|e| format!("Failed to flush recovered session event log: {e}"))
}

fn parse_session_events_content(content: &str) -> Result<Vec<AgentSessionEvent>, String> {
    match serde_json::from_str(content) {
        Ok(events) => Ok(events),
        Err(error) => recover_unclosed_session_events(content)
            .map_err(|_| format!("Failed to parse session event log: {error}")),
    }
}

fn recover_unclosed_session_events(content: &str) -> Result<Vec<AgentSessionEvent>, ()> {
    if !content.trim_start().starts_with('[') {
        return Err(());
    }

    let mut end = content.len();
    loop {
        let mut prefix = content[..end].trim_end();
        if let Some(stripped) = prefix.strip_suffix(',') {
            prefix = stripped.trim_end();
        }
        if let Some(candidate) = close_unclosed_json_containers(prefix) {
            if let Ok(events) = serde_json::from_str(&candidate) {
                return Ok(events);
            }
        }
        if end == 0 {
            return Err(());
        }
        end = previous_char_boundary(content, end);
    }
}

fn close_unclosed_json_containers(prefix: &str) -> Option<String> {
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
    let mut candidate = prefix.to_string();
    for opening in stack.into_iter().rev() {
        candidate.push(if opening == '[' { ']' } else { '}' });
    }
    Some(candidate)
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

fn event_log_is_valid_json(file: &mut std::fs::File) -> Result<bool, String> {
    file.seek(SeekFrom::Start(0))
        .map_err(|e| format!("Failed to seek session event log: {e}"))?;
    Ok(serde_json::from_reader::<_, serde::de::IgnoredAny>(file).is_ok())
}

fn last_non_whitespace_byte(
    file: &mut std::fs::File,
    end_exclusive: u64,
) -> Result<Option<(u64, u8)>, String> {
    let mut pos = end_exclusive;
    let mut byte = [0_u8; 1];
    while pos > 0 {
        pos -= 1;
        file.seek(SeekFrom::Start(pos))
            .map_err(|e| format!("Failed to seek session event log: {e}"))?;
        file.read_exact(&mut byte)
            .map_err(|e| format!("Failed to read session event log: {e}"))?;
        if !byte[0].is_ascii_whitespace() {
            return Ok(Some((pos, byte[0])));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usecase::agent_session::event_log::PromptInput;
    use crate::usecase::agent_session::session::{ChatSession, MessagePart, SessionState};
    use std::io::BufReader;

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
        let file = std::fs::File::open(event_log_file_in_dir(dir)).unwrap();
        serde_json::from_reader(BufReader::new(file)).unwrap()
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
    fn last_non_whitespace_byte_skips_trailing_whitespace() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("bytes.txt");
        std::fs::write(&path, b"[1]\n \t").unwrap();
        let mut file = std::fs::File::open(&path).unwrap();
        let len = file.metadata().unwrap().len();

        let found = last_non_whitespace_byte(&mut file, len).unwrap();

        assert_eq!(found, Some((2, b']')));
    }
}
