use releashd::test_support::integration::workflow::{FacetFileQueryService, FacetQueryService};
pub(crate) mod tests {
    use super::{FacetFileQueryService, FacetQueryService};

    use releashd::test_support::integration::workflow::FacetRepository;
    use std::fs;

    use releashd::test_support::integration::workflow::facet_FacetKind as FacetKind;
    use releashd::test_support::integration::workflow::WorkflowFacetFileRepository;

    use tempfile::TempDir;

    #[test]
    pub fn list_summaries_preserves_existing_shape_fields() {
        // Given
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("instructions")).unwrap();
        fs::write(tmp.path().join("instructions/impl.md"), "# Title\nBody").unwrap();

        // When
        let summaries = FacetFileQueryService::new(tmp.path())
            .list_facet_summaries(FacetKind::Instruction)
            .unwrap();

        // Then
        assert!(summaries
            .iter()
            .any(|summary| summary.key == "impl" && summary.kind == "instruction"));
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
        let facet = repository.get(FacetKind::Policy, "coding").unwrap();
        let summaries = FacetFileQueryService::new(dir.path())
            .list_facet_summaries(FacetKind::Policy)
            .unwrap();
        // Then
        assert_eq!(facet.file_path().unwrap(), path);
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
    }

    #[test]
    fn test_facet入出力_利用者ファイルの読み込み失敗を返す() {
        // Given
        let dir = TempDir::new().unwrap();
        let repository = WorkflowFacetFileRepository::new(dir.path());
        fs::create_dir_all(dir.path().join("policies/coding.md")).unwrap();
        // When / Then
        assert!(repository.get(FacetKind::Policy, "coding").is_err());
        assert!(FacetFileQueryService::new(dir.path())
            .list_facet_summaries(FacetKind::Policy)
            .is_err());
    }
}

#[test]
fn test_facet一覧_不正なキーも診断とともに残す() {
    use releashd::test_support::integration::workflow::FacetKind;
    // Given
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("policies")).unwrap();
    std::fs::write(dir.path().join("policies/valid.md"), "# Valid").unwrap();
    std::fs::write(dir.path().join("policies/bad key!.md"), "# Invalid").unwrap();
    // When
    let list = FacetFileQueryService::new(dir.path())
        .list_facet_summaries(FacetKind::Policy)
        .unwrap();
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
