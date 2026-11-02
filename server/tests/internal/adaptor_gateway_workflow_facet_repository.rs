pub(crate) mod tests {

    use releashd::test_support::integration::workflow::FacetRepository;
    use std::fs;

    use releashd::test_support::integration::workflow::facet_FacetKind as FacetKind;
    use releashd::test_support::integration::workflow::WorkflowFacetFileRepository;
    use releashd::test_support::integration::workflow::{Facet, WorkflowError};
    use tempfile::TempDir;

    #[test]
    pub fn save_new_rejects_duplicate_key() {
        let tmp = TempDir::new().unwrap();
        let repo = WorkflowFacetFileRepository::new(tmp.path());

        save(&repo, "impl", "body", true).unwrap();
        let err = save(&repo, "impl", "body", true).unwrap_err();

        assert!(matches!(err, WorkflowError::Validation(_)));
    }

    #[test]
    pub fn test_facet保存_多段template参照を受理する() {
        // Given
        let tmp = TempDir::new().unwrap();
        let repo = WorkflowFacetFileRepository::new(tmp.path());
        let content = "Use {{ context.outer.inner.value }}";

        // When
        save(&repo, "nested", content, true).unwrap();

        // Then
        assert_eq!(
            fs::read_to_string(tmp.path().join("instructions/nested.md")).unwrap(),
            content
        );
    }

    #[test]
    pub fn list_summaries_preserves_existing_shape_fields() {
        todo!()
    }

    #[test]
    pub fn save_and_delete_regenerate_facet_editor_stub() {
        let tmp = TempDir::new().unwrap();
        let repo = WorkflowFacetFileRepository::new(tmp.path());

        save(&repo, "impl", "# Implement", true).unwrap();
        let after_save = fs::read_to_string(tmp.path().join(".releash/facets.lua")).unwrap();
        repo.delete(&repo.get(FacetKind::Instruction, "impl").unwrap())
            .unwrap();
        let after_delete = fs::read_to_string(tmp.path().join(".releash/facets.lua")).unwrap();

        assert!(after_save.contains("impl ReleashFacet Implement"));
        assert!(!after_delete.contains("impl ReleashFacet Implement"));
    }

    #[test]
    pub fn editor_stub_generation_failure_does_not_fail_facet_save() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join(".releash"), "blocks generated directory").unwrap();
        let repo = WorkflowFacetFileRepository::new(tmp.path());

        save(&repo, "impl", "body", true).unwrap();

        assert_eq!(
            fs::read_to_string(tmp.path().join("instructions/impl.md")).unwrap(),
            "body"
        );
    }

    fn save(
        repo: &WorkflowFacetFileRepository,
        key: &str,
        content: &str,
        is_new: bool,
    ) -> Result<(), WorkflowError> {
        let mut facet = repo.get_or_new(FacetKind::Instruction, key)?;
        facet.update_content(content, is_new, &repo.list(FacetKind::Instruction)?)?;
        repo.save(facet, is_new)
    }

    #[test]
    fn test_facet取得と一覧_builtinと同名の利用者ファイルはfile由来() {
        todo!()
    }

    #[test]
    fn test_facet取得_同梱facetはbuiltin由来() {
        todo!()
    }

    #[test]
    fn test_facet取得_未保存なら空本文の利用者facetを返す() {
        // Given
        let dir = TempDir::new().unwrap();
        let repository = WorkflowFacetFileRepository::new(dir.path());
        // When
        let facet: Facet = repository
            .get_or_new(FacetKind::Instruction, "new-facet")
            .unwrap();
        // Then
        assert!(facet.content().is_empty());
        assert_eq!(
            facet.file_path().unwrap(),
            dir.path().join("instructions/new-facet.md")
        );
        assert!(!facet.file_path().unwrap().exists());
        assert!(matches!(
            repository.get(FacetKind::Instruction, "new-facet"),
            Err(WorkflowError::NotFound(_))
        ));
    }

    #[test]
    fn test_facet入出力_利用者ファイルの読み書き失敗を返す() {
        todo!()
    }
}

#[test]
fn test_facet一覧_不正なキーも診断とともに残す() {
    todo!()
}
