//! worktree 責務の WebSocket ハンドラ（薄い入口）。
//!
//! query service で worktree を読み、response メッセージへ整形する。worktree 選択時の
//! broadcaster / pty_manager 連携（トランスポート副作用）は引数で受け取る。

use std::sync::Arc;

use tokio::sync::Mutex;

use crate::git_host::PrCache;
use crate::protocol::*;
use crate::pty::PtyManager;
use crate::usecase::repository_usecase::RepositoryUsecase;
use crate::ws_bridge::WsBroadcaster;

pub(crate) fn no_repo_error() -> WsMessage {
    WsMessage::Error(ErrorMsg {
        code: "NO_REPO".to_string(),
        message: "リポジトリパスが設定されていません".to_string(),
    })
}

/// 設定済み全リポジトリの worktree 一覧（repository ローカル情報のみ）を組み立てる。
///
/// PR ステータス（git_host 由来）の合成は行わない。PR は [`push_worktree_pr_status`] が
/// 後追いで配信する（repository × git_host のクロスドメイン合成を handler から排した
/// 一覧表示 / PR 表示の 2 段階化）。
pub(crate) async fn build_all_worktrees(
    repo_paths: Vec<String>,
    usecase: Arc<RepositoryUsecase>,
) -> Vec<WorktreeEntryMsg> {
    tokio::task::spawn_blocking(move || {
        let mut all_entries = Vec::new();
        for repo_path in &repo_paths {
            let entries = usecase.list_worktrees(repo_path).unwrap_or_default();
            for e in entries {
                all_entries.push(WorktreeEntryMsg {
                    name: e.name,
                    path: e.path,
                    branch: e.branch,
                    is_main: e.is_main,
                    is_locked: e.is_locked,
                    dirty_count: e.dirty_count,
                    base_branch: e.base_branch,
                    repo_path: Some(repo_path.clone()),
                });
            }
        }
        all_entries
    })
    .await
    .unwrap_or_default()
}

/// worktree 一覧を返す（repository ローカル情報のみ）。リポジトリ未設定時は `NO_REPO` エラー。
pub(crate) async fn handle_worktree_list_request(
    repo_paths: Vec<String>,
    usecase: Arc<RepositoryUsecase>,
) -> Option<WsMessage> {
    if repo_paths.is_empty() {
        return Some(no_repo_error());
    }
    let worktrees = build_all_worktrees(repo_paths, usecase).await;
    Some(WsMessage::WorktreeListResponse(WorktreeListResponse {
        worktrees,
    }))
}

/// worktree 一覧返却後に PR ステータスを後追いで配信する。
///
/// PR 取得（git_host）と worktree（repository）の突き合わせ（branch 名マッチング）を
/// Rust 側で行い、PR が存在する worktree のみを `path` 付きで `WorktreePrStatusSync`
/// として broadcaster へ送る。クロスドメイン合成は handler の同期パスから切り離し、
/// 専用の後追いタスクへ閉じる。
pub(crate) fn push_worktree_pr_status(
    repo_paths: Vec<String>,
    pr_cache: Arc<PrCache>,
    usecase: Arc<RepositoryUsecase>,
    broadcaster: Arc<WsBroadcaster>,
) {
    if repo_paths.is_empty() {
        return;
    }
    tokio::task::spawn_blocking(move || {
        let mut entries = Vec::new();
        for repo_path in &repo_paths {
            let pr_status = crate::git_host::fetch_pr_status_with_cache(&pr_cache, repo_path);
            let worktrees = usecase.list_worktrees(repo_path).unwrap_or_default();
            for wt in worktrees {
                if let Some(pr) = pr_status.open_prs.get(&wt.branch) {
                    entries.push(WorktreePrEntry {
                        path: wt.path,
                        pr_number: pr.number,
                        pr_url: pr.url.clone(),
                    });
                }
            }
        }
        broadcaster.try_send(WsMessage::WorktreePrStatusSync(WorktreePrStatusSync {
            entries,
        }));
    });
}

/// worktree を選択する。検証成功時は選択状態を更新し、ブランチ情報・選択結果・PTY 状態を
/// broadcaster へ送る。
pub(crate) async fn handle_worktree_select_request(
    req: &WorktreeSelectRequest,
    repo_paths: Vec<String>,
    usecase: Arc<RepositoryUsecase>,
    broadcaster: &Arc<WsBroadcaster>,
    pty_manager: Option<&Arc<PtyManager>>,
    selected_worktree: &Arc<Mutex<Option<String>>>,
) -> Option<WsMessage> {
    if repo_paths.is_empty() {
        return Some(no_repo_error());
    }
    let requested_path = req.path.clone();
    let broadcaster = broadcaster.clone();

    // valid 判定は is_managed_worktree と同一走査のため共通化する（DRY）。
    let valid = is_managed_worktree(repo_paths, Arc::clone(&usecase), &requested_path).await;

    if !valid {
        return Some(WsMessage::WorktreeSelectResponse(WorktreeSelectResponse {
            success: false,
            path: requested_path,
            error: Some("指定されたworktreeが見つかりません".to_string()),
        }));
    }

    {
        let mut wt = selected_worktree.lock().await;
        *wt = Some(requested_path.clone());
    }

    let wt_path = requested_path.clone();
    let usecase_for_branch = Arc::clone(&usecase);
    if let Ok(branch) = tokio::task::spawn_blocking(move || {
        usecase_for_branch
            .get_current_branch(&wt_path)
            .unwrap_or_default()
    })
    .await
    {
        broadcaster.try_send(WsMessage::BranchInfoResponse(BranchInfoResponse { branch }));
    }

    broadcaster.try_send(WsMessage::WorktreeSelectResponse(WorktreeSelectResponse {
        success: true,
        path: requested_path.clone(),
        error: None,
    }));

    if let Some(pm) = pty_manager {
        for session in pm.list_pty_sessions() {
            if session.worktree_path.as_deref() == Some(&requested_path) {
                let (cols, rows) = pm.get_pty_size(session.pty_id).unwrap_or((80, 24));
                broadcaster.try_send(WsMessage::PtyReady(PtyReady {
                    pty_id: session.pty_id,
                    cols,
                    rows,
                    label: session.label.clone(),
                    worktree_path: session.worktree_path.clone(),
                }));
            }
        }
    }

    None
}

/// 指定パスが設定済みリポジトリの管理下にある worktree か検証する。
pub(crate) async fn is_managed_worktree(
    repo_paths: Vec<String>,
    usecase: Arc<RepositoryUsecase>,
    worktree_path: &str,
) -> bool {
    let requested_path = worktree_path.to_string();
    tokio::task::spawn_blocking(move || {
        for repo_path in &repo_paths {
            let paths = usecase.list_worktree_paths(repo_path).unwrap_or_default();
            if paths.iter().any(|p| p == &requested_path) {
                return true;
            }
        }
        false
    })
    .await
    .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptor::controller::wiring::build_repository_usecase;
    use crate::git::test_helpers::{add_and_commit, create_initial_commit, create_test_repo};
    use tempfile::TempDir;

    fn make_usecase() -> Arc<RepositoryUsecase> {
        Arc::new(build_repository_usecase())
    }

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

    #[test]
    fn test_no_repo_error() {
        let msg = no_repo_error();
        match msg {
            WsMessage::Error(e) => {
                assert_eq!(e.code, "NO_REPO");
                assert!(!e.message.is_empty());
            }
            _ => panic!("Expected Error variant"),
        }
    }

    #[tokio::test]
    async fn test_is_managed_worktree_accepts_known_worktree() {
        let (_dir, repo_path) = setup_repo_with_file("file.txt", "content");
        assert!(is_managed_worktree(vec![repo_path.clone()], make_usecase(), &repo_path).await);
    }

    #[tokio::test]
    async fn test_handle_worktree_list_request_with_repo() {
        let (_dir, repo_path) = setup_repo_with_file("file.txt", "content");
        let result = handle_worktree_list_request(vec![repo_path], make_usecase()).await;
        let msg = result.unwrap();
        match msg {
            WsMessage::WorktreeListResponse(r) => {
                assert!(!r.worktrees.is_empty());
            }
            _ => panic!("Expected WorktreeListResponse"),
        }
    }

    #[tokio::test]
    async fn test_handle_worktree_select_request_invalid_path() {
        let (_dir, repo_path) = setup_repo_with_file("file.txt", "content");
        let broadcaster = Arc::new(WsBroadcaster::default());
        let selected = make_selected(None);
        let req = WorktreeSelectRequest {
            path: "/nonexistent/worktree/path".to_string(),
        };
        let result = handle_worktree_select_request(
            &req,
            vec![repo_path],
            make_usecase(),
            &broadcaster,
            None,
            &selected,
        )
        .await;
        match result {
            Some(WsMessage::WorktreeSelectResponse(r)) => {
                assert!(!r.success);
                assert!(r.error.is_some());
            }
            _ => panic!("Expected WorktreeSelectResponse with success=false"),
        }
    }

    #[tokio::test]
    async fn test_handle_worktree_select_request_valid_path() {
        let (_dir, repo_path) = setup_repo_with_file("file.txt", "content");
        let broadcaster = Arc::new(WsBroadcaster::default());
        // broadcaster の sender を張り、リモート観測結果（ブロードキャスト）を検証する。
        let (tx, mut rx) = WsBroadcaster::create_channel();
        broadcaster.set_sender(Some(tx));
        let selected = make_selected(None);
        let req = WorktreeSelectRequest {
            path: repo_path.clone(),
        };
        let _result = handle_worktree_select_request(
            &req,
            vec![repo_path.clone()],
            make_usecase(),
            &broadcaster,
            None,
            &selected,
        )
        .await;

        // 選択状態が更新される。
        {
            let wt = selected.lock().await;
            assert_eq!(wt.as_ref().unwrap(), &repo_path);
        }

        // ブロードキャストされた応答を収集する。
        let mut messages = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            messages.push(msg);
        }

        // 現在ブランチが BranchInfoResponse として通知される。
        let branch_info = messages.iter().find_map(|m| match m {
            WsMessage::BranchInfoResponse(b) => Some(b),
            _ => None,
        });
        assert!(
            branch_info.is_some_and(|b| !b.branch.is_empty()),
            "BranchInfoResponse が非空ブランチで通知されること"
        );

        // 成功した WorktreeSelectResponse が通知される。
        let select_ok = messages.iter().find_map(|m| match m {
            WsMessage::WorktreeSelectResponse(r) => Some(r),
            _ => None,
        });
        let select_ok = select_ok.expect("WorktreeSelectResponse が通知されること");
        assert!(select_ok.success);
        assert_eq!(select_ok.path, repo_path);
        assert!(select_ok.error.is_none());
    }

    #[tokio::test]
    async fn test_build_all_worktrees() {
        let (_dir, repo_path) = setup_repo_with_file("file.txt", "content");
        let worktrees = build_all_worktrees(vec![repo_path], make_usecase()).await;
        assert!(!worktrees.is_empty());
    }
}
