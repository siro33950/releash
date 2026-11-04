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
        match self {
            Self::Claude => "Claude",
            Self::Codex => "Codex",
        }
    }
}

#[cfg(test)]
#[path = "provider_kind_test.rs"]
mod provider_kind_tests;
