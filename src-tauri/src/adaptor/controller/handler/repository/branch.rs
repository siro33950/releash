//! branch 責務の WebSocket ハンドラ（薄い入口）。

use std::sync::Arc;

use tokio::sync::Mutex;

use crate::adaptor::controller::handler::shared::{join_error_msg, no_worktree_selected_error};
use crate::protocol::*;
use crate::usecase::repository_usecase::RepositoryUsecase;

/// 選択中 worktree のパスを解決し、ブロッキング処理を `spawn_blocking` で実行する。
/// worktree 未選択時は `NO_WORKTREE_SELECTED` エラーを返す。
pub(crate) async fn with_worktree_blocking<F>(
    selected_worktree: &Arc<Mutex<Option<String>>>,
    f: F,
) -> Option<WsMessage>
where
    F: FnOnce(String) -> WsMessage + Send + 'static,
{
    let repo_path = {
        let wt = selected_worktree.lock().await;
        match wt.as_ref() {
            Some(p) => p.clone(),
            None => return Some(no_worktree_selected_error()),
        }
    };
    match tokio::task::spawn_blocking(move || f(repo_path)).await {
        Ok(msg) => Some(msg),
        Err(e) => Some(join_error_msg(e)),
    }
}

/// 選択中 worktree の現在ブランチを返す。
pub(crate) async fn handle_branch_info_request(
    usecase: &Arc<RepositoryUsecase>,
    selected_worktree: &Arc<Mutex<Option<String>>>,
) -> Option<WsMessage> {
    let usecase = Arc::clone(usecase);
    with_worktree_blocking(selected_worktree, move |repo_path| {
        let branch = usecase.get_current_branch(&repo_path).unwrap_or_default();
        WsMessage::BranchInfoResponse(BranchInfoResponse { branch })
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptor::controller::wiring::build_repository_usecase;
    use crate::git::test_helpers::{add_and_commit, create_initial_commit, create_test_repo};
    use tempfile::TempDir;

    fn make_selected(path: Option<String>) -> Arc<Mutex<Option<String>>> {
        Arc::new(Mutex::new(path))
    }

    fn setup_repo_with_file(name: &str, content: &str) -> (TempDir, String) {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, name, content, &format!("add {name}"));
        let repo_path = dir
            .path()
            .canonicalize()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        (dir, repo_path)
    }

    #[tokio::test]
    async fn test_with_worktree_blocking_none() {
        let selected = make_selected(None);
        let result = with_worktree_blocking(&selected, |_| {
            WsMessage::Error(ErrorMsg {
                code: "SHOULD_NOT_REACH".to_string(),
                message: String::new(),
            })
        })
        .await;
        let msg = result.unwrap();
        match msg {
            WsMessage::Error(e) => assert_eq!(e.code, "NO_WORKTREE_SELECTED"),
            _ => panic!("Expected NO_WORKTREE_SELECTED error"),
        }
    }

    #[tokio::test]
    async fn test_with_worktree_blocking_some() {
        let selected = make_selected(Some("/test/repo".to_string()));
        let result = with_worktree_blocking(&selected, |path| {
            WsMessage::BranchInfoResponse(BranchInfoResponse { branch: path })
        })
        .await;
        let msg = result.unwrap();
        match msg {
            WsMessage::BranchInfoResponse(r) => assert_eq!(r.branch, "/test/repo"),
            _ => panic!("Expected BranchInfoResponse"),
        }
    }

    #[tokio::test]
    async fn test_handle_branch_info_request_no_worktree() {
        let usecase = Arc::new(build_repository_usecase());
        let selected = make_selected(None);
        let result = handle_branch_info_request(&usecase, &selected).await;
        let msg = result.unwrap();
        match msg {
            WsMessage::Error(e) => assert_eq!(e.code, "NO_WORKTREE_SELECTED"),
            _ => panic!("Expected NO_WORKTREE_SELECTED"),
        }
    }

    #[tokio::test]
    async fn test_handle_branch_info_request_with_repo() {
        let usecase = Arc::new(build_repository_usecase());
        let (_dir, repo_path) = setup_repo_with_file("file.txt", "content");
        let selected = make_selected(Some(repo_path));
        let result = handle_branch_info_request(&usecase, &selected).await;
        let msg = result.unwrap();
        match msg {
            WsMessage::BranchInfoResponse(r) => {
                assert!(!r.branch.is_empty());
            }
            _ => panic!("Expected BranchInfoResponse"),
        }
    }
}
