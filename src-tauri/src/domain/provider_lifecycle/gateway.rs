use super::{IssuedProviderLifecycleCredential, ProviderLifecycleCapabilityHash};

pub(crate) trait ProviderLifecycleCredentialGateway: Send + Sync {
    fn issue(&self) -> IssuedProviderLifecycleCredential;

    fn hash(&self, capability: &str) -> ProviderLifecycleCapabilityHash;
}
