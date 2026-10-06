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
        fn review_file_metadata_at_branch_base(
            &self,
            _file_path: &str,
            _base_commit_oid: Option<&str>,
        ) -> Result<ReviewSideMetadata, CodeError> {
            Ok(ReviewSideMetadata::Present { size_bytes: 4 })
        }
        fn review_file_bytes_at_branch_base(
            &self,
            _file_path: &str,
            base_commit_oid: Option<&str>,
        ) -> Result<ReviewSideBytes, CodeError> {
            Ok(ReviewSideBytes::Present(
                format!("base@{}", base_commit_oid.unwrap_or("HEAD")).into_bytes(),
            ))
        }
        fn review_staged_metadata(
            &self,
            _file_path: &str,
        ) -> Result<ReviewSideMetadata, CodeError> {
            Ok(ReviewSideMetadata::Present { size_bytes: 6 })
        }
        fn review_staged_bytes(&self, _file_path: &str) -> Result<ReviewSideBytes, CodeError> {
            Ok(ReviewSideBytes::Present(b"staged".to_vec()))
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
                index: 0,
                hunk_id: String::new(),
                old_start: 1,
                old_lines: 1,
                new_start: 1,
                new_lines: 1,
                lines: vec!["-a".to_string(), "+b".to_string()],
            }])
        }
    }

    struct FakeBranchDiff;
    impl BranchDiffQuery for FakeBranchDiff {
        fn summary(
            &self,
            _repo_path: &str,
            base_name: Option<&str>,
            _base_commit_oid: Option<&str>,
        ) -> Result<BranchDiffSummaryDto, CodeError> {
            Ok(BranchDiffSummaryDto {
                base_branch: base_name.unwrap_or("main").to_string(),
                changed_files: vec![],
                stats: crate::usecase::code_dto::DiffStatsDto {
                    additions: 0,
                    deletions: 0,
                },
            })
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
            Arc::new(FakeBranchDiff),
            Arc::new(FakeBranchBase),
        )
    }

    #[test]
    fn test_差分算出_停止を全read_modelの失敗として返す() {
        use crate::common::operation_context::OperationStopped;

        struct StoppedDiff(OperationStopped);
        impl DiffComputer for StoppedDiff {
            fn diff_buffers(
                &self,
                _: &str,
                _: &str,
                _: Option<&str>,
            ) -> Result<Vec<Hunk>, CodeError> {
                Err(self.0.into())
            }
        }
        for stopped in [OperationStopped::Expired, OperationStopped::Cancelled] {
            let mut service = service();
            service.diff_computer = Arc::new(StoppedDiff(stopped));
            let errors = [
                service.compute_diff_hunks("a", "b", None).unwrap_err(),
                service
                    .compute_hidden_ranges_from_content("a", "b", 3)
                    .unwrap_err(),
                service
                    .compute_visible_markdown_blocks("a", "b", 3)
                    .unwrap_err(),
                service
                    .compute_markdown_diff_ranges("a", "b", DiffSide::Modified)
                    .unwrap_err(),
                service.compute_markdown_split_rows("a", "b").unwrap_err(),
                service
                    .compute_markdown_inline_chunks("a", "b")
                    .unwrap_err(),
            ];
            for error in errors {
                assert!(
                    matches!(error, CodeUsecaseError::Code(CodeError::Technical(actual)) if actual == stopped.into())
                );
            }
        }
    }

    #[test]
    fn test_diff_hunks算出はchange_groupを付与する() {
        let s = service();
        // FakeDiffComputer が 1 hunk（-a/+b）を返す → change group が 1 件算出される。
        let result = s.compute_diff_hunks("a\n", "b\n", None).unwrap();
        assert_eq!(result.hunks.len(), 1);
        assert_eq!(result.change_groups.len(), 1);
        assert_eq!(result.change_groups[0].hunk_index, 0);
    }

    #[test]
    fn test_markdown_diff_read_modelを算出する() {
        let s = service();

        let ranges = s
            .compute_markdown_diff_ranges("a\n", "b\n", DiffSide::Modified)
            .unwrap();
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0].start_line, 1);

        let rows = s.compute_markdown_split_rows("a\n", "b\n").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].left.as_deref(), Some("a\n"));
        assert_eq!(rows[0].right.as_deref(), Some("b\n"));

        let chunks = s.compute_markdown_inline_chunks("a\n", "b\n").unwrap();
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].content, "a\n");
        assert_eq!(chunks[1].content, "b\n");
    }

    #[test]
    fn test_branch_diffサマリを委譲する() {
        let s = service();
        let summary = s.get_branch_diff_summary("/repo", Some("develop")).unwrap();
        assert_eq!(summary.base_branch, "develop");
    }

    #[test]
    fn test_branch_diff_base未指定は現在ブランチbaseを補完する() {
        // Thread 6: base_branch=None の通常経路でも resolver が解決した base 名
        // （FakeBranchBase → "main"）を補完して gateway へ渡す。HEAD フォールバックに
        // 倒れないことを担保する。
        let s = service();
        let summary = s.get_branch_diff_summary("/repo", None).unwrap();
        assert_eq!(summary.base_branch, "main");
    }

    #[test]
    fn test_language判定を委譲する() {
        let s = service();
        assert_eq!(s.get_language_from_path("main.rs"), "rust");
    }
}
