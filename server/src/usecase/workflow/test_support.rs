use crate::domain::workflow::{WorkflowDefinitionContent, WorkflowError};

use super::ports::WorkflowDefinitionSourceService;

pub(crate) struct NoopDefinitionSourceGateway;

impl WorkflowDefinitionSourceService for NoopDefinitionSourceGateway {
    fn get_source(&self, _file_stem: &str) -> Result<Option<String>, WorkflowError> {
        Ok(None)
    }

    fn parse(
        &self,
        _source: &str,
    ) -> Result<
        crate::domain::workflow::WorkflowDefinition,
        crate::domain::workflow::WorkflowSourceSaveError,
    > {
        todo!()
    }
}
