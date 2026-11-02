pub(crate) mod tests {

    use releashd::test_support::integration::workflow::FacetRepository;
    use std::fs;

    use releashd::test_support::integration::workflow::facet_FacetKind as FacetKind;
    use releashd::test_support::integration::workflow::WorkflowFacetFileRepository;
    use releashd::test_support::integration::workflow::{Facet, WorkflowError, WorkflowSource};
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
        let tmp = TempDir::new().unwrap();
        let repo = WorkflowFacetFileRepository::new(tmp.path());
        save(&repo, "impl", "# Title\nBody", true).unwrap();

        let summaries = repo.list_summaries(FacetKind::Instruction).unwrap();

        assert!(summaries
            .iter()
            .any(|summary| summary.key == "impl" && summary.kind == "instruction"));
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
        // Given
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join("policies")).unwrap();
        let path = dir.path().join("policies/coding.md");
        fs::write(&path, "# User coding\nBody").unwrap();
        let repository = WorkflowFacetFileRepository::new(dir.path());
        // When
        let mut facet = repository.get(FacetKind::Policy, "coding").unwrap();
        let summaries = repository.list_summaries(FacetKind::Policy).unwrap();
        // Then
        assert!(matches!(facet.source(), WorkflowSource::File { path: source } if source == &path));
        let row = summaries
            .iter()
            .find(|summary| summary.key == "coding")
            .unwrap();
        assert!(!row.builtin);
        assert_eq!(row.description, "User coding");
        assert_eq!(
            summaries
                .iter()
                .filter(|summary| summary.key == "coding")
                .count(),
            1
        );
        facet
            .update_content(
                "# Updated",
                false,
                &repository.list(FacetKind::Policy).unwrap(),
            )
            .unwrap();
        repository.save(facet.clone(), false).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "# Updated");
        repository.delete(&facet).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn test_facet取得_同梱facetはbuiltin由来() {
        // Given
        let dir = TempDir::new().unwrap();
        let repository = WorkflowFacetFileRepository::new(dir.path());
        // When
        let facet = repository.get(FacetKind::Policy, "coding").unwrap();
        // Then
        assert_eq!(facet.source(), &WorkflowSource::Builtin);
        assert!(!facet.content().is_empty());
        assert!(
            repository
                .list_summaries(FacetKind::Policy)
                .unwrap()
                .iter()
                .find(|summary| summary.key == "coding")
                .unwrap()
                .builtin
        );
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
        // Given
        let dir = TempDir::new().unwrap();
        let repository = WorkflowFacetFileRepository::new(dir.path());
        fs::create_dir_all(dir.path().join("policies/coding.md")).unwrap();
        let facet = repository
            .get_or_new(FacetKind::Instruction, "write-failure")
            .unwrap();
        fs::create_dir_all(facet.file_path().unwrap()).unwrap();
        // When / Then
        assert!(repository.get(FacetKind::Policy, "coding").is_err());
        assert!(repository.get_or_new(FacetKind::Policy, "coding").is_err());
        assert!(repository.list_summaries(FacetKind::Policy).is_err());
        assert!(repository.save(facet.clone(), true).is_err());
        assert!(repository.delete(&facet).is_err());
        assert_eq!(
            fs::read_dir(dir.path().join("instructions"))
                .unwrap()
                .count(),
            1
        );
    }
}

#[test]
fn test_facet一覧_不正なキーも診断とともに残す() {
    use releashd::test_support::integration::workflow::{
        FacetKind, FacetRepository, WorkflowFacetFileRepository,
    };
    // Given
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("policies")).unwrap();
    std::fs::write(dir.path().join("policies/valid.md"), "# Valid").unwrap();
    std::fs::write(dir.path().join("policies/bad key!.md"), "# Invalid").unwrap();
    let repository = WorkflowFacetFileRepository::new(dir.path());
    // When
    let list = repository.list_summaries(FacetKind::Policy).unwrap();
    // Then
    assert!(list
        .iter()
        .any(|row| row.key == "valid" && row.description == "Valid" && !row.builtin));
    let invalid = list.iter().find(|row| row.key == "bad key!").unwrap();
    assert_eq!(invalid.kind, "policy");
    assert!(invalid.description.is_empty());
    assert!(!invalid.builtin);
    let report =
        releashd::test_support::integration::workflow::diagnose_all(dir.path(), dir.path())
            .unwrap();
    assert_eq!(report.facet_summaries["policy/bad key!"].error_count, 1);
    assert!(report
        .items
        .iter()
        .any(|item| item.code == "FAC001" && item.facet_key.as_deref() == Some("bad key!")));
}
