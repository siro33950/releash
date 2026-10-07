use connectrpc::ErrorCode;
use releashd::test_support::integration::platform::EditorError;
use releashd::test_support::integration::repository::ConfigRepository;
use releashd::test_support::integration::repository::ConfigUpdate;
use releashd::test_support::integration::settings::AppConfigDocument;
use releashd::test_support::integration::settings::AppConfigError;
use releashd::test_support::integration::transport::ConnectFailure;
use releashd::test_support::integration::workflow::resolve_facet_editor_path;
use releashd::test_support::integration::workflow::ExternalEditorGateway;
use releashd::test_support::integration::workflow::WorkflowError;
use releashd::test_support::integration::workflow::WorkflowExternalEditorGateway;
use std::path::PathBuf;
use std::sync::Arc;

struct RemoveEditorTarget(PathBuf);

impl ConfigRepository for RemoveEditorTarget {
    fn load(&self) -> Result<AppConfigDocument, AppConfigError> {
        std::fs::remove_file(&self.0).unwrap();
        Ok(
            releashd::test_support::integration::settings::config_to_domain(
                &releashd::test_support::integration::settings::ReleashConfig::default(),
            ),
        )
    }

    fn save(&self, _: AppConfigDocument) -> Result<(), AppConfigError> {
        unreachable!()
    }

    fn update(&self, _: ConfigUpdate) -> Result<(), AppConfigError> {
        unreachable!()
    }
}

#[test]
pub fn test_エディタ起動失敗_workflowとfacetが元の分類を保持する() {
    // Given
    for is_facet in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = if is_facet {
            releashd::test_support::integration::workflow::save_facet(
                releashd::test_support::integration::workflow::FacetKind::Instruction,
                "custom",
                "body",
                dir.path(),
            )
            .unwrap();
            resolve_facet_editor_path(dir.path(), "instructions", "custom").unwrap()
        } else {
            let path = dir.path().join("custom.lua");
            std::fs::write(&path, "return nil").unwrap();
            path
        };
        let gateway = WorkflowExternalEditorGateway::test_new(
            Arc::new(RemoveEditorTarget(path)),
            dir.path().to_path_buf(),
            dir.path().to_path_buf(),
        );
        // When
        let error = if is_facet {
            gateway.open_facet("instructions", "custom")
        } else {
            gateway.open_workflow("custom")
        }
        .unwrap_err();
        // Then
        assert!(matches!(
            error,
            WorkflowError::Editor(EditorError::Launch(_))
        ));
        assert_eq!(error.connect_code(), ErrorCode::FailedPrecondition);
        let direct = releashd::test_support::integration::transport::classified_error(
            EditorError::Launch("missing".into()),
        );
        assert_eq!(
            releashd::test_support::integration::transport::classified_error(error).code,
            direct.code
        );
    }
}
pub(crate) mod tests {

    use releashd::test_support::integration::workflow::definition_FacetRefs as FacetRefs;
    use releashd::test_support::integration::workflow::resolve_facet_editor_path;
    use releashd::test_support::integration::workflow::resolve_workflow_editor_path;
    use releashd::test_support::integration::workflow::NodeDefinition;
    use releashd::test_support::integration::workflow::NodeKind;
    use releashd::test_support::integration::workflow::SessionSpec;
    use releashd::test_support::integration::workflow::WorkflowDefinition as WorkflowDefinitionYaml;
    use tempfile::TempDir;

    #[test]
    pub fn workflow_editor_path_rejects_builtin_and_resolves_custom_file() {
        let tmp = TempDir::new().unwrap();
        let workflow = WorkflowDefinitionYaml {
            name: "custom".to_string(),
            description: String::new(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![NodeDefinition {
                name: "main".to_string(),
                kind: NodeKind::Session(SessionSpec {
                    facets: FacetRefs {
                        instruction: Some("implement".to_string()),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
                ..NodeDefinition::default()
            }],
            entry: "main".to_string(),
        };
        releashd::test_support::integration::workflow::save_workflow(tmp.path(), &workflow)
            .unwrap();

        let path = resolve_workflow_editor_path(tmp.path(), "custom").unwrap();

        assert_eq!(path.file_name().unwrap(), "custom.yml");
        if let Some(summary) =
            releashd::test_support::integration::workflow::list_builtin_workflows().first()
        {
            assert!(resolve_workflow_editor_path(tmp.path(), &summary.name).is_err());
        }
    }

    #[test]
    pub fn workflow_editor_path_resolves_lua_file() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("custom-lua.lua"), "return nil").unwrap();

        let path = resolve_workflow_editor_path(tmp.path(), "custom-lua").unwrap();

        assert_eq!(path.file_name().unwrap(), "custom-lua.lua");
    }

    #[test]
    pub fn facet_editor_path_rejects_builtin_and_resolves_custom_file() {
        let tmp = TempDir::new().unwrap();
        releashd::test_support::integration::workflow::save_facet(
            releashd::test_support::integration::workflow::FacetKind::Instruction,
            "custom",
            "body",
            tmp.path(),
        )
        .unwrap();

        let path = resolve_facet_editor_path(tmp.path(), "instructions", "custom").unwrap();

        assert_eq!(path.file_name().unwrap(), "custom.md");
        if let Some(key) = releashd::test_support::integration::workflow::list_builtin_facet_keys(
            releashd::test_support::integration::workflow::FacetKind::Instruction,
        )
        .first()
        {
            assert!(resolve_facet_editor_path(tmp.path(), "instructions", key).is_err());
        }
        assert!(resolve_facet_editor_path(tmp.path(), "persona", "custom").is_err());
    }
}
