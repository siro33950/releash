pub(crate) mod tests {
    use super::super::*;
    use crate::usecase::workflow::test_helpers::FakeDefinitionRepository;
    use crate::usecase::workflow::test_support::NoopDefinitionSourceGateway;

    #[test]
    fn duplicate_workflow_loads_source_and_saves_copy_as_custom_definition() {
        todo!()
    }

    #[test]
    fn delete_workflow_delegates_to_definition_repository() {
        todo!()
    }

    struct LuaDefinitionSourceGateway;

    impl WorkflowDefinitionSourceService for LuaDefinitionSourceGateway {
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

    #[test]
    fn duplicate_workflow_rejects_lua_source() {
        todo!()
    }
}
