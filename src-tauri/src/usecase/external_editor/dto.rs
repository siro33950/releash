use crate::domain::external_editor::EditorInfo;

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub struct EditorInfoDto {
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalEditorState {
    pub selected: String,
    pub editors: Vec<EditorInfoDto>,
}

impl From<EditorInfo> for EditorInfoDto {
    fn from(editor: EditorInfo) -> Self {
        Self {
            name: editor.name,
            path: editor.path,
        }
    }
}
