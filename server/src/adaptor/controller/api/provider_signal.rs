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
    result: &Result<
        (
            ProviderLifecycleIngressResult,
            bool,
            Option<crate::usecase::provider_lifecycle::ingress::ProviderUsagePreparation>,
        ),
        E,
    >,
    elapsed: std::time::Duration,
) {
    if matches!(
        result,
        Ok((ProviderLifecycleIngressResult::Applied, true, _))
    ) {
        crate::infrastructure::telemetry::metrics::record_terminal_launch(
            crate::infrastructure::telemetry::metrics::TerminalLaunch::HookIngress,
            elapsed,
        );
    }
}

pub(super) async fn prepare_usage(
    deadline: Option<std::time::Instant>,
    preparation: crate::usecase::provider_lifecycle::ingress::ProviderUsagePreparation,
) -> Result<(), crate::usecase::provider_usage::UsageError> {
    use crate::common::operation_context::{current, ingress_result, wait};
    use crate::usecase::provider_usage::UsageError;
    ingress_result(
        deadline,
        async {
            wait(&current(), preparation)
                .await
                .map_err(|error| UsageError::Technical(error.into()))?
        },
        |_, error| UsageError::Technical(error.into()),
    )
    .await
}

#[cfg(test)]
#[path = "provider_signal_test.rs"]
mod provider_signal_tests;
