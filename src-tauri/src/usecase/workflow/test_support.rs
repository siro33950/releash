use crate::domain::workflow::{WorkflowDefinition, WorkflowError};

use super::ports::WorkflowDefinitionSourceGateway;

pub(crate) struct NoopDefinitionSourceGateway;

impl WorkflowDefinitionSourceGateway for NoopDefinitionSourceGateway {
    fn get_source(&self, _file_stem: &str) -> Result<Option<String>, WorkflowError> {
        Ok(None)
    }

    fn save_source(
        &self,
        _source: &str,
        _original_name: Option<&str>,
    ) -> Result<WorkflowDefinition, WorkflowError> {
        Err(WorkflowError::external("not used"))
    }
}
