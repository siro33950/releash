#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ProviderKind {
    Claude,
    Codex,
}

impl ProviderKind {
    pub(crate) fn supported() -> &'static [Self] {
        &[Self::Claude, Self::Codex]
    }
}
