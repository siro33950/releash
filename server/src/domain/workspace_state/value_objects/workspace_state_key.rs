#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WorkspaceStateKey(String);

impl WorkspaceStateKey {
    pub(crate) fn from_storage(value: String) -> Self {
        Self(value)
    }
    pub(crate) fn storage_value(&self) -> &str {
        &self.0
    }
}
