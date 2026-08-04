#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderLifecycleCapabilityHash([u8; 32]);

impl ProviderLifecycleCapabilityHash {
    pub(crate) fn from_digest(digest: [u8; 32]) -> Self {
        Self(digest)
    }

    pub(crate) fn matches(&self, candidate: &Self) -> bool {
        self.0
            .iter()
            .zip(candidate.0.iter())
            .fold(0_u8, |difference, (left, right)| {
                difference | (left ^ right)
            })
            == 0
    }
}
