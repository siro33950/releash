use std::io::BufReader;
use std::path::Path;

use super::layout::{event_log_file_in_dir, session_dir, write_json_pretty_atomic};
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
        let mut events = self.read_session_events_from_dir(&dir)?;
        events.push(event.clone());
        write_json_pretty_atomic(&event_log_file_in_dir(&dir), &events, "session event log")?;
        Ok(events)
    }

    pub(super) fn read_session_events_from_dir(
        &self,
        dir: &Path,
    ) -> Result<Vec<AgentSessionEvent>, String> {
        let path = event_log_file_in_dir(dir);
        match std::fs::File::open(&path) {
            Ok(file) => serde_json::from_reader(BufReader::new(file))
                .map_err(|e| format!("Failed to parse session event log: {e}")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(format!("Failed to read session event log: {e}")),
        }
    }
}
