use sha2::{Digest, Sha256};

use crate::domain::provider_lifecycle::{
    IssuedProviderLifecycleCredential, ProviderLifecycleCapabilityHash,
    ProviderLifecycleCredentialGateway,
};

#[derive(Default)]
pub(crate) struct LocalProviderLifecycleCredentialGateway;

impl ProviderLifecycleCredentialGateway for LocalProviderLifecycleCredentialGateway {
    fn issue(&self) -> IssuedProviderLifecycleCredential {
        let binding_id = uuid::Uuid::new_v4().simple().to_string();
        let capability = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let capability_hash = self.hash(&capability);
        IssuedProviderLifecycleCredential::new(binding_id, capability, capability_hash)
    }

    fn hash(&self, capability: &str) -> ProviderLifecycleCapabilityHash {
        ProviderLifecycleCapabilityHash::from_digest(Sha256::digest(capability.as_bytes()).into())
    }
}
