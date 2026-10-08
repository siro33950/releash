use std::sync::Arc;

use crate::domain::workflow::{WorkflowDefinition, WorkflowDefinitionRepository, WorkflowError};
use crate::usecase::workflow::ports::{WorkflowDefinitionSourceGateway, WorkflowSourceSaveError};

#[derive(Clone)]
pub struct WorkflowDefinitionUsecase {
    definitions: Arc<dyn WorkflowDefinitionRepository>,
    definition_sources: Arc<dyn WorkflowDefinitionSourceGateway>,
}

impl WorkflowDefinitionUsecase {
    pub fn new(
        definitions: Arc<dyn WorkflowDefinitionRepository>,
        definition_sources: Arc<dyn WorkflowDefinitionSourceGateway>,
    ) -> Self {
        Self {
            definitions,
            definition_sources,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn save_workflow_source(
        &self,
        source: &str,
        original_name: Option<&str>,
    ) -> Result<WorkflowDefinition, WorkflowError> {
        self.definition_sources.save_source(source, original_name)
    }

    pub fn save_workflow_source_with_diagnostics(
        &self,
        source: &str,
        original_name: Option<&str>,
    ) -> Result<WorkflowDefinition, WorkflowSourceSaveError> {
        self.definition_sources
            .save_source_with_diagnostics(source, original_name)
    }

    pub fn delete_workflow(&self, name: &str) -> Result<(), WorkflowError> {
        self.definitions.delete(name)
    }

    pub fn duplicate_workflow(
        &self,
        source_name: &str,
        new_name: &str,
    ) -> Result<(), WorkflowError> {
        if self.definition_sources.source_format(source_name)?
            == crate::domain::workflow::WorkflowSourceFormat::Lua
        {
            return Err(WorkflowError::validation(
                "Lua workflow は複製できません。外部エディタで編集してください",
            ));
        }
        let mut definition = self.definitions.get(source_name)?.ok_or_else(|| {
            WorkflowError::NotFound(format!(
                "ソースワークフロー '{source_name}' が見つかりません"
            ))
        })?;
        definition.name = new_name.to_string();
        definition.builtin = false;
        self.definitions.save(definition, None)
    }
}

#[cfg(test)]
#[path = "definition_test.rs"]
mod definition_tests;
