mod tests {
    use std::collections::{HashMap, VecDeque};
    use std::sync::Mutex;

    use super::super::*;

    struct FakeSnapshotProvider {
        snapshots: Mutex<VecDeque<Arc<RepositorySnapshot>>>,
    }

    impl FakeSnapshotProvider {
        fn new(snapshots: Vec<RepositorySnapshot>) -> Self {
            Self {
                snapshots: Mutex::new(snapshots.into_iter().map(Arc::new).collect()),
            }
        }
    }

    impl ReviewSnapshotProvider for FakeSnapshotProvider {
        fn snapshot(
            &self,
            _worktree_path: &str,
        ) -> Result<Arc<RepositorySnapshot>, CodeUsecaseError> {
            self.snapshots.lock().unwrap().pop_front().ok_or_else(|| {
                CodeUsecaseError::from(CodeError::Rule("snapshot queue exhausted".to_string()))
            })
        }
    }

    struct FakeReviewCode {
        calls: Mutex<Vec<String>>,
        reads: Mutex<Vec<String>>,
        source_metadata: HashMap<(String, &'static str), ReviewSideMetadata>,
        source_bytes: HashMap<(String, &'static str), ReviewSideBytes>,
        source_byte_sequences: Mutex<HashMap<(String, &'static str), VecDeque<ReviewSideBytes>>>,
        binary_attributes: HashMap<String, bool>,
        hunk_count_by_path: HashMap<String, usize>,
        diff_results: HashMap<(String, String), DiffHunksResultDto>,
    }

    impl FakeReviewCode {
        fn new() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                reads: Mutex::new(Vec::new()),
                source_metadata: HashMap::new(),
                source_bytes: HashMap::new(),
                source_byte_sequences: Mutex::new(HashMap::new()),
                binary_attributes: HashMap::new(),
                hunk_count_by_path: HashMap::new(),
                diff_results: HashMap::new(),
            }
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }

        fn with_source_bytes(
            mut self,
            file_path: &str,
            source: ReviewContentSource,
            bytes: ReviewSideBytes,
        ) -> Self {
            self.source_bytes
                .insert((file_path.to_string(), review_source_key(source)), bytes);
            self
        }

        fn with_source_metadata(
            mut self,
            file_path: &str,
            source: ReviewContentSource,
            metadata: ReviewSideMetadata,
        ) -> Self {
            self.source_metadata
                .insert((file_path.to_string(), review_source_key(source)), metadata);
            self
        }

        fn with_binary_attribute(mut self, file_path: &str, binary: bool) -> Self {
            self.binary_attributes.insert(file_path.to_string(), binary);
            self
        }

        fn with_hunk_count(mut self, file_path: &str, hunk_count: usize) -> Self {
            self.hunk_count_by_path
                .insert(file_path.to_string(), hunk_count);
            self
        }

        fn with_diff_result(
            mut self,
            original: &str,
            modified: &str,
            result: DiffHunksResultDto,
        ) -> Self {
            self.diff_results
                .insert((original.into(), modified.into()), result);
            self
        }

        fn metadata_for(&self, file_path: &str, source: ReviewContentSource) -> ReviewSideMetadata {
            let key = (file_path.to_string(), review_source_key(source));
            if let Some(bytes) = self
                .source_byte_sequences
                .lock()
                .unwrap()
                .get(&key)
                .and_then(|sequence| sequence.front())
            {
                return match bytes {
                    ReviewSideBytes::Present(bytes) => ReviewSideMetadata::Present {
                        size_bytes: bytes.len() as u64,
                    },
                    ReviewSideBytes::Missing => ReviewSideMetadata::Missing,
                };
            }
            self.source_metadata.get(&key).copied().unwrap_or_else(|| {
                match self.source_bytes.get(&key) {
                    Some(ReviewSideBytes::Present(bytes)) => ReviewSideMetadata::Present {
                        size_bytes: bytes.len() as u64,
                    },
                    _ => ReviewSideMetadata::Missing,
                }
            })
        }
    }

    #[async_trait::async_trait]

    impl ReviewCodePort for FakeReviewCode {
        fn file_target(&self, worktree: &str, file: &str) -> Result<String, CodeUsecaseError> {
            Ok(format!("{worktree}/{file}"))
        }

        fn directory_entries(
            &self,
            _: &str,
            _: &str,
            _: &[FileStatusDto],
        ) -> Result<Vec<crate::usecase::directory_query::DirectoryEntryDto>, CodeUsecaseError>
        {
            Ok(Vec::new())
        }

        fn select_review_side_source(
            &self,
            file_path: &str,
            side: ReviewBlobSide,
            base: ReviewBase,
        ) -> Result<SelectedReviewSide, CodeUsecaseError> {
            let source = review_content_source_for(side, base);
            Ok(SelectedReviewSide {
                source,
                metadata: self.metadata_for(file_path, source),
            })
        }

        fn read_review_source_bytes(
            &self,
            file_path: &str,
            source: ReviewContentSource,
        ) -> Result<ReviewSideBytes, CodeUsecaseError> {
            self.reads.lock().unwrap().push(file_path.into());
            let key = (file_path.to_string(), review_source_key(source));
            {
                let mut sequences = self.source_byte_sequences.lock().unwrap();
                if let Some(sequence) = sequences.get_mut(&key) {
                    if sequence.len() > 1 {
                        return Ok(sequence.pop_front().unwrap());
                    }
                    if let Some(bytes) = sequence.front() {
                        return Ok(bytes.clone());
                    }
                }
            }
            Ok(self
                .source_bytes
                .get(&key)
                .cloned()
                .unwrap_or(ReviewSideBytes::Missing))
        }

        fn review_working_tree_matches_source(
            &self,
            file_path: &str,
            original: ReviewContentSource,
        ) -> Result<bool, CodeUsecaseError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("matches:{original:?}"));
            Ok(self.metadata_for(file_path, original)
                == self.metadata_for(file_path, ReviewContentSource::WorkingTree)
                && self
                    .source_bytes
                    .get(&(file_path.into(), review_source_key(original)))
                    == self.source_bytes.get(&(
                        file_path.into(),
                        review_source_key(ReviewContentSource::WorkingTree),
                    )))
        }

        fn review_binary_by_attributes(&self, file_path: &str) -> Result<bool, CodeUsecaseError> {
            Ok(self
                .binary_attributes
                .get(file_path)
                .copied()
                .unwrap_or(false))
        }

        fn compute_diff_hunks(
            &self,
            original: &str,
            modified: &str,
            file_path: Option<&str>,
        ) -> Result<DiffHunksResultDto, CodeUsecaseError> {
            let hunk_count = file_path.and_then(|path| self.hunk_count_by_path.get(path).copied());
            if hunk_count.is_none() && !self.diff_results.is_empty() {
                return self
                    .diff_results
                    .get(&(original.into(), modified.into()))
                    .cloned()
                    .ok_or_else(|| CodeError::Rule("unexpected diff input".into()).into());
            }
            let hunk_count = hunk_count.unwrap_or_else(|| usize::from(original != modified));
            Ok(DiffHunksResultDto {
                hunks: (0..hunk_count)
                    .map(|_| HunkDto {
                        old_start: 1,
                        old_lines: 1,
                        new_start: 1,
                        new_lines: 1,
                        lines: vec!["@@".to_string()],
                    })
                    .collect(),
            })
        }
    }

    fn file_status(path: &str, index_status: &str, worktree_status: &str) -> FileStatusDto {
        FileStatusDto {
            path: path.to_string(),
            index_status: index_status.to_string(),
            worktree_status: worktree_status.to_string(),
        }
    }

    fn repository_snapshot(version: u64) -> RepositorySnapshot {
        RepositorySnapshot {
            version,
            status: Vec::new(),
            dirty_count: 0,
        }
    }

    fn snapshot_with_single_status(
        version: u64,
        path: &str,
        index_status: &str,
        worktree_status: &str,
    ) -> RepositorySnapshot {
        RepositorySnapshot {
            version,
            status: vec![file_status(path, index_status, worktree_status)],
            dirty_count: 0,
        }
    }

    fn review_content_source_for(side: ReviewBlobSide, base: ReviewBase) -> ReviewContentSource {
        ReviewContentSource::select(side, base)
    }

    fn review_source_key(source: ReviewContentSource) -> &'static str {
        match source {
            ReviewContentSource::BranchBaseTip => "branch-base-tip",
            ReviewContentSource::Head => "head",
            ReviewContentSource::WorkingTree => "working-tree",
        }
    }

    fn present_text(content: &str) -> ReviewSideBytes {
        ReviewSideBytes::Present(content.as_bytes().to_vec())
    }

    fn present_metadata(size_bytes: u64) -> ReviewSideMetadata {
        ReviewSideMetadata::Present { size_bytes }
    }

    fn usecase_with_code(
        snapshots: Vec<RepositorySnapshot>,
        code: FakeReviewCode,
    ) -> ReviewUsecase {
        ReviewUsecase::new_with_ports(
            Arc::new(FakeSnapshotProvider::new(snapshots)),
            Arc::new(code),
        )
    }

    #[tokio::test]
    async fn review_target_rejects_invalid_and_empty_paths() {
        for raw in ["../secret.txt", "src/../secret.txt", "/etc/passwd", "/"] {
            let err = resolve_review_target("/repo", raw).unwrap_err().to_string();
            assert!(
                err.contains("invalid review target path"),
                "unexpected error for {raw}: {err}"
            );
        }

        let err = resolve_review_target("/repo", "").unwrap_err().to_string();
        assert!(err.contains("empty review target path"));
    }

    #[tokio::test]
    async fn review_blob_mime_mapping_lives_in_usecase() {
        assert_eq!(review_blob_mime_for_path("assets/LOGO.PNG"), "image/png");
        assert_eq!(review_blob_mime_for_path("photo.jpg"), "image/jpeg");
        assert_eq!(review_blob_mime_for_path("photo.jpeg"), "image/jpeg");
        assert_eq!(review_blob_mime_for_path("icons/app.svg"), "image/svg+xml");
        assert_eq!(review_blob_mime_for_path("pic.webp"), "image/webp");
        assert_eq!(
            review_blob_mime_for_path("archive.bin"),
            "application/octet-stream"
        );
    }

    #[test]
    fn test_ファイルタブ_変更一覧外の大きい全文と差分を上限なしで返す() {
        // Given
        let content = "long line\n".repeat(120_000);
        let code = Arc::new(
            FakeReviewCode::new()
                .with_source_bytes(
                    "/repo/file.rs",
                    ReviewContentSource::Head,
                    present_text("old\n"),
                )
                .with_source_bytes(
                    "/repo/file.rs",
                    ReviewContentSource::WorkingTree,
                    present_text(&content),
                )
                .with_hunk_count("file.rs", 301),
        );
        let usecase = ReviewUsecase::new_with_ports(
            Arc::new(FakeSnapshotProvider::new(vec![repository_snapshot(1)])),
            code,
        );
        // When
        let view = usecase
            .get_file_tab_view("/repo", "file.rs", "head")
            .unwrap();
        // Then
        assert_eq!(
            view.initial_mode,
            crate::domain::code::value_objects::review::FileMode::Diff
        );
        assert_eq!(view.patch.matches("\n@@ -").count(), 301);
        let ReviewFileViewDto::TextDiff(text) = view.view else {
            panic!("expected text");
        };
        assert_eq!(text.modified, content);
    }

    #[test]
    fn test_ファイルタブ_変更なしはfileで開きpatchを付けない() {
        // Given
        let code = Arc::new(
            FakeReviewCode::new()
                .with_source_bytes(
                    "/repo/file.rs",
                    ReviewContentSource::BranchBaseTip,
                    present_text("same\n"),
                )
                .with_source_bytes(
                    "/repo/file.rs",
                    ReviewContentSource::WorkingTree,
                    present_text("same\n"),
                ),
        );
        let usecase = ReviewUsecase::new_with_ports(
            Arc::new(FakeSnapshotProvider::new(vec![repository_snapshot(1)])),
            code,
        );
        // When
        let view = usecase
            .get_file_tab_view("/repo", "file.rs", "branch-base")
            .unwrap();
        // Then
        assert_eq!(
            view.initial_mode,
            crate::domain::code::value_objects::review::FileMode::File
        );
        assert!(view.patch.is_empty());
        assert!(matches!(view.view, ReviewFileViewDto::TextDiff(_)));
    }
    #[test]
    fn test_ファイルタブ_indexが非テキストでもheadと作業ツリーの差分を返す() {
        // Given
        let result = DiffHunksResultDto {
            hunks: vec![HunkDto {
                old_start: 1,
                old_lines: 1,
                new_start: 1,
                new_lines: 1,
                lines: vec!["-old".into(), "+new".into()],
            }],
        };
        for _index in [vec![0xff], vec![0]] {
            let code = FakeReviewCode::new()
                .with_source_bytes(
                    "/repo/file.rs",
                    ReviewContentSource::Head,
                    present_text("old\n"),
                )
                .with_source_bytes(
                    "/repo/file.rs",
                    ReviewContentSource::WorkingTree,
                    present_text("new\n"),
                )
                .with_diff_result("old\n", "new\n", result.clone());
            let usecase = usecase_with_code(vec![repository_snapshot(1)], code);
            // When
            let view = usecase
                .get_file_tab_view("/repo", "file.rs", "head")
                .unwrap();
            // Then
            assert_eq!(
                view.initial_mode,
                crate::domain::code::value_objects::review::FileMode::Diff
            );
            assert!(view.patch.contains("-old\n+new\n"));
            let ReviewFileViewDto::TextDiff(text) = view.view else {
                panic!("text diff")
            };
            assert_eq!(text.original, "old\n");
            assert_eq!(text.modified, "new\n");
        }
    }

    #[test]
    fn test_ファイルタブ_画像の変更追加削除と変更なしで存在する側だけを返す() {
        for (original, modified, mode) in [
            (Some(vec![1, 2, 3]), Some(vec![4, 5, 6]), "diff"),
            (None, Some(vec![4, 5, 6]), "diff"),
            (Some(vec![1, 2, 3]), None, "diff"),
            (Some(vec![1, 2, 3]), Some(vec![1, 2, 3]), "file"),
        ] {
            // Given
            let code = FakeReviewCode::new()
                .with_source_bytes(
                    "/repo/image.png",
                    ReviewContentSource::Head,
                    original
                        .clone()
                        .map(ReviewSideBytes::Present)
                        .unwrap_or(ReviewSideBytes::Missing),
                )
                .with_source_bytes(
                    "/repo/image.png",
                    ReviewContentSource::WorkingTree,
                    modified
                        .clone()
                        .map(ReviewSideBytes::Present)
                        .unwrap_or(ReviewSideBytes::Missing),
                )
                .with_binary_attribute("/repo/image.png", true);
            let usecase = usecase_with_code(vec![repository_snapshot(1)], code);
            // When
            let view = usecase
                .get_file_tab_view("/repo", "image.png", "head")
                .unwrap();
            // Then
            assert_eq!(view.initial_mode.as_str(), mode);
            let ReviewFileViewDto::Image(image) = view.view else {
                panic!("image")
            };
            assert_eq!(
                image.original_url,
                original.map(|_| "data:image/png;base64,AQID".into())
            );
            assert_eq!(
                image.modified_url,
                modified.map(|b| if b[0] == 1 {
                    "data:image/png;base64,AQID".into()
                } else {
                    "data:image/png;base64,BAUG".into()
                })
            );
        }
    }

    #[test]
    fn test_ファイルタブ_バイナリ属性とバイト列の分類で本文を返さずサイズを返す() {
        for (bytes, attribute) in [
            (vec![0xff, 1, 2], false),
            (vec![b'a', 0, b'b'], false),
            (b"text".to_vec(), true),
        ] {
            // Given
            let size = bytes.len() as u64;
            let code = FakeReviewCode::new()
                .with_source_bytes(
                    "/repo/file.bin",
                    ReviewContentSource::Head,
                    ReviewSideBytes::Present(if attribute { vec![b'a'] } else { vec![0xff] }),
                )
                .with_source_bytes(
                    "/repo/file.bin",
                    ReviewContentSource::WorkingTree,
                    ReviewSideBytes::Present(bytes),
                )
                .with_binary_attribute("/repo/file.bin", attribute);
            let usecase = usecase_with_code(vec![repository_snapshot(1)], code);
            // When
            let view = usecase
                .get_file_tab_view("/repo", "file.bin", "head")
                .unwrap();
            // Then
            assert!(view.patch.is_empty());
            let ReviewFileViewDto::Binary(binary) = view.view else {
                panic!("binary")
            };
            assert_eq!(binary.original_size, Some(1));
            assert_eq!(binary.modified_size, Some(size));
        }
    }
    #[test]
    fn test_ファイルタブ_属性バイナリは本文を読まずmetadataのサイズを返す() {
        // Given
        let code = Arc::new(
            FakeReviewCode::new()
                .with_source_metadata(
                    "/repo/file.bin",
                    ReviewContentSource::Head,
                    present_metadata(123),
                )
                .with_source_metadata(
                    "/repo/file.bin",
                    ReviewContentSource::WorkingTree,
                    present_metadata(456),
                )
                .with_binary_attribute("/repo/file.bin", true),
        );
        let usecase = ReviewUsecase::new_with_ports(
            Arc::new(FakeSnapshotProvider::new(vec![
                snapshot_with_single_status(1, "file.bin", "none", "modified"),
            ])),
            code.clone(),
        );
        // When
        let view = usecase
            .get_file_tab_view("/repo", "file.bin", "head")
            .unwrap();
        // Then
        assert_eq!(
            view.initial_mode,
            crate::domain::code::value_objects::review::FileMode::Diff
        );
        let ReviewFileViewDto::Binary(binary) = view.view else {
            panic!("binary view")
        };
        assert_eq!(binary.original_size, Some(123));
        assert_eq!(binary.modified_size, Some(456));
        assert!(code.reads.lock().unwrap().is_empty());
    }
    #[test]
    fn test_属性バイナリ初期表示_選択した比較の一致だけで決め本文を取得しない() {
        for (base, original_source, original, modified, expected) in [
            (
                "head",
                ReviewContentSource::Head,
                b"AAA".to_vec(),
                b"AAA".to_vec(),
                crate::domain::code::value_objects::review::FileMode::File,
            ),
            (
                "branch-base",
                ReviewContentSource::BranchBaseTip,
                b"BBB".to_vec(),
                b"AAA".to_vec(),
                crate::domain::code::value_objects::review::FileMode::Diff,
            ),
        ] {
            // Given
            let code = Arc::new(
                FakeReviewCode::new()
                    .with_source_bytes(
                        "/repo/file.bin",
                        original_source,
                        ReviewSideBytes::Present(original),
                    )
                    .with_source_bytes(
                        "/repo/file.bin",
                        ReviewContentSource::WorkingTree,
                        ReviewSideBytes::Present(modified),
                    )
                    .with_binary_attribute("/repo/file.bin", true),
            );
            let usecase = ReviewUsecase::new_with_ports(
                Arc::new(FakeSnapshotProvider::new(vec![
                    snapshot_with_single_status(1, "file.bin", "modified", "modified"),
                ])),
                code.clone(),
            );
            // When
            let view = usecase
                .get_file_tab_view("/repo", "file.bin", base)
                .unwrap();
            // Then
            assert_eq!(view.initial_mode, expected);
            let ReviewFileViewDto::Binary(binary) = view.view else {
                panic!("binary");
            };
            assert_eq!(binary.original_size, Some(3));
            assert_eq!(binary.modified_size, Some(3));
            assert!(code.reads.lock().unwrap().is_empty());
            assert!(code
                .calls()
                .contains(&format!("matches:{original_source:?}")));
        }
    }
}
