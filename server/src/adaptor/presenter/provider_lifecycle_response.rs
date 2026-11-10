use crate::domain::provider_lifecycle::{
    ProviderLifecycleIngressResult, ProviderLifecycleRejection,
};
pub(crate) fn rejection_reason(reason: ProviderLifecycleRejection) -> &'static str {
    match reason {
        ProviderLifecycleRejection::BindingNotActive => "binding_not_active",
        ProviderLifecycleRejection::InvalidCapability => "invalid_capability",
        ProviderLifecycleRejection::BindingMismatch => "binding_mismatch",
        ProviderLifecycleRejection::ProviderMismatch => "provider_mismatch",
        ProviderLifecycleRejection::ScopeMismatch => "scope_mismatch",
        ProviderLifecycleRejection::BindingExpired => "binding_expired",
        ProviderLifecycleRejection::SessionNotAssociated => "session_not_associated",
        ProviderLifecycleRejection::ProviderSessionMismatch => "provider_session_mismatch",
        ProviderLifecycleRejection::TranscriptMismatch => "transcript_mismatch",
    }
}

#[cfg(test)]
#[path = "provider_lifecycle_response_test.rs"]
mod provider_lifecycle_response_tests;

impl From<ProviderLifecycleIngressResult>
    for crate::adaptor::presenter::client::ReceiveProviderSignalResponse
{
    fn from(result: ProviderLifecycleIngressResult) -> Self {
        use crate::adaptor::presenter::client::ReceiveProviderSignalRejected;
        Self {
            rejected: match result {
                ProviderLifecycleIngressResult::Rejected(reason) => {
                    Some(ReceiveProviderSignalRejected {
                        reason: rejection_reason(reason).into(),
                    })
                }
                _ => None,
            },
        }
    }
}

pub(crate) fn usage_result(
    result: Result<
        ProviderLifecycleIngressResult,
        crate::usecase::provider_lifecycle::ProviderLifecycleIngressUsecaseError,
    >,
) -> Result<(), connectrpc::ConnectError> {
    match result.map_err(crate::adaptor::presenter::connect::classified_error)? {
        ProviderLifecycleIngressResult::Rejected(reason) => Err(
            crate::adaptor::presenter::connect::invalid_request(rejection_reason(reason)),
        ),
        _ => Ok(()),
    }
}
