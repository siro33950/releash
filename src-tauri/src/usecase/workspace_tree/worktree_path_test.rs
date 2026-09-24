use super::*;

struct Query;

#[async_trait::async_trait]
impl WorkspaceWorktreePathQuery for Query {
    async fn workspace_worktree_path(&self, path: &str) -> Result<String, WorkflowError> {
        assert_eq!(path, "/isolated");
        Ok("/workspace".into())
    }
}

#[tokio::test]
async fn test_workspace解決_共有queryの結果を返す() {
    // Given
    let usecase = WorkspaceWorktreePathUsecase::new(Arc::new(Query));
    // When / Then
    assert_eq!(
        usecase.workspace_worktree_path("/isolated").await.unwrap(),
        "/workspace"
    );
}
