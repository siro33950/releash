use super::*;

#[test]
fn test_agent_session購読_worktree通知はpathによらず選びrepository通知では選ばない() {
    // Given
    let target = SubscriptionTarget::SessionTab("session".into());
    // When / Then
    for path in ["/repo", "/other"] {
        assert!(target.affected_by(&StateChangeSource::Worktree(path.into())));
        assert!(!target.affected_by(&StateChangeSource::Repository(vec![path.into()])));
    }
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
        SubscriptionTarget::Branches("/repo".into(), None),
        SubscriptionTarget::AvailableBranches("/repo".into()),
        SubscriptionTarget::ReleashBase("/repo".into()),
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
fn test_automation購読_定義の変化に反応し置き場の監視を要求する() {
    use crate::domain::workflow::FacetKind;
    // Given
    let targets = [
        SubscriptionTarget::Workflows,
        SubscriptionTarget::Workflow("wf".into()),
        SubscriptionTarget::Facets(FacetKind::Policy),
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

#[test]
fn test_cli購読通知_実行は全worktreeでreviewは全comment通知で読み直す() {
    // Given / When / Then
    for target in [
        SubscriptionTarget::WorkflowExecution("id".into()),
        SubscriptionTarget::WorkflowOutput("id".into(), "node".into()),
        SubscriptionTarget::NodeArtifact("node-execution".into()),
    ] {
        assert!(target.affected_by(&StateChangeSource::Worktree("/other".into())));
        assert!(!target.affected_by(&StateChangeSource::ReviewComments(None)));
    }
    for target in [
        SubscriptionTarget::ReviewSessionThreads("id".into(), Default::default()),
        SubscriptionTarget::ReviewWorktreeThreads("/repo".into(), Default::default()),
        SubscriptionTarget::ReviewSessionThread("id".into(), "thread".into()),
        SubscriptionTarget::ReviewSessionThreadHistory("id".into(), "thread".into()),
    ] {
        assert!(target.affected_by(&StateChangeSource::ReviewComments(Some("/other".into()))));
        assert_eq!(
            target.watches(&[], &[], "/comments", "", ""),
            vec![WatchRequirement::Files(
                "/comments".into(),
                StateChangeSource::ReviewComments(None)
            )]
        );
    }
}

#[test]
fn test_daemon状態購読_daemon変化だけを受け取り外部watchを増やさない() {
    // Given / When / Then
    let target = SubscriptionTarget::DaemonInfo;
    assert!(target.affected_by(&StateChangeSource::Daemon));
    assert!(!target.affected_by(&StateChangeSource::AppConfig));
    assert!(!SubscriptionTarget::Workspaces.affected_by(&StateChangeSource::Daemon));
    assert!(target.watches(&[], &[], "", "", "").is_empty());
}

#[test]
fn test_ファイルタブと木_対応するファイルとgitの変化を購読する() {
    // Given
    let targets = [
        SubscriptionTarget::Directory("/repo".into(), "src".into()),
        SubscriptionTarget::FileTab("/repo".into(), "src/file.rs".into(), ReviewBase::Head),
    ];
    // When / Then
    for target in targets {
        assert!(target.affected_by(&StateChangeSource::Worktree("/repo".into())));
        assert!(target.affected_by(&StateChangeSource::Repository(vec!["/repo".into()])));
        assert!(!target.affected_by(&StateChangeSource::Worktree("/other".into())));
        assert_eq!(target.watches(&[], &[], "", "", "").len(), 2);
    }
}

#[test]
fn test_ファイルthread購読_該当worktreeのcomment変更だけを受ける() {
    // Given
    let target = SubscriptionTarget::FileThreads("/repo".into(), "file.rs".into());
    // When / Then
    assert!(target.affected_by(&StateChangeSource::ReviewComments(Some("/repo".into()))));
    assert!(target.affected_by(&StateChangeSource::ReviewComments(None)));
    assert!(!target.affected_by(&StateChangeSource::ReviewComments(Some("/other".into()))));
}

#[test]
fn test_sessionタブ購読_idだけを受け取りworktree変化で更新する() {
    // Given
    let args = ["agent-session"];
    // When
    let target = SubscriptionTarget::from_parts("session-tab", &args).unwrap();
    // Then
    assert_eq!(
        SubscriptionTarget::parse(&target.to_string()).unwrap(),
        target
    );
    assert!(target.affected_by(&StateChangeSource::Worktree("/repo".into())));
    assert!(SubscriptionTarget::from_parts("session-tab", &[]).is_err());
    assert!(SubscriptionTarget::from_parts("session-tab", &["/repo", "agent-session"]).is_err());
    assert!(SubscriptionTarget::from_parts("session-tab", &[" "]).is_err());
}

#[test]
fn test_実行木概要購読_全worktreeと失敗の通知で更新し外部監視を追加しない() {
    // Given
    let target = SubscriptionTarget::ExecutionOverview;
    assert!(target.affected_by(&StateChangeSource::WorkspaceList));
    // When / Then
    for path in ["/repo", "/other"] {
        assert!(target.affected_by(&StateChangeSource::Worktree(path.into())));
        assert!(!target.affected_by(&StateChangeSource::Repository(vec![path.into()])));
    }
    for source in [
        StateChangeSource::Repositories,
        StateChangeSource::WorkspaceState("workspace".into()),
        StateChangeSource::WorkflowDefinitions,
        StateChangeSource::Providers,
        StateChangeSource::Daemon,
    ] {
        assert!(!target.affected_by(&source));
    }
    assert!(target
        .watches(
            &["/repo".into()],
            &["/history".into()],
            "/comments",
            "/workflows",
            "/hooks"
        )
        .is_empty());
}

#[test]
fn test_notion購読対象_外部の情報としてissueと同じ契機で取り直す() {
    // Given
    let targets = [
        SubscriptionTarget::NotionTasks(crate::usecase::notion::usecase::NotionTaskListRequest {
            path: "/repo".into(),
            count: 20,
            title: None,
            labels: Default::default(),
        }),
        SubscriptionTarget::Issues("/repo".into()),
    ];
    // When
    let results: Vec<_> = targets
        .iter()
        .map(|target| {
            (
                target.external_information(),
                target.affected_by(&StateChangeSource::Repositories),
            )
        })
        .collect();
    // Then
    assert_eq!(results, vec![(true, false); targets.len()]);
}

#[test]
fn test_notion購読対象_同じrepoの設定変更で取り直す() {
    // Given
    let targets = [SubscriptionTarget::NotionTasks(
        crate::usecase::notion::usecase::NotionTaskListRequest {
            path: "/repo".into(),
            count: 20,
            title: None,
            labels: Default::default(),
        },
    )];
    // When
    let results: Vec<_> = targets
        .iter()
        .map(|target| target.affected_by(&StateChangeSource::NotionConfig("/repo".into())))
        .collect();
    // Then
    assert_eq!(results, vec![true; targets.len()]);
}

#[test]
fn test_notion購読対象_別のrepoの設定変更と他の設定変更では取り直さない() {
    // Given
    let targets = [SubscriptionTarget::NotionTasks(
        crate::usecase::notion::usecase::NotionTaskListRequest {
            path: "/repo".into(),
            count: 20,
            title: None,
            labels: Default::default(),
        },
    )];
    // When
    let results: Vec<_> = targets
        .iter()
        .map(|target| {
            (
                target.affected_by(&StateChangeSource::NotionConfig("/other".into())),
                target.affected_by(&StateChangeSource::AppConfig),
            )
        })
        .collect();
    // Then
    assert_eq!(results, vec![(false, false); targets.len()]);
}
