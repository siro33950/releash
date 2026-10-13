use super::CodexUsageRecord;
#[derive(Debug, Clone)]
pub struct CodexUsageObservation {
    pub session: String,
    started_at: std::time::SystemTime,
    last_record: Option<String>,
}
impl CodexUsageObservation {
    pub fn new(session: &str, started_at: std::time::SystemTime) -> Self {
        Self {
            session: session.into(),
            started_at,
            last_record: None,
        }
    }
    pub fn retain_for_poll(
        &self,
        sessions: &[String],
        poll_started_at: std::time::SystemTime,
    ) -> bool {
        self.started_at >= poll_started_at || sessions.contains(&self.session)
    }
    pub fn accept(&mut self, record: &CodexUsageRecord) -> bool {
        let remainder = match self.started_at.duration_since(std::time::UNIX_EPOCH) {
            Ok(duration) => duration.subsec_nanos() % 1_000_000,
            Err(error) => (1_000_000 - error.duration().subsec_nanos() % 1_000_000) % 1_000_000,
        };
        let boundary = self.started_at - std::time::Duration::from_nanos(u64::from(remainder));
        if record.timestamp < boundary || self.last_record.as_ref() == Some(&record.id) {
            return false;
        }
        self.last_record = Some(record.id.clone());
        true
    }
}
#[cfg(test)]
#[path = "observation_test.rs"]
mod observation_tests;
