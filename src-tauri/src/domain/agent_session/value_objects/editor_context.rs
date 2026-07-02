#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EditorContext {
    pub active_editor_path: Option<String>,
    pub open_editor_paths: Vec<String>,
    pub selection: Option<EditorSelection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorSelection {
    pub file_path: String,
    pub start_line: u32,
    pub end_line: u32,
}
