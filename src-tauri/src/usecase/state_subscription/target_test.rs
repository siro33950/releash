use super::*;

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
fn test_失敗購読_対象と全体に更新を配信する() {
    for target in ["tree", "*"] {
        let subscription = SubscriptionTarget::Failures(target.into(), 0);
        assert!(subscription.affected_by(&StateChangeSource::Failures("tree".into())));
    }
    assert!(!SubscriptionTarget::Failures("other".into(), 0)
        .affected_by(&StateChangeSource::Failures("tree".into())));
}

#[test]
fn test_監視要件_共有対象とbranch対象の必要な監視を選ぶ() {
    // Given
    let repositories = vec!["/repo".into()];
    let workspaces = SubscriptionTarget::Workspaces;
    let branch = SubscriptionTarget::Branches("/repo".into(), Some("feature".into()));
    // When / Then
    assert_eq!(
        workspaces.watches(&repositories, &[]),
        vec![WatchRequirement::Git("/repo".into())]
    );
    assert_eq!(
        branch.watches(&[], &[]),
        vec![WatchRequirement::Git("/repo".into())]
    );
    assert!(branch.affected_by(&StateChangeSource::Repository(vec!["/repo".into()])));
    assert!(!branch.affected_by(&StateChangeSource::Repository(vec!["/other".into()])));
}

#[test]
fn test_repository通知_path一致の対象だけを選ぶ() {
    // Given
    let targets = [
        SubscriptionTarget::BranchBase("/repo".into(), "feature".into()),
        SubscriptionTarget::BranchStatus("/repo".into()),
        SubscriptionTarget::CurrentBranch("/repo".into()),
        SubscriptionTarget::Worktrees("/repo".into()),
        SubscriptionTarget::RepositoryRoot("/repo".into()),
    ];
    // When / Then
    for target in targets {
        assert!(
            target.affected_by(&StateChangeSource::Repository(vec!["/repo".into()])),
            "{target:?}"
        );
        assert!(
            !target.affected_by(&StateChangeSource::Repository(vec!["/other".into()])),
            "{target:?}"
        );
    }
    assert!(SubscriptionTarget::Providers.affected_by(&StateChangeSource::Providers));
    assert!(!SubscriptionTarget::Providers.affected_by(&StateChangeSource::ProviderHistory));
}
