use super::ProviderLifecycleRejection;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProviderLifecycleIngressResult {
    Applied,
    Duplicate,
    Rejected(ProviderLifecycleRejection),
}
