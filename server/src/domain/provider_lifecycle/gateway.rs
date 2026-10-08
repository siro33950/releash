use super::{IssuedProviderLifecycleCredential, ProviderLifecycleCapabilityHash};

pub trait ProviderLifecycleCredentialGateway: Send + Sync {
    fn issue(&self) -> IssuedProviderLifecycleCredential;

    fn hash(&self, capability: &str) -> ProviderLifecycleCapabilityHash;
}
