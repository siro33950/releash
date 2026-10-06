use crate::domain::provider_lifecycle::{
    ProviderLifecycleIngressResult, ProviderLifecycleRejection,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ProviderLifecycleReceiveResponse {
    Applied,
    Ignored,
    Duplicate,
    Rejected { reason: String },
}

impl From<ProviderLifecycleIngressResult> for ProviderLifecycleReceiveResponse {
    fn from(result: ProviderLifecycleIngressResult) -> Self {
        match result {
            ProviderLifecycleIngressResult::Ignored => Self::Ignored,
            ProviderLifecycleIngressResult::Applied => Self::Applied,
            ProviderLifecycleIngressResult::Duplicate => Self::Duplicate,
            ProviderLifecycleIngressResult::Rejected(reason) => Self::Rejected {
                reason: rejection_reason(reason).into(),
            },
        }
    }
}

pub(crate) fn rejection_reason(reason: ProviderLifecycleRejection) -> &'static str {
    match reason {
        ProviderLifecycleRejection::BindingNotActive => "binding_not_active",
        ProviderLifecycleRejection::InvalidCapability => "invalid_capability",
        ProviderLifecycleRejection::BindingMismatch => "binding_mismatch",
        ProviderLifecycleRejection::ProviderMismatch => "provider_mismatch",
        ProviderLifecycleRejection::ScopeMismatch => "scope_mismatch",
        ProviderLifecycleRejection::BindingExpired => "binding_expired",
        ProviderLifecycleRejection::SessionAlreadyAssociated => "session_already_associated",
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
        use crate::adaptor::presenter::client::{
            receive_provider_signal_response::Result as Wire, ReceiveProviderSignalRejected,
        };
        Self {
            result: Some(match result {
                ProviderLifecycleIngressResult::Applied => Wire::Applied(Default::default()),
                ProviderLifecycleIngressResult::Duplicate => Wire::Duplicate(Default::default()),
                ProviderLifecycleIngressResult::Ignored => Wire::Ignored(Default::default()),
                ProviderLifecycleIngressResult::Rejected(reason) => {
                    Wire::Rejected(ReceiveProviderSignalRejected {
                        reason: rejection_reason(reason).into(),
                    })
                }
            }),
        }
    }
}
