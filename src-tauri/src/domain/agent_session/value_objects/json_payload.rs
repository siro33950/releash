#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct JsonPayload(String);

impl JsonPayload {
    /// Construct a JSON payload whose validity has already been checked at a boundary.
    pub fn new_unchecked(raw: String) -> Self {
        Self(raw)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[allow(dead_code)] // issues-1301 G-1: owned JSON extraction is retained for backend wire adapters that currently borrow via as_str.
    pub fn into_string(self) -> String {
        self.0
    }
}
