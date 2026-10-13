use super::UsageWindow;
use crate::domain::provider_lifecycle::ProviderKind;
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderUsage {
    pub provider: ProviderKind,
    pub windows: Vec<UsageWindow>,
    pub plan: Option<String>,
}
impl ProviderUsage {
    pub fn claude(
        windows: Vec<UsageWindow>,
        organization: Option<&str>,
        tier: Option<&str>,
    ) -> Self {
        let parts: Vec<_> = [organization, tier]
            .into_iter()
            .flatten()
            .filter(|value| !value.is_empty())
            .collect();
        Self {
            provider: ProviderKind::Claude,
            windows,
            plan: (!parts.is_empty()).then(|| parts.join(" · ")),
        }
    }
}

#[cfg(test)]
#[path = "usage_test.rs"]
mod usage_tests;
