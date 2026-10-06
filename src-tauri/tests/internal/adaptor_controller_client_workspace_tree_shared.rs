use releash_lib::test_support::integration::transport::ClientCommandDispatch;
use releash_lib::test_support::integration::wire;
use releash_lib::test_support::integration::workspace::register_shared;

#[tokio::test]
pub async fn test_一覧更新dispatch_登録済みrepositoryの走査をやり直してから応答する() {
    // Given: 監視中の Repository に、監視が気付いていない worktree が増えている
    let (app, _data, _store) =
        crate::adaptor_controller_client_workflow_mod::tests::make_read_only_app();
    let deps = &app.client;
    let state = deps.app_state.clone().unwrap();
    let (repo_dir, repo) = crate::test_support_git::create_test_repo();
    crate::test_support_git::create_initial_commit(&repo);
    let path = repo_dir
        .path()
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    state.repo_paths_usecase.add(&path).unwrap();
    app.repository_state.start_git_dir_watching(&path).unwrap();
    let worktree_count = || async {
        state
            .workspace_list
            .read()
            .await
            .unwrap()
            .repositories
            .iter()
            .find(|repository| repository.path == path)
            .and_then(|repository| repository.worktrees.value.as_ref().map(Vec::len))
    };
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while worktree_count().await != Some(1) {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let worktrees = tempfile::tempdir().unwrap();
    repo.worktree("feature", &worktrees.path().join("feature"), None)
        .unwrap();
    let mut dispatch = ClientCommandDispatch::new(
        releash_lib::test_support::integration::daemon::DaemonUsecase::test_with_repository(
            releash_lib::test_support::integration::daemon::serving(),
        ),
    );
    register_shared(&mut dispatch, deps);

    // When
    let result = dispatch
        .dispatch(wire::command_request::Command::RefreshWorkspaces(
            wire::RefreshWorkspacesRequest {},
        ))
        .await
        .unwrap();

    // Then
    assert!(matches!(
        result,
        wire::command_result::Command::RefreshWorkspaces(_)
    ));
    assert_eq!(worktree_count().await, Some(2));
}
