use crate::domain::external_editor::EditorError;
use crate::domain::external_editor::EditorLauncherGateway;

#[derive(Clone)]
pub struct NativeEditorLauncherGateway;

impl EditorLauncherGateway for NativeEditorLauncherGateway {
    fn open_path(&self, path: &str, editor: &str, label: &str) -> Result<(), EditorError> {
        open_path_with(path, editor, label, |path, editor| match editor {
            Some(editor) => open::with_detached(path, editor),
            None => open::that_detached(path),
        })
    }
}

fn open_path_with(
    path: &str,
    editor: &str,
    label: &str,
    open: impl FnOnce(&str, Option<&str>) -> std::io::Result<()>,
) -> Result<(), EditorError> {
    if editor.is_empty() {
        std::fs::metadata(path)
            .and_then(|_| open(path, None))
            .map_err(|e| EditorError::Launch(format!("{label}を開けませんでした: {e}")))
    } else {
        open(path, Some(editor))
            .map_err(|e| EditorError::Launch(format!("エディタで{label}を開けませんでした: {e}")))
    }
}

#[cfg(test)]
#[path = "launcher_impl_test.rs"]
mod launcher_impl_tests;
