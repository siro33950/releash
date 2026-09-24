use super::*;
use crate::domain::git_host::PrStatus;
use crate::usecase::repository_dto::BranchCardDto;
use crate::usecase::workspace_tree::{
    WorkspaceListQueryService, WorkspaceListUsecase, WorkspaceListUsecaseError,
};
use parking_lot::Mutex;
use tauri::Manager;

#[derive(Default)]
struct Query {
    calls: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl WorkspaceListQueryService for Query {
    fn repositories(&self) -> Result<Vec<String>, WorkspaceListUsecaseError> {
        self.calls.lock().push("repositories".into());
        Ok(vec!["/a".into(), "/b".into()])
    }

    async fn branches(&self, path: &str) -> Result<Vec<BranchCardDto>, WorkspaceListUsecaseError> {
        self.calls.lock().push(format!("branches:{path}"));
        Ok(vec![BranchCardDto {
            name: "main".into(),
            is_deleting: false,
            is_main_worktree: true,
            worktree_path: Some(path.into()),
            dirty_count: 0,
            is_merged: false,
            ahead: 0,
            behind: 0,
            has_upstream: false,
            base_ahead: 0,
        }])
    }

    fn pr_status(&self, _: &str) -> Result<PrStatus, WorkspaceListUsecaseError> {
        Ok(PrStatus::default())
    }

    async fn nodes(
        &self,
        path: &str,
    ) -> Result<WorkspaceTreeSnapshotDto, WorkspaceListUsecaseError> {
        self.calls.lock().push(format!("nodes:{path}"));
        Ok(WorkspaceTreeSnapshotDto {
            nodes: vec![],
            archived_sessions: vec![],
            preferred_node_id: None,
        })
    }

    async fn history(
        &self,
        path: &str,
    ) -> Result<Vec<WorkspaceWorkflowHistoryItemDto>, WorkspaceListUsecaseError> {
        self.calls.lock().push(format!("history:{path}"));
        Ok(vec![])
    }
}

#[tokio::test]
async fn test_一覧更新dispatch_省略時は全体を指定時は対象worktreeだけ取得する() {
    // Given
    let (app, _data, _store) =
        crate::adaptor::controller::client::workflow::tests::make_read_only_app();
    app.manage(Arc::new(
        crate::infrastructure::file_watcher::FileWatcherManager::default(),
    ));
    let mut deps = crate::desktop_test_support::build_client_dependencies(app.handle());
    let query = Arc::new(Query::default());
    deps.app_state.as_mut().unwrap().workspace_list =
        Arc::new(WorkspaceListUsecase::new(query.clone()));
    let mut dispatch = ClientCommandDispatch::new(
        deps.app_state.as_ref().unwrap().repository_usecase.clone(),
        Arc::new(crate::usecase::application_startup::ApplicationStartupAuthority::ready()),
    );
    register_shared(&mut dispatch, &deps);
    // When
    let result = dispatch
        .dispatch(wire::command_request::Command::RefreshWorkspaces(
            wire::RefreshWorkspacesRequest {
                repo_path: None,
                worktree_path: None,
            },
        ))
        .await
        .unwrap();
    // Then
    assert!(matches!(
        result,
        wire::command_result::Command::RefreshWorkspaces(_)
    ));
    let mut calls = query.calls.lock().clone();
    assert_eq!(calls.remove(0), "repositories");
    calls.sort();
    assert_eq!(
        calls,
        vec![
            "branches:/a",
            "branches:/b",
            "history:/a",
            "history:/b",
            "nodes:/a",
            "nodes:/b"
        ]
    );
    query.calls.lock().clear();
    // When
    let result = dispatch
        .dispatch(wire::command_request::Command::RefreshWorkspaces(
            wire::RefreshWorkspacesRequest {
                repo_path: None,
                worktree_path: Some("/b".into()),
            },
        ))
        .await
        .unwrap();
    // Then
    assert!(matches!(
        result,
        wire::command_result::Command::RefreshWorkspaces(_)
    ));
    assert_eq!(*query.calls.lock(), vec!["nodes:/b", "history:/b"]);
    query.calls.lock().clear();
    // When
    dispatch
        .dispatch(wire::command_request::Command::RefreshWorkspaces(
            wire::RefreshWorkspacesRequest {
                repo_path: Some("/a".into()),
                worktree_path: None,
            },
        ))
        .await
        .unwrap();
    // Then
    assert_eq!(
        *query.calls.lock(),
        vec!["branches:/a", "nodes:/a", "history:/a"]
    );
}
