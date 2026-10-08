use crate::domain::external_editor::{EditorInfo, InstalledEditorGateway};

pub fn detect_editors(scanner: &dyn InstalledEditorGateway) -> Vec<EditorInfo> {
    scanner.scan()
}
