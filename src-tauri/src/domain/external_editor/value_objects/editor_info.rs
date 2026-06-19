use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EditorInfo {
    pub name: String,
    pub path: String,
}
