#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderKind {
    Claude,
    Codex,
}

impl ProviderKind {
    pub(crate) fn supported() -> &'static [Self] {
        &[Self::Claude, Self::Codex]
    }
}
