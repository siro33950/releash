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
        workspaces.watches(&repositories, &[], "", "", ""),
        vec![WatchRequirement::Git("/repo".into())]
    );
    assert_eq!(
        branch.watches(&[], &[], "", "", ""),
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

#[test]
fn test_review購読_worktreeのgit監視とcomment置き場のfile監視を要求する() {
    use crate::domain::code::{ReviewBase, ReviewSection};
    // Given
    let snapshot = SubscriptionTarget::ReviewSnapshot("/repo".into(), ReviewBase::Head);
    let view = SubscriptionTarget::ReviewFileView(
        "/repo".into(),
        "src/a.rs".into(),
        ReviewSection::Changes,
        ReviewBase::Head,
    );
    let threads = SubscriptionTarget::ReviewThreads("/repo".into());
    let history = SubscriptionTarget::SessionHistory("/repo".into(), 1);
    // When / Then
    for target in [&snapshot, &view] {
        assert_eq!(
            target.watches(&[], &[], "/data/review-comments", "", ""),
            vec![WatchRequirement::Git("/repo".into())]
        );
        assert!(target.affected_by(&StateChangeSource::Repository(vec!["/repo".into()])));
        assert!(!target.affected_by(&StateChangeSource::Repository(vec!["/other".into()])));
        assert!(!target.affected_by(&StateChangeSource::ReviewComments(None)));
    }
    assert_eq!(
        threads.watches(&[], &[], "/data/review-comments", "", ""),
        vec![WatchRequirement::Files(
            "/data/review-comments".into(),
            StateChangeSource::ReviewComments(None)
        )]
    );
    assert!(threads.affected_by(&StateChangeSource::ReviewComments(None)));
    assert!(threads.affected_by(&StateChangeSource::ReviewComments(Some("/repo".into()))));
    assert!(!threads.affected_by(&StateChangeSource::ReviewComments(Some("/other".into()))));
    assert!(!threads.affected_by(&StateChangeSource::Repository(vec!["/repo".into()])));
    assert_eq!(
        history.watches(&[], &["/history".into()], "/data/review-comments", "", ""),
        vec![WatchRequirement::Files(
            "/history".into(),
            StateChangeSource::ProviderHistory
        )]
    );
}

#[test]
fn test_automation購読_定義の変化に反応し置き場の監視を要求する() {
    use crate::domain::workflow::FacetKind;
    // Given
    let targets = [
        SubscriptionTarget::Workflows,
        SubscriptionTarget::Workflow("wf".into()),
        SubscriptionTarget::WorkflowSource("wf".into()),
        SubscriptionTarget::Facets(FacetKind::Policy),
        SubscriptionTarget::Facet(FacetKind::Knowledge, "key".into()),
        SubscriptionTarget::Diagnostics,
    ];
    // When / Then
    for target in &targets {
        assert!(
            target.affected_by(&StateChangeSource::WorkflowDefinitions),
            "{target:?}"
        );
        assert!(
            !target.affected_by(&StateChangeSource::Repository(vec!["/repo".into()])),
            "{target:?}"
        );
        assert!(
            !target.affected_by(&StateChangeSource::ProviderHistory),
            "{target:?}"
        );
        assert_eq!(
            target.watches(&["/repo".into()], &["/claude".into()], "", "/workflows", ""),
            vec![WatchRequirement::Files(
                "/workflows".into(),
                StateChangeSource::WorkflowDefinitions
            )],
            "{target:?}"
        );
    }
    assert!(SubscriptionTarget::Workflows.affected_by(&StateChangeSource::Worktree("/wt".into())));
    assert!(
        !SubscriptionTarget::Diagnostics.affected_by(&StateChangeSource::Worktree("/wt".into()))
    );
    assert!(!SubscriptionTarget::Workspaces.affected_by(&StateChangeSource::WorkflowDefinitions));
    assert_eq!(
        SubscriptionTarget::SessionHistory("/repo".into(), 20).watches(
            &[],
            &["/claude".into()],
            "",
            "/workflows",
            ""
        ),
        vec![WatchRequirement::Files(
            "/claude".into(),
            StateChangeSource::ProviderHistory
        )]
    );
}
