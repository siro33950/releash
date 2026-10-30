use super::*;

#[test]
fn test_base解決_個別指定_repository指定_既定の順で選ぶ() {
    // Given / When / Then
    for (overrides, base, default, expected) in [
        (
            vec![("feature", "topic")],
            Some("develop"),
            Some("main"),
            Some("topic"),
        ),
        (
            vec![("other", "topic")],
            Some("develop"),
            Some("main"),
            Some("develop"),
        ),
        (vec![], None, Some("main"), Some("main")),
        (vec![], None, None, None),
    ] {
        let config = RepositoryConfig::new(
            "/repo".into(),
            base.map(str::to_owned),
            overrides
                .into_iter()
                .map(|(branch, base)| (branch.into(), base.into()))
                .collect(),
            default.map(str::to_owned),
            None,
        );
        assert_eq!(config.resolve_base_branch("feature"), expected);
        assert_eq!(config.base_branch(), base);
    }
}

#[test]
fn test_不要な個別指定_存在しないbranchだけ返す() {
    // Given
    let config = RepositoryConfig::new(
        "/repo".into(),
        None,
        HashMap::from([
            ("alive".into(), "main".into()),
            ("stale.one".into(), "main".into()),
            ("stale/two".into(), "main".into()),
        ]),
        Some("main".into()),
        None,
    );

    // When
    let mut stale = config.stale_branch_overrides(&["alive".into(), "unconfigured".into()]);
    stale.sort();

    // Then
    assert_eq!(stale, ["stale.one", "stale/two"]);
    assert_eq!(config.stale_branch_overrides(&[]).len(), 3);
    assert!(config
        .stale_branch_overrides(&["alive".into(), "stale.one".into(), "stale/two".into()])
        .is_empty());
}

#[test]
fn test_repository設定_復元と変更で他の設定を保つ() {
    // Given
    let notion = NotionRepoConfig {
        api_token: "token".into(),
        database_id: "database".into(),
        property_mapping: Default::default(),
    };
    let mut config = RepositoryConfig::new(
        "/repo".into(),
        Some("develop".into()),
        HashMap::new(),
        Some("main".into()),
        Some(notion.clone()),
    );

    // When / Then
    assert_eq!(config.repo_path(), "/repo");
    assert_eq!(config.notion(), Some(&notion));
    assert_eq!(config.configured_notion(), Some(&notion));
    config.set_base_branch(None);
    assert_eq!(config.base_branch(), None);
    assert_eq!(config.resolve_base_branch("feature"), Some("main"));
    config.set_notion(None);
    assert_eq!(config.notion(), None);
    assert_eq!(config.resolve_base_branch("feature"), Some("main"));
}
