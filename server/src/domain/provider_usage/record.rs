use super::ProviderUsage;
#[derive(Debug, Clone)]
pub struct CodexUsageRecord {
    pub id: String,
    pub timestamp: std::time::SystemTime,
    pub usage: ProviderUsage,
}
