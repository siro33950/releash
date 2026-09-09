use super::*;

struct Query;

impl WorkspaceWorktreePathQuery for Query {
    fn workspace_worktree_path(&self, path: &str) -> Result<String, WorkflowError> {
        assert_eq!(path, "/isolated");
        Ok("/workspace".into())
    }
}

#[test]
fn test_workspace解決_共有queryの結果を返す() {
    // Given
    let usecase = WorkspaceWorktreePathUsecase::new(Arc::new(Query));
    // When / Then
    assert_eq!(
        usecase.workspace_worktree_path("/isolated").unwrap(),
        "/workspace"
    );
}
