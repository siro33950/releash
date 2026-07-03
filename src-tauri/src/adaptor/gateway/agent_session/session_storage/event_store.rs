use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use super::layout::{event_log_file_in_dir, session_dir};
use super::FileSessionStorage;
use crate::usecase::agent_session::event_log::AgentSessionEvent;

impl FileSessionStorage {
    pub fn load_session_events(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<Vec<AgentSessionEvent>, String> {
        self.ensure_loaded(app_data_dir)?;
        if let Some(err) = self.invalid_sessions.read().get(session_id) {
            return Err(err.clone());
        }
        if !self.cache.read().contains_key(session_id) {
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
        self.ensure_loaded(app_data_dir)?;
        if let Some(err) = self.invalid_sessions.read().get(session_id) {
            return Err(err.clone());
        }
        if !self.cache.read().contains_key(session_id) {
            return Err(format!("Session not found: {session_id}"));
        }
        let _lock = self.file_lock.lock();
        let dir = session_dir(app_data_dir, session_id)?;
        self.append_session_event_to_dir(&dir, event)?;
        self.read_session_events_from_dir(&dir)
    }

    pub fn append_session_event_without_projection(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        event: &AgentSessionEvent,
    ) -> Result<(), String> {
        self.ensure_loaded(app_data_dir)?;
        if let Some(err) = self.invalid_sessions.read().get(session_id) {
            return Err(err.clone());
        }
        if !self.cache.read().contains_key(session_id) {
            return Err(format!("Session not found: {session_id}"));
        }
        let _lock = self.file_lock.lock();
        let dir = session_dir(app_data_dir, session_id)?;
        self.append_session_event_to_dir(&dir, event)
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

    fn append_session_event_to_dir(
        &self,
        dir: &Path,
        event: &AgentSessionEvent,
    ) -> Result<(), String> {
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
            return Ok(());
        }

        let Some((closing_pos, b']')) = last_non_whitespace_byte(&mut file, len)? else {
            return Err(
                "Failed to append session event: event log does not end with a JSON array"
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
        Ok(())
    }
}

fn parse_session_events_content(content: &str) -> Result<Vec<AgentSessionEvent>, String> {
    match serde_json::from_str(content) {
        Ok(events) => Ok(events),
        Err(error) => {
            if last_non_whitespace_char(content) == Some(']') {
                return Err(format!("Failed to parse session event log: {error}"));
            }
            recover_unclosed_session_events(content)
                .map_err(|_| format!("Failed to parse session event log: {error}"))
        }
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
        let candidate = format!("{prefix}\n]\n");
        if let Ok(events) = serde_json::from_str(&candidate) {
            return Ok(events);
        }
        if end == 0 {
            return Err(());
        }
        end = previous_char_boundary(content, end);
    }
}

fn previous_char_boundary(value: &str, end: usize) -> usize {
    let mut previous = end.saturating_sub(1);
    while previous > 0 && !value.is_char_boundary(previous) {
        previous -= 1;
    }
    previous
}

fn last_non_whitespace_char(value: &str) -> Option<char> {
    value.chars().rev().find(|ch| !ch.is_whitespace())
}

fn indent_json_payload(payload: &str) -> String {
    payload
        .lines()
        .map(|line| format!("  {line}"))
        .collect::<Vec<_>>()
        .join("\n")
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
    use std::io::BufReader;

    fn event(turn_id: u64) -> AgentSessionEvent {
        AgentSessionEvent::TurnStarted {
            turn_id,
            message_id: format!("m-{turn_id}"),
            assistant_message_id: None,
            prompt: PromptInput::default(),
            at: turn_id as f64,
        }
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

        storage
            .append_session_event_to_dir(tmp.path(), &event(1))
            .unwrap();

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
    fn append_session_event_rejects_log_that_does_not_end_with_array() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = FileSessionStorage::default();
        std::fs::write(event_log_file_in_dir(tmp.path()), "[{}").unwrap();

        let error = storage
            .append_session_event_to_dir(tmp.path(), &event(1))
            .unwrap_err();

        assert!(error.contains("does not end with a JSON array"));
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
