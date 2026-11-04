#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderKind {
    Claude,
    Codex,
}

impl ProviderKind {
    pub(crate) fn supported() -> &'static [Self] {
        &[Self::Claude, Self::Codex]
    }

    /// 利用者に見せる provider の名前。
    pub fn display_name(self) -> &'static str {
        todo!()
    }
}
