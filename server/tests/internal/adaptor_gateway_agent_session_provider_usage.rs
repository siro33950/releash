use releashd::test_support::integration::provider_usage::*;

#[test]
fn test_使用率_claudeの片窓と読めないplanでも反映する() {
    // Given
    let root = tempfile::tempdir().unwrap();
    let source = LocalProviderUsageSource {
        claude_account: root.path().join(".claude.json"),
        codex_home: root.path().into(),
    };
    // When
    let usage = source.claude(br#"{"rate_limits":{"five_hour":{"used_percentage":81,"resets_at":123},"seven_day":null}}"#).unwrap().unwrap();
    // Then
    assert_eq!(usage.windows.len(), 1);
    assert!(usage.windows[0].warning);
    assert_eq!(usage.windows[0].window_minutes, 300);
    assert!(usage.plan.is_none());
    std::fs::write(&source.claude_account, br#"{"oauthAccount":{"organizationType":"claude_max","organizationRateLimitTier":"default_claude_max_20x"}}"#).unwrap();
    assert_eq!(
        source
            .claude(br#"{"rate_limits":{"seven_day":{"used_percentage":80,"resets_at":123}}}"#)
            .unwrap()
            .unwrap()
            .plan
            .as_deref(),
        Some("claude_max · default_claude_max_20x")
    );
    assert!(source.claude(br#"{"rate_limits":null}"#).unwrap().is_none());
    assert!(source
        .claude(br#"{"rate_limits":{"five_hour":{"used_percentage":101,"resets_at":123}}}"#)
        .is_err());
    assert!(source.claude(b"bad json").is_err());
}

#[test]
fn test_使用率_codexは対象sessionの最新の完結したtoken_countを読む() {
    // Given
    let root = tempfile::tempdir().unwrap();
    let rollout = root.path().join("session.jsonl");
    let connection = rusqlite::Connection::open(root.path().join("state_5.sqlite")).unwrap();
    connection
        .execute_batch("CREATE TABLE threads(id TEXT PRIMARY KEY, rollout_path TEXT)")
        .unwrap();
    connection
        .execute(
            "INSERT INTO threads VALUES (?1, ?2)",
            ["session", rollout.to_str().unwrap()],
        )
        .unwrap();
    std::fs::write(&rollout, concat!(
        "{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"rate_limits\":{\"primary\":{\"used_percent\":20,\"window_minutes\":300,\"resets_at\":123}}}}\n",
        "{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"rate_limits\":{\"primary\":{\"used_percent\":90,\"window_minutes\":300,\"resets_at\":456},\"plan_type\":\"pro\"}}}\n",
        "{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\""
    )).unwrap();
    // When
    let (_, usage) = read_codex(root.path(), "session").unwrap().unwrap();
    // Then
    assert_eq!(usage.windows[0].used_percent, 90.0);
    assert_eq!(usage.windows[0].resets_at, 456);
    assert_eq!(usage.plan.as_deref(), Some("pro"));
    assert!(read_codex(root.path(), "other").unwrap().is_none());
}
