use super::*;
use crate::domain::code::{BranchBaseResolver, CodeError, DiffComputer, FileContentRepository};
use crate::usecase::code_query_service::BranchDiffQuery;
use std::sync::Mutex;

struct RecordingStaging {
    calls: Mutex<Vec<String>>,
}
#[async_trait::async_trait]
impl StagingRepository for RecordingStaging {
    fn stage(&self, repo_path: &str, paths: Vec<String>) -> Result<(), CodeError> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("stage:{repo_path}:{}", paths.join(",")));
        Ok(())
    }
    fn unstage(&self, repo_path: &str, _paths: Vec<String>) -> Result<(), CodeError> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("unstage:{repo_path}"));
        Ok(())
    }
    async fn stage_hunk(&self, repo_path: &str, _patch: &str) -> Result<(), CodeError> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("stage_hunk:{repo_path}"));
        Ok(())
    }
    async fn unstage_hunk(&self, repo_path: &str, _patch: &str) -> Result<(), CodeError> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("unstage_hunk:{repo_path}"));
        Ok(())
    }
}

#[derive(Default)]
struct RecordingFileContent {
    calls: Mutex<Vec<String>>,
}

impl RecordingFileContent {
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn record(&self, call: String) {
        self.calls.lock().unwrap().push(call);
    }
}

impl FileContentRepository for RecordingFileContent {
    fn review_file_metadata_at_ref(
        &self,
        f: &str,
        r: &str,
    ) -> Result<ReviewSideMetadata, CodeError> {
        self.record(format!("metadata:ref:{r}:{f}"));
        Ok(ReviewSideMetadata::Present { size_bytes: 1 })
    }
    fn review_file_bytes_at_ref(&self, f: &str, r: &str) -> Result<ReviewSideBytes, CodeError> {
        self.record(format!("bytes:ref:{r}:{f}"));
        Ok(ReviewSideBytes::Present(b"head".to_vec()))
    }
    fn review_file_metadata_at_branch_base(
        &self,
        f: &str,
        b: Option<&str>,
    ) -> Result<ReviewSideMetadata, CodeError> {
        self.record(format!("metadata:branch-base:{}:{f}", b.unwrap_or("none")));
        Ok(ReviewSideMetadata::Present { size_bytes: 1 })
    }
    fn review_file_bytes_at_branch_base(
        &self,
        f: &str,
        b: Option<&str>,
    ) -> Result<ReviewSideBytes, CodeError> {
        self.record(format!("bytes:branch-base:{}:{f}", b.unwrap_or("none")));
        Ok(ReviewSideBytes::Present(b"base".to_vec()))
    }
    fn review_staged_metadata(&self, f: &str) -> Result<ReviewSideMetadata, CodeError> {
        self.record(format!("metadata:staged:{f}"));
        Ok(ReviewSideMetadata::Present { size_bytes: 1 })
    }
    fn review_staged_bytes(&self, f: &str) -> Result<ReviewSideBytes, CodeError> {
        self.record(format!("bytes:staged:{f}"));
        Ok(ReviewSideBytes::Present(b"staged".to_vec()))
    }
    fn review_working_tree_metadata(&self, f: &str) -> Result<ReviewSideMetadata, CodeError> {
        self.record(format!("metadata:working-tree:{f}"));
        Ok(ReviewSideMetadata::Present { size_bytes: 1 })
    }
    fn review_working_tree_bytes(&self, f: &str) -> Result<ReviewSideBytes, CodeError> {
        self.record(format!("bytes:working-tree:{f}"));
        Ok(ReviewSideBytes::Present(b"working".to_vec()))
    }
    fn review_binary_by_attributes(&self, _f: &str) -> Result<bool, CodeError> {
        Ok(false)
    }
}

struct StubDiffComputer;
impl DiffComputer for StubDiffComputer {
    fn diff_buffers(&self, _o: &str, _m: &str, _f: Option<&str>) -> Result<Vec<Hunk>, CodeError> {
        Ok(Vec::new())
    }
}

struct StubBranchDiff;
impl BranchDiffQuery for StubBranchDiff {
    fn summary(
        &self,
        _repo_path: &str,
        _base_name: Option<&str>,
        _base_commit_oid: Option<&str>,
    ) -> Result<BranchDiffSummaryDto, CodeError> {
        Ok(BranchDiffSummaryDto {
            base_branch: String::new(),
            changed_files: vec![],
            stats: crate::usecase::code_dto::DiffStatsDto {
                additions: 0,
                deletions: 0,
            },
        })
    }
}

struct StubBranchBase;
impl BranchBaseResolver for StubBranchBase {
    fn resolve_base_branch_name(&self, _p: &str) -> Result<Option<String>, CodeError> {
        Ok(None)
    }
    fn resolve_base_commit_oid(
        &self,
        _path_hint: &str,
        _base_name: &str,
    ) -> Result<Option<String>, CodeError> {
        Ok(None)
    }
}

fn usecase(staging: Arc<RecordingStaging>) -> CodeUsecase {
    let query = CodeQueryService::new(
        Arc::new(RecordingFileContent::default()),
        Arc::new(StubDiffComputer),
        Arc::new(StubBranchDiff),
        Arc::new(StubBranchBase),
    );
    CodeUsecase::new(staging, query)
}

fn usecase_with_file_content(file_content: Arc<dyn FileContentRepository>) -> CodeUsecase {
    let query = CodeQueryService::new(
        file_content,
        Arc::new(StubDiffComputer),
        Arc::new(StubBranchDiff),
        Arc::new(StubBranchBase),
    );
    CodeUsecase::new(
        Arc::new(RecordingStaging {
            calls: Mutex::new(Vec::new()),
        }),
        query,
    )
}

#[tokio::test]
async fn test_stageはstagingリポジトリへ委譲する() {
    let staging = Arc::new(RecordingStaging {
        calls: Mutex::new(Vec::new()),
    });
    let uc = usecase(staging.clone());

    uc.git_stage("/repo", vec!["a.rs".to_string()]).unwrap();
    uc.git_unstage_hunk("/repo", "patch").await.unwrap();

    let calls = staging.calls.lock().unwrap();
    assert_eq!(calls[0], "stage:/repo:a.rs");
    assert_eq!(calls[1], "unstage_hunk:/repo");
}

#[tokio::test]
async fn review_side_source_mapping_uses_production_metadata_and_bytes_methods() {
    let file_content = Arc::new(RecordingFileContent::default());
    let uc = usecase_with_file_content(file_content.clone());
    let file_path = "/repo/file.txt";
    let cases = [
        (
            ReviewSection::Changes,
            ReviewBase::Head,
            ReviewBlobSide::Original,
            ReviewContentSource::Staged,
            format!("metadata:staged:{file_path}"),
            format!("bytes:staged:{file_path}"),
        ),
        (
            ReviewSection::Changes,
            ReviewBase::Head,
            ReviewBlobSide::Modified,
            ReviewContentSource::WorkingTree,
            format!("metadata:working-tree:{file_path}"),
            format!("bytes:working-tree:{file_path}"),
        ),
        (
            ReviewSection::Staged,
            ReviewBase::Head,
            ReviewBlobSide::Original,
            ReviewContentSource::Head,
            format!("metadata:ref:HEAD:{file_path}"),
            format!("bytes:ref:HEAD:{file_path}"),
        ),
        (
            ReviewSection::Staged,
            ReviewBase::Head,
            ReviewBlobSide::Modified,
            ReviewContentSource::Staged,
            format!("metadata:staged:{file_path}"),
            format!("bytes:staged:{file_path}"),
        ),
        (
            ReviewSection::Changes,
            ReviewBase::BranchBase,
            ReviewBlobSide::Original,
            ReviewContentSource::BranchBase,
            format!("metadata:branch-base:none:{file_path}"),
            format!("bytes:branch-base:none:{file_path}"),
        ),
        (
            ReviewSection::Changes,
            ReviewBase::BranchBase,
            ReviewBlobSide::Modified,
            ReviewContentSource::WorkingTree,
            format!("metadata:working-tree:{file_path}"),
            format!("bytes:working-tree:{file_path}"),
        ),
    ];

    let mut expected_calls = Vec::new();
    for (section, base, side, expected_source, metadata_call, bytes_call) in cases {
        let selected = uc
            .select_review_side_source(file_path, side, section, base)
            .unwrap();
        assert_eq!(selected.source, expected_source);
        uc.read_review_source_bytes(file_path, selected.source)
            .unwrap();
        expected_calls.push(metadata_call);
        expected_calls.push(bytes_call);
    }

    assert_eq!(file_content.calls(), expected_calls);
}
