use super::*;

#[test]
fn test_購読対象_区切り文字と日本語を含む引数が往復する() {
    // Given
    let target = SubscriptionTarget::BranchBase("/作業:repo".into(), "feat/test".into());
    // When
    let parsed = SubscriptionTarget::parse(&target.to_string());
    // Then
    assert_eq!(parsed, Ok(target));
}

#[test]
fn test_購読対象_空の引数と不正な件数と未知の名前を拒否する() {
    // Given / When / Then
    for value in [
        "branches:0:",
        "branches:8:/repo",
        "session-history:5:/repo1:0",
        "session-history:5:/repo2:-1",
        "missing",
    ] {
        assert!(SubscriptionTarget::parse(value).is_err(), "{value}");
    }
    for count in [1, 100, 120, 402, 420] {
        let target = SubscriptionTarget::SessionHistory("/repo".into(), count);
        assert_eq!(SubscriptionTarget::parse(&target.to_string()), Ok(target));
    }
}

#[test]
fn test_agent_session購読_worktree通知はpathによらず選びrepository通知では選ばない() {
    // Given
    let target = SubscriptionTarget::AgentSession("session".into());
    // When / Then
    for path in ["/repo", "/other"] {
        assert!(target.affected_by(&StateChangeSource::Worktree(path.into())));
        assert!(!target.affected_by(&StateChangeSource::Repository(vec![path.into()])));
    }
}

#[test]
fn test_購読対象_構造化入力を検証し対象名との変換を所有する() {
    // Given
    let args = ["/作業:repo", "feat/test"];
    // When
    let target = SubscriptionTarget::from_parts("branch-base", &args).unwrap();
    // Then
    assert_eq!(
        target,
        SubscriptionTarget::BranchBase(args[0].into(), args[1].into())
    );
    assert_eq!(
        target.parts(),
        ("branch-base", args.map(String::from).to_vec())
    );
    assert_eq!(SubscriptionTarget::parse(&target.to_string()), Ok(target));
    for (name, args) in [
        ("branches", vec![""]),
        ("branches", vec![" "]),
        ("branches", vec!["/repo\0"]),
        ("session-history", vec!["/repo", "0"]),
        ("session-history", vec!["/repo", "invalid"]),
        ("session-history", vec!["/repo", "020"]),
        ("session-history", vec!["/repo", "+20"]),
        ("providers", vec!["extra"]),
        ("unknown", vec![]),
    ] {
        assert!(SubscriptionTarget::from_parts(name, &args).is_err());
    }
}

#[test]
fn test_失敗購読_対象と全体に更新を配信する() {
    for target in ["tree", "*"] {
        let subscription = SubscriptionTarget::Failures(target.into(), 0);
        assert_eq!(
            SubscriptionTarget::parse(&subscription.to_string()),
            Ok(subscription.clone())
        );
        assert!(subscription.affected_by(&StateChangeSource::Failures("tree".into())));
    }
    assert!(!SubscriptionTarget::Failures("other".into(), 0)
        .affected_by(&StateChangeSource::Failures("tree".into())));
}

#[test]
fn test_失敗購読_ページ指定を検証して往復する() {
    // Given / When
    let target = SubscriptionTarget::from_parts("failures", &["*", "100"]).unwrap();
    // Then
    assert_eq!(SubscriptionTarget::parse(&target.to_string()), Ok(target));
    for offset in ["0", "-1", "+100", "0100", "4096", "invalid"] {
        assert!(SubscriptionTarget::from_parts("failures", &["*", offset]).is_err());
    }
}
