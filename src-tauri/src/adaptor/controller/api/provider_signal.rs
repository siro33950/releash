use crate::domain::provider_lifecycle::{
    ProviderKind, ProviderLifecycleIngressResult, ProviderLifecycleScope, ProviderLifecycleSlotId,
};

pub(super) fn ingress_context(
    provider: impl Into<ProviderKind>,
    slot_id: &str,
    agent_session_id: &str,
) -> Result<
    (
        ProviderKind,
        ProviderLifecycleSlotId,
        ProviderLifecycleScope,
    ),
    crate::domain::provider_lifecycle::ProviderLifecycleInputError,
> {
    Ok((
        provider.into(),
        ProviderLifecycleSlotId::new(slot_id)?,
        ProviderLifecycleScope::new(agent_session_id)?,
    ))
}
impl From<crate::adaptor::presenter::client::agent_session_provider_dto::Value> for ProviderKind {
    fn from(value: crate::adaptor::presenter::client::agent_session_provider_dto::Value) -> Self {
        match value {
            crate::adaptor::presenter::client::agent_session_provider_dto::Value::Claude => {
                Self::Claude
            }
            crate::adaptor::presenter::client::agent_session_provider_dto::Value::Codex => {
                Self::Codex
            }
        }
    }
}
pub(super) fn record_ingress<E>(
    result: &Result<(ProviderLifecycleIngressResult, bool), E>,
    elapsed: std::time::Duration,
) {
    if matches!(result, Ok((ProviderLifecycleIngressResult::Applied, true))) {
        crate::infrastructure::telemetry::metrics::record_terminal_launch(
            crate::infrastructure::telemetry::metrics::TerminalLaunch::HookIngress,
            elapsed,
        );
    }
}
