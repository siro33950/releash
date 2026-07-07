use std::sync::Arc;

use crate::domain::workflow::{WorkflowDefinition, WorkflowDefinitionRepository, WorkflowError};
use crate::usecase::workflow::ports::WorkflowDefinitionSourceGateway;

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

    pub fn save_workflow_source(
        &self,
        source: &str,
        original_name: Option<&str>,
    ) -> Result<WorkflowDefinition, WorkflowError> {
        self.definition_sources.save_source(source, original_name)
    }

    pub fn delete_workflow(&self, name: &str) -> Result<(), WorkflowError> {
        self.definitions.delete(name)
    }

    pub fn duplicate_workflow(
        &self,
        source_name: &str,
        new_name: &str,
    ) -> Result<(), WorkflowError> {
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
mod tests {
    use super::*;
    use crate::domain::workflow::WorkflowSummary;
    use crate::usecase::workflow::ports::WorkflowDefinitionSourceGateway;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeDefinitionRepository {
        definitions: Mutex<HashMap<String, WorkflowDefinition>>,
        deleted: Mutex<Vec<String>>,
    }

    impl FakeDefinitionRepository {
        fn seed(&self, definition: WorkflowDefinition) {
            self.definitions
                .lock()
                .unwrap()
                .insert(definition.name.clone(), definition);
        }

        fn get_saved(&self, name: &str) -> Option<WorkflowDefinition> {
            self.definitions.lock().unwrap().get(name).cloned()
        }
    }

    impl WorkflowDefinitionRepository for FakeDefinitionRepository {
        fn list(&self, _running_names: &[String]) -> Result<Vec<WorkflowSummary>, WorkflowError> {
            Ok(Vec::new())
        }

        fn get(&self, file_stem: &str) -> Result<Option<WorkflowDefinition>, WorkflowError> {
            Ok(self.definitions.lock().unwrap().get(file_stem).cloned())
        }

        fn save(
            &self,
            definition: WorkflowDefinition,
            _original_name: Option<&str>,
        ) -> Result<(), WorkflowError> {
            self.definitions
                .lock()
                .unwrap()
                .insert(definition.name.clone(), definition);
            Ok(())
        }

        fn delete(&self, name: &str) -> Result<(), WorkflowError> {
            self.deleted.lock().unwrap().push(name.to_string());
            self.definitions.lock().unwrap().remove(name);
            Ok(())
        }
    }

    struct NoopDefinitionSourceGateway;

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

    fn definition(name: &str, builtin: bool) -> WorkflowDefinition {
        WorkflowDefinition {
            name: name.to_string(),
            description: "desc".to_string(),
            builtin,
            variables: Default::default(),
            nodes: Vec::new(),
        }
    }

    #[test]
    fn duplicate_workflow_loads_source_and_saves_copy_as_custom_definition() {
        let definitions = Arc::new(FakeDefinitionRepository::default());
        definitions.seed(definition("source", true));
        let usecase = WorkflowDefinitionUsecase::new(
            definitions.clone(),
            Arc::new(NoopDefinitionSourceGateway),
        );

        usecase.duplicate_workflow("source", "copy").unwrap();

        let saved = definitions.get_saved("copy").unwrap();
        assert_eq!(saved.name, "copy");
        assert!(!saved.builtin);
    }

    #[test]
    fn delete_workflow_delegates_to_definition_repository() {
        let definitions = Arc::new(FakeDefinitionRepository::default());
        definitions.seed(definition("target", false));
        let usecase = WorkflowDefinitionUsecase::new(
            definitions.clone(),
            Arc::new(NoopDefinitionSourceGateway),
        );

        usecase.delete_workflow("target").unwrap();

        assert!(definitions.get_saved("target").is_none());
        assert_eq!(definitions.deleted.lock().unwrap().as_slice(), ["target"]);
    }
}
