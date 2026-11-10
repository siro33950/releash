mod code_query_service_tests {
    use super::super::*;
    use crate::domain::code::CodeError;

    struct FakeFileContent;
    impl FileContentRepository for FakeFileContent {
        fn review_file_metadata_at_ref(
            &self,
            _file_path: &str,
            _git_ref: &str,
        ) -> Result<ReviewSideMetadata, CodeError> {
            Ok(ReviewSideMetadata::Present { size_bytes: 4 })
        }
        fn review_file_bytes_at_ref(
            &self,
            file_path: &str,
            git_ref: &str,
        ) -> Result<ReviewSideBytes, CodeError> {
            Ok(ReviewSideBytes::Present(
                format!("{file_path}@{git_ref}").into_bytes(),
            ))
        }
        fn review_working_tree_metadata(
            &self,
            _file_path: &str,
        ) -> Result<ReviewSideMetadata, CodeError> {
            Ok(ReviewSideMetadata::Present { size_bytes: 7 })
        }
        fn review_working_tree_bytes(
            &self,
            _file_path: &str,
        ) -> Result<ReviewSideBytes, CodeError> {
            Ok(ReviewSideBytes::Present(b"working".to_vec()))
        }
        fn review_working_tree_matches_ref(
            &self,
            file_path: &str,
            git_ref: &str,
        ) -> Result<bool, CodeError> {
            Ok(file_path == git_ref)
        }
        fn review_binary_by_attributes(&self, _file_path: &str) -> Result<bool, CodeError> {
            Ok(false)
        }
    }

    struct FakeDiffComputer;
    impl DiffComputer for FakeDiffComputer {
        fn diff_buffers(
            &self,
            _original: &str,
            _modified: &str,
            _file_path: Option<&str>,
        ) -> Result<Vec<Hunk>, CodeError> {
            Ok(vec![Hunk {
                old_start: 1,
                old_lines: 1,
                new_start: 1,
                new_lines: 1,
                lines: vec!["-a".to_string(), "+b".to_string()],
            }])
        }
    }

    struct FakeBranchBase;
    impl BranchBaseResolver for FakeBranchBase {
        fn resolve_base_branch_name(&self, _path_hint: &str) -> Result<Option<String>, CodeError> {
            Ok(Some("main".to_string()))
        }
        fn resolve_base_commit_oid(
            &self,
            _path_hint: &str,
            base_name: &str,
        ) -> Result<Option<String>, CodeError> {
            // Fake では base 名をそのまま OID 代わりに返し、resolver → file_content の
            // 配線（base 名で解決した値が下流へ渡る）を担保する。
            Ok(Some(base_name.to_string()))
        }
    }

    fn service() -> CodeQueryService {
        CodeQueryService::new(
            Arc::new(FakeFileContent),
            Arc::new(FakeDiffComputer),
            Arc::new(FakeBranchBase),
        )
    }

    #[test]
    fn test_内容同一性_headとbaseの先端を比較元へ渡す() {
        // Given
        let service = service();
        // When / Then
        assert!(service
            .review_working_tree_matches_source(
                "HEAD",
                crate::domain::code::value_objects::review::ReviewContentSource::Head
            )
            .unwrap());
        assert!(service
            .review_working_tree_matches_source(
                "main",
                crate::domain::code::value_objects::review::ReviewContentSource::BranchBaseTip
            )
            .unwrap());
        assert!(!service
            .review_working_tree_matches_source(
                "HEAD",
                crate::domain::code::value_objects::review::ReviewContentSource::BranchBaseTip
            )
            .unwrap());
    }

    #[test]
    fn test_diff_hunks算出は差分のhunkを返す() {
        let s = service();
        let result = s.compute_diff_hunks("a\n", "b\n", None).unwrap();
        assert_eq!(result.hunks.len(), 1);
    }

    #[test]
    fn test_ファイルタブbase比較_merge_base経路を使わずbaseの先端を読む() {
        // Given
        let service = service();
        // When / Then
        assert_eq!(
            service
                .review_file_bytes_at_branch_base_tip("file.rs")
                .unwrap(),
            ReviewSideBytes::Present(b"file.rs@main".to_vec())
        );
        assert_eq!(
            service
                .review_file_metadata_at_branch_base_tip("file.rs")
                .unwrap(),
            ReviewSideMetadata::Present { size_bytes: 4 }
        );
    }
}
