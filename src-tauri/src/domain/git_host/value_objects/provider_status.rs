#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderStatus {
    Available,
    CliNotFound { cli: String },
    NotAuthenticated,
    UnsupportedPlatform,
    NoRemote,
}
