use super::*;
use crate::domain::app_config::{
    repository::ConfigUpdate, value_objects::AppConfigDocument, AppConfigError,
};
use crate::domain::external_editor::EditorError;
use crate::domain::failure::{ClassifiedFailure, FailureKind};

struct RemoveEditorTarget(PathBuf);

impl ConfigRepository for RemoveEditorTarget {
    fn load(&self) -> Result<AppConfigDocument, AppConfigError> {
        std::fs::remove_file(&self.0).unwrap();
        Ok(
            crate::adaptor::gateway::app_config::config_models::config_to_domain(
                &crate::adaptor::gateway::app_config::config_models::ReleashConfig::default(),
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
fn test_エディタ起動失敗_workflowとfacetが元の分類を保持する() {
    // Given
    for is_facet in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = if is_facet {
            facet::save_facet(facet::FacetKind::Instruction, "custom", "body", dir.path()).unwrap();
            resolve_facet_editor_path(dir.path(), "instructions", "custom").unwrap()
        } else {
            let path = dir.path().join("custom.lua");
            std::fs::write(&path, "return nil").unwrap();
            path
        };
        let gateway = WorkflowExternalEditorGateway {
            config: Arc::new(RemoveEditorTarget(path)),
            workflows_dir: dir.path().to_path_buf(),
            facets_base_dir: dir.path().to_path_buf(),
        };
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
        assert_eq!(error.failure_kind(), FailureKind::StateRequired);
        let direct = crate::adaptor::protocol::connect::classified_error(EditorError::Launch(
            "missing".into(),
        ));
        assert_eq!(
            crate::adaptor::protocol::connect::classified_error(error).code,
            direct.code
        );
    }
}
