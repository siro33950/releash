use super::*;
use crate::domain::workflow::{ExecutionTreeArchiveCandidate, WorkflowError};
use std::sync::Mutex;

struct Trees {
    candidates: Vec<ExecutionTreeArchiveCandidate>,
    pages: Mutex<Vec<Option<String>>>,
    archives: Mutex<Vec<String>>,
    owners: Mutex<Vec<(String, String)>>,
}

#[async_trait::async_trait]
impl ExecutionTreeGc for Trees {
    async fn record_repository_root(&self, id: &str, root: &str) -> Result<(), WorkflowError> {
        self.owners.lock().unwrap().push((id.into(), root.into()));
        if id == "6-record-fail" {
            return Err(WorkflowError::external("repository record failed"));
        }
        Ok(())
    }
    async fn execution_trees(
        &self,
        after: Option<&str>,
    ) -> Result<Vec<ExecutionTreeArchiveCandidate>, WorkflowError> {
        self.pages.lock().unwrap().push(after.map(str::to_string));
        Ok(self
            .candidates
            .iter()
            .filter(|tree| after.is_none_or(|id| tree.execution_id.as_str() > id))
            .take(2)
            .cloned()
            .collect())
    }
    async fn archive_removed_tree(&self, id: &str) -> Result<(), WorkflowError> {
        self.archives.lock().unwrap().push(id.into());
        if id == "2-fail" {
            return Err(WorkflowError::external("archive failed"));
        }
        Ok(())
    }
}

#[tokio::test]
async fn test_gc実行木archive_ページを反復し所属repoだけを判定して失敗後も続ける() {
    // Given
    let trees = Trees {
        candidates: [
            ("1-live", "/live", Some("/repo")),
            ("2-fail", "/gone-fail", Some("/repo")),
            ("3-unreadable", "/gone-unreadable", Some("/unreadable")),
            ("4-removed", "/gone", Some("/repo")),
            ("5-legacy", "/repo-worktrees/gone", None),
            ("6-record-fail", "/gone-owner", Some("/repo")),
            ("7-linked", "/arbitrary-linked", None),
            ("8-removed", "/gone-next", Some("/repo")),
        ]
        .into_iter()
        .map(|(id, path, repo)| ExecutionTreeArchiveCandidate {
            execution_id: id.into(),
            worktree_path: path.into(),
            workspace_identity: path.into(),
            repository_root: repo.map(str::to_string),
        })
        .collect(),
        pages: Mutex::default(),
        archives: Mutex::default(),
        owners: Mutex::default(),
    };
    let resolution = LiveWorktreeResolution::new(
        LiveWorktreeSet::from_worktrees(["/live", "/arbitrary-linked"].map(|path| LiveWorktree {
            path: path.into(),
            workspace_state_keys: vec![],
            review_comment_keys: vec![],
        })),
        vec!["/unreadable".into()],
        HashSet::new(),
    )
    .with_repository_paths(vec!["/repo".into(), "/unreadable".into()])
    .with_worktree_repositories(HashMap::from([(
        "/arbitrary-linked".into(),
        "/repo".into(),
    )]));
    // When
    let errors = archive_removed_execution_trees(Some(&resolution), &trees)
        .await
        .unwrap();
    // Then
    assert_eq!(errors, 2);
    assert_eq!(
        *trees.archives.lock().unwrap(),
        ["2-fail", "4-removed", "5-legacy", "8-removed"]
    );
    assert_eq!(
        *trees.pages.lock().unwrap(),
        [
            None,
            Some("2-fail".into()),
            Some("4-removed".into()),
            Some("6-record-fail".into()),
            Some("8-removed".into())
        ]
    );
    assert_eq!(trees.owners.lock().unwrap().len(), 8);
    assert!(trees
        .owners
        .lock()
        .unwrap()
        .contains(&("7-linked".into(), "/repo".into())));
    let calls = trees.pages.lock().unwrap().len();
    assert_eq!(
        archive_removed_execution_trees(None, &trees).await.unwrap(),
        0
    );
    assert_eq!(trees.pages.lock().unwrap().len(), calls);
}
