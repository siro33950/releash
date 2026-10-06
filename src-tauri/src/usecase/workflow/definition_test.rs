pub(crate) mod tests {
    use super::super::*;
    use crate::usecase::workflow::test_helpers::FakeDefinitionRepository;
    use crate::usecase::workflow::test_support::NoopDefinitionSourceGateway;

    fn definition(name: &str, builtin: bool) -> WorkflowDefinition {
        WorkflowDefinition {
            name: name.to_string(),
            description: "desc".to_string(),
            builtin,
            schemas: Default::default(),
            nodes: Vec::new(),
            entry: "main".to_string(),
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

    struct LuaDefinitionSourceGateway;

    impl WorkflowDefinitionSourceGateway for LuaDefinitionSourceGateway {
        fn get_source(&self, _file_stem: &str) -> Result<Option<String>, WorkflowError> {
            Ok(None)
        }

        fn source_format(
            &self,
            _file_stem: &str,
        ) -> Result<crate::domain::workflow::WorkflowSourceFormat, WorkflowError> {
            Ok(crate::domain::workflow::WorkflowSourceFormat::Lua)
        }

        fn save_source(
            &self,
            _source: &str,
            _original_name: Option<&str>,
        ) -> Result<WorkflowDefinition, WorkflowError> {
            unreachable!()
        }
    }

    #[test]
    fn duplicate_workflow_rejects_lua_source() {
        let definitions = Arc::new(FakeDefinitionRepository::default());
        definitions.seed(definition("source", false));
        let usecase =
            WorkflowDefinitionUsecase::new(definitions, Arc::new(LuaDefinitionSourceGateway));

        let error = usecase.duplicate_workflow("source", "copy").unwrap_err();

        assert!(error.to_string().contains("Lua workflow"));
    }
}
