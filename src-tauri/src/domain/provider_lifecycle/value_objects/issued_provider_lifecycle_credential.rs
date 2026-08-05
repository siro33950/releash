use super::ProviderLifecycleCapabilityHash;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IssuedProviderLifecycleCredential {
    binding_id: String,
    capability: String,
    capability_hash: ProviderLifecycleCapabilityHash,
}

impl IssuedProviderLifecycleCredential {
    pub(crate) fn new(
        binding_id: String,
        capability: String,
        capability_hash: ProviderLifecycleCapabilityHash,
    ) -> Self {
        Self {
            binding_id,
            capability,
            capability_hash,
        }
    }

    pub(crate) fn into_parts(self) -> (String, String, ProviderLifecycleCapabilityHash) {
        (self.binding_id, self.capability, self.capability_hash)
    }
}
