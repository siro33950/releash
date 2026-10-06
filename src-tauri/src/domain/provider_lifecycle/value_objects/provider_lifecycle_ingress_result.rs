use super::ProviderLifecycleRejection;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderLifecycleIngressResult {
    Applied,
    Duplicate,
    Rejected(ProviderLifecycleRejection),
}
