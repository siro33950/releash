pub(crate) mod tests {
    use super::super::*;
    use crate::usecase::workflow::test_helpers::FakeDefinitionRepository;
    use crate::usecase::workflow::test_support::{
        NoopDefinitionSourceGateway, NoopDirectoryService, NoopEditorLauncherGateway,
        NoopEditorSettingsGateway,
    };
    fn definition(name: &str, builtin: bool) -> WorkflowDefinition {
        WorkflowDefinition::restore(
            name.into(),
            "/workflows".into(),
            if builtin {
                crate::domain::workflow::WorkflowSource::Builtin
            } else {
                crate::domain::workflow::WorkflowSource::File {
                    path: format!("/workflows/{name}.yml").into(),
                }
            },
            crate::domain::workflow::WorkflowSourceFormat::Yaml,
            Ok(crate::domain::workflow::WorkflowDefinitionContent {
                name: name.into(),
                ..Default::default()
            }),
        )
    }

    #[test]
    fn duplicate_workflow_loads_source_and_saves_copy_as_custom_definition() {
        let definitions = Arc::new(FakeDefinitionRepository::default());
        definitions.seed(definition("source", true));
        let usecase = WorkflowDefinitionUsecase::new(
            definitions.clone(),
            Arc::new(NoopDefinitionSourceGateway),
            Arc::new(NoopEditorSettingsGateway),
            Arc::new(NoopEditorLauncherGateway),
            Arc::new(NoopDirectoryService),
        );

        usecase.duplicate_workflow("source", "copy").unwrap();

        let saved = definitions.get_saved("copy").unwrap();
        assert_eq!(saved.file_stem(), "copy");
        assert!(!saved.source().is_builtin());
    }

    #[test]
    fn delete_workflow_delegates_to_definition_repository() {
        let definitions = Arc::new(FakeDefinitionRepository::default());
        definitions.seed(definition("target", false));
        let usecase = WorkflowDefinitionUsecase::new(
            definitions.clone(),
            Arc::new(NoopDefinitionSourceGateway),
            Arc::new(NoopEditorSettingsGateway),
            Arc::new(NoopEditorLauncherGateway),
            Arc::new(NoopDirectoryService),
        );

        usecase.delete_workflow("target").unwrap();

        assert!(definitions.get_saved("target").is_none());
        assert_eq!(definitions.deleted.lock().unwrap().as_slice(), ["target"]);
    }

    #[test]
    fn duplicate_workflow_rejects_lua_source() {
        let definitions = Arc::new(FakeDefinitionRepository::default());
        definitions.seed(WorkflowDefinition::restore(
            "source".into(),
            "/workflows".into(),
            crate::domain::workflow::WorkflowSource::File {
                path: "/workflows/source.lua".into(),
            },
            crate::domain::workflow::WorkflowSourceFormat::Lua,
            Ok(crate::domain::workflow::WorkflowDefinitionContent {
                name: "source".into(),
                ..Default::default()
            }),
        ));
        let usecase = WorkflowDefinitionUsecase::new(
            definitions,
            Arc::new(NoopDefinitionSourceGateway),
            Arc::new(NoopEditorSettingsGateway),
            Arc::new(NoopEditorLauncherGateway),
            Arc::new(NoopDirectoryService),
        );

        let error = usecase.duplicate_workflow("source", "copy").unwrap_err();

        assert!(error.to_string().contains("Lua workflow"));
    }
}

#[test]
fn test_source保存エラー変換_業務エラーと診断を保持する() {
    use super::*;
    use crate::domain::workflow::value_objects::diagnostic::{
        DiagnosticItem, DiagnosticStage, Severity,
    };
    // Given
    let error = WorkflowError::external("save failed");
    let diagnostic = DiagnosticItem {
        code: "WFS001".into(),
        severity: Severity::Error,
        stage: DiagnosticStage::ParseShape,
        span: None,
        message: "invalid source".into(),
        workflow_name: None,
        node_name: None,
        facet_key: None,
        facet_kind: None,
        field: None,
    };
    // When / Then
    assert_eq!(
        source_save_error_to_workflow_error(WorkflowSourceSaveError::Workflow(error.clone())),
        error
    );
    assert_eq!(
        source_save_error_to_workflow_error(WorkflowSourceSaveError::Diagnostics(vec![diagnostic])),
        WorkflowError::validation("workflow_diagnostics: WFS001: invalid source")
    );
}
