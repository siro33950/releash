use releashd::test_support::client_api_acceptance::{
    connect_client, read_current_branch, ClientApiAcceptanceHost, GitGateway,
};
use releashd::test_support::integration::settings::{
    AppConfig, ReleashConfig, RepositoryConfigUsecase,
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
    let app_config = Arc::new(AppConfig::new(
        ReleashConfig::default(),
        data.path().join("releash.toml"),
    ));
    let gateway = Arc::new(GitGateway::new(app_config.clone()));
    let config = RepositoryConfigUsecase::new(app_config, gateway.clone(), Default::default());
    let path = repo.path().to_str().unwrap();
    config.set_releash_base(path, Some("ws-branch")).unwrap();
    let host = ClientApiAcceptanceHost::start(data.path(), gateway);
    let client = connect_client(&host.endpoint());
    // When
    let branch = read_current_branch(&client, serde_json::json!({"repoPath": path}))
        .await
        .unwrap();
    // Then
    assert_eq!(branch, "ws-branch");
    assert_eq!(
        config.get_releash_base(path).unwrap().as_deref(),
        Some("ws-branch")
    );
    // When
    config.set_releash_base(path, None).unwrap();
    // Then
    assert_eq!(config.get_releash_base(path).unwrap(), None);
}
