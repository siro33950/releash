pub(crate) mod tests {
    use releash_lib::test_support::integration::usecase::workflow::FacetRepository;
    use std::fs;

    use super::super::*;
    use tempfile::TempDir;

    #[test]
    pub fn save_new_rejects_duplicate_key() {
        let tmp = TempDir::new().unwrap();
        let repo = WorkflowFacetFileRepository::new(tmp.path());

        repo.save(FacetKind::Instruction, "impl", "body", true)
            .unwrap();
        let err = repo
            .save(FacetKind::Instruction, "impl", "body", true)
            .unwrap_err();

        assert!(matches!(err, WorkflowError::Validation(_)));
    }

    #[test]
    pub fn test_facet保存_多段template参照を受理する() {
        // Given
        let tmp = TempDir::new().unwrap();
        let repo = WorkflowFacetFileRepository::new(tmp.path());
        let content = "Use {{ context.outer.inner.value }}";

        // When
        repo.save(FacetKind::Instruction, "nested", content, true)
            .unwrap();

        // Then
        assert_eq!(
            fs::read_to_string(tmp.path().join("instructions/nested.md")).unwrap(),
            content
        );
    }

    #[test]
    pub fn list_summaries_preserves_existing_shape_fields() {
        let tmp = TempDir::new().unwrap();
        let repo = WorkflowFacetFileRepository::new(tmp.path());
        repo.save(FacetKind::Instruction, "impl", "# Title\nBody", true)
            .unwrap();

        let summaries = repo.list_summaries(FacetKind::Instruction).unwrap();

        assert!(summaries
            .iter()
            .any(|summary| summary.key == "impl" && summary.kind == "instruction"));
    }

    #[test]
    pub fn save_and_delete_regenerate_facet_editor_stub() {
        let tmp = TempDir::new().unwrap();
        let repo = WorkflowFacetFileRepository::new(tmp.path());

        repo.save(FacetKind::Instruction, "impl", "# Implement", true)
            .unwrap();
        let after_save = fs::read_to_string(tmp.path().join(".releash/facets.lua")).unwrap();
        repo.delete(FacetKind::Instruction, "impl").unwrap();
        let after_delete = fs::read_to_string(tmp.path().join(".releash/facets.lua")).unwrap();

        assert!(after_save.contains("impl ReleashFacet Implement"));
        assert!(!after_delete.contains("impl ReleashFacet Implement"));
    }

    #[test]
    pub fn editor_stub_generation_failure_does_not_fail_facet_save() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join(".releash"), "blocks generated directory").unwrap();
        let repo = WorkflowFacetFileRepository::new(tmp.path());

        repo.save(FacetKind::Instruction, "impl", "body", true)
            .unwrap();

        assert_eq!(
            fs::read_to_string(tmp.path().join("instructions/impl.md")).unwrap(),
            "body"
        );
    }
}
