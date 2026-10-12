#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WorkspaceStateKey(String);

impl WorkspaceStateKey {
    pub(crate) fn from_storage(value: String) -> Self {
        Self(value)
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn for_test(value: impl Into<String>) -> Self {
        Self::from_storage(value.into())
    }
    pub(crate) fn storage_value(&self) -> &str {
        &self.0
    }
}
