use super::*;
use crate::domain::code::value_objects::file_list::{NameQuery, WalkStep, WorktreeEntry};
use crate::domain::code::{
    BranchBaseResolver, CodeError, DiffComputer, FileContentRepository, Hunk,
};
use crate::usecase::directory_query::DirectoryQuery;
use crate::usecase::repository_dto::FileStatusDto;

struct Dependencies;
impl StagingRepository for Dependencies {
    fn stage(&self, _repo_path: &str, _paths: Vec<String>) -> Result<(), CodeError> {
        unreachable!()
    }
    fn unstage(&self, _repo_path: &str, _paths: Vec<String>) -> Result<(), CodeError> {
        unreachable!()
    }
}
impl FileContentRepository for Dependencies {
    fn review_file_metadata_at_ref(
        &self,
        _file_path: &str,
        _git_ref: &str,
    ) -> Result<ReviewSideMetadata, CodeError> {
        unreachable!()
    }
    fn review_file_bytes_at_ref(
        &self,
        _file_path: &str,
        _git_ref: &str,
    ) -> Result<ReviewSideBytes, CodeError> {
        unreachable!()
    }
    fn review_working_tree_metadata(
        &self,
        _file_path: &str,
    ) -> Result<ReviewSideMetadata, CodeError> {
        unreachable!()
    }
    fn review_working_tree_bytes(&self, _file_path: &str) -> Result<ReviewSideBytes, CodeError> {
        unreachable!()
    }
    fn review_binary_by_attributes(&self, _file_path: &str) -> Result<bool, CodeError> {
        unreachable!()
    }
    fn review_working_tree_matches_ref(
        &self,
        _file_path: &str,
        _git_ref: &str,
    ) -> Result<bool, CodeError> {
        unreachable!()
    }
}
impl DiffComputer for Dependencies {
    fn diff_buffers(
        &self,
        _original: &str,
        _modified: &str,
        _file_path: Option<&str>,
    ) -> Result<Vec<Hunk>, CodeError> {
        unreachable!()
    }
}
impl BranchBaseResolver for Dependencies {
    fn resolve_base_branch_name(&self, _path_hint: &str) -> Result<Option<String>, CodeError> {
        unreachable!()
    }
    fn resolve_base_commit_oid(
        &self,
        _path_hint: &str,
        _base_name: &str,
    ) -> Result<Option<String>, CodeError> {
        unreachable!()
    }
}

struct Directory {
    fail: bool,
}
impl DirectoryQuery for Directory {
    fn file_target(&self, _: &str, _: &str) -> Result<String, CodeUsecaseError> {
        unreachable!()
    }
}
impl WorktreeEntries for Directory {
    fn entries(&self, _: &str, _: &str) -> Result<Vec<WorktreeEntry>, CodeError> {
        unreachable!()
    }
    fn walk(
        &self,
        worktree: &str,
        visit: &mut dyn FnMut(WorktreeEntry) -> WalkStep,
    ) -> Result<(), CodeError> {
        assert_eq!(worktree, "/repo");
        if self.fail {
            return Err(CodeError::Rule("walk failed".into()));
        }
        for path in ["keep.rs", "skip.rs"] {
            assert_eq!(
                visit(WorktreeEntry {
                    path: path.into(),
                    name: path.into(),
                    directory: false,
                    ignored: false
                }),
                WalkStep::Continue
            );
        }
        Ok(())
    }
}

fn usecase(fail: bool) -> CodeUsecase {
    let dependencies = Arc::new(Dependencies);
    CodeUsecase::new(
        dependencies.clone(),
        CodeQueryService::new(dependencies.clone(), dependencies.clone(), dependencies),
        Arc::new(Directory { fail }),
        Arc::new(Directory { fail }),
    )
}

#[test]
fn test_絞込一覧_列挙結果へ条件とgit状態を適用する() {
    // Given
    let usecase = usecase(false);
    let filter = FileFilter::NameAndChanged(NameQuery::new("keep").unwrap());
    let statuses = vec![FileStatusDto {
        path: "keep.rs".into(),
        index_status: "new".into(),
        worktree_status: "modified".into(),
    }];
    // When
    let result = usecase.filtered_files("/repo", &filter, &statuses).unwrap();
    // Then
    assert!(!result.truncated);
    assert_eq!(result.roots.len(), 1);
    assert_eq!(result.roots[0].entry.path, "keep.rs");
    assert_eq!(result.roots[0].status, GitStatus::Modified);
}

#[test]
fn test_絞込一覧_列挙失敗を返す() {
    // Given / When
    let error = usecase(true)
        .filtered_files("/repo", &FileFilter::Changed, &[])
        .unwrap_err();
    // Then
    assert!(
        matches!(error, CodeUsecaseError::Code(CodeError::Rule(message)) if message == "walk failed")
    );
}

#[test]
fn test_絞込一覧_不正なgit状態では列挙せず失敗を返す() {
    // Given / When / Then
    for (index, working) in [("invalid", "none"), ("none", "invalid")] {
        let statuses = vec![FileStatusDto {
            path: "a".into(),
            index_status: index.into(),
            worktree_status: working.into(),
        }];
        let error = usecase(false)
            .filtered_files("must not walk", &FileFilter::Changed, &statuses)
            .unwrap_err();
        assert!(matches!(error, CodeUsecaseError::Code(CodeError::Rule(_))));
    }
}
