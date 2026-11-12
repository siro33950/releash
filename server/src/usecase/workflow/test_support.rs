#[cfg(test)]
use crate::domain::workflow::WorkflowError;

pub struct NoopEditorSettingsGateway;
impl crate::domain::external_editor::EditorSettingsGateway for NoopEditorSettingsGateway {
    fn selected_editor(&self) -> Result<String, crate::domain::external_editor::EditorError> {
        Ok(String::new())
    }
}

pub struct NoopEditorLauncherGateway;
impl crate::domain::external_editor::EditorLauncherGateway for NoopEditorLauncherGateway {
    fn open_paths(
        &self,
        _: &[std::path::PathBuf],
        _: &str,
        _: &str,
    ) -> Result<(), crate::domain::external_editor::EditorError> {
        Ok(())
    }
}

#[cfg(test)]
pub(crate) struct NoopDirectoryService;
#[cfg(test)]
impl crate::domain::workflow::WorkflowDirectoryService for NoopDirectoryService {
    fn prepare(&self) -> Result<crate::domain::workflow::WorkflowDirectory, WorkflowError> {
        Ok(crate::domain::workflow::WorkflowDirectory::new(
            "/workflows".into(),
        ))
    }
}
