use releash_lib::client_api_acceptance::{
    connect_client, read_current_branch, BranchGateway, ClientApiAcceptanceHost,
};
use std::sync::Arc;

#[tokio::test(flavor = "multi_thread")]
async fn test_状態購読の実配線で現在のbranchを届ける() {
    // Given
    let data = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    let git = git2::Repository::init(repo.path()).unwrap();
    let tree_id = git.index().unwrap().write_tree().unwrap();
    let tree = git.find_tree(tree_id).unwrap();
    let signature = git2::Signature::now("test", "test@example.com").unwrap();
    git.commit(
        Some("refs/heads/ws-branch"),
        &signature,
        &signature,
        "initial",
        &tree,
        &[],
    )
    .unwrap();
    git.set_head("refs/heads/ws-branch").unwrap();
    let host = ClientApiAcceptanceHost::start(data.path(), Arc::new(BranchGateway));
    let client = connect_client(&host.endpoint());

    // When
    let branch = read_current_branch(
        &client,
        serde_json::json!({"repoPath": repo.path().to_str().unwrap()}),
    )
    .await
    .unwrap();

    // Then
    assert_eq!(branch, "ws-branch");
}
