use super::{CodexUsageObservation, CodexUsageRecord, ProviderUsage};
pub trait ProviderUsageRepository: Send + Sync {
    fn replace(&self, usage: ProviderUsage);
    fn start_observation(&self, observation: CodexUsageObservation);
    fn has_observation(&self, session: &str) -> bool;
    fn accept_record(&self, session: &str, record: &CodexUsageRecord) -> bool;
    fn retain_observations(&self, sessions: &[String], poll_started_at: std::time::SystemTime);
}
