use super::ProviderLifecycleRejection;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderLifecycleIngressResult {
    Applied,
    Ignored,
    Duplicate,
    Rejected(ProviderLifecycleRejection),
}
