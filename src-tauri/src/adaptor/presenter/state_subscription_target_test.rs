use super::*;

#[test]
fn test_購読対象_廃止した識別子検索とroot計算を受理しない() {
    for target in ["session-node", "repository-root"] {
        assert_eq!(
            SubscriptionTarget::from_parts(target, &["/repo", "session"]),
            Err(SubscriptionError::UnknownTarget)
        );
    }
}

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
fn test_review購読対象_baseとsectionを検証して往復する() {
    // Given
    let targets = [
        SubscriptionTarget::ReviewSnapshot("/作業:repo".into(), ReviewBase::BranchBase),
        SubscriptionTarget::ReviewFileView(
            "/repo".into(),
            "src/a.rs".into(),
            ReviewSection::Staged,
            ReviewBase::Head,
        ),
        SubscriptionTarget::ReviewThreads("/repo".into()),
    ];
    // When / Then
    for target in targets {
        assert_eq!(SubscriptionTarget::parse(&target.to_string()), Ok(target));
    }
    assert_eq!(
        SubscriptionTarget::from_parts("review-file-view", &["/repo", "a.rs", "changes", "head"])
            .unwrap()
            .parts(),
        (
            "review-file-view",
            vec![
                "/repo".into(),
                "a.rs".into(),
                "changes".into(),
                "head".into()
            ]
        )
    );
    for (name, args) in [
        ("review-snapshot", vec!["/repo", "main"]),
        ("review-snapshot", vec!["/repo"]),
        ("review-file-view", vec!["/repo", "a.rs", "all", "head"]),
        ("review-file-view", vec!["/repo", "a.rs", "changes"]),
        ("review-threads", vec![]),
        ("review-threads", vec![""]),
    ] {
        assert!(
            SubscriptionTarget::from_parts(name, &args).is_err(),
            "{name} {args:?}"
        );
    }
}

#[test]
fn test_automation購読対象_facet種別を検証して往復する() {
    use crate::domain::workflow::FacetKind;
    // Given / When / Then
    for (name, args, target) in [
        ("workflows", vec![], SubscriptionTarget::Workflows),
        (
            "workflow",
            vec!["dev"],
            SubscriptionTarget::Workflow("dev".into()),
        ),
        (
            "workflow-source",
            vec!["dev"],
            SubscriptionTarget::WorkflowSource("dev".into()),
        ),
        (
            "facets",
            vec!["policy"],
            SubscriptionTarget::Facets(FacetKind::Policy),
        ),
        (
            "facet",
            vec!["instruction", "review:guide"],
            SubscriptionTarget::Facet(FacetKind::Instruction, "review:guide".into()),
        ),
        ("diagnostics", vec![], SubscriptionTarget::Diagnostics),
    ] {
        let parsed = SubscriptionTarget::from_parts(name, &args).unwrap();
        assert_eq!(parsed, target);
        assert_eq!(
            parsed.parts(),
            (
                name,
                args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>()
            )
        );
        assert_eq!(SubscriptionTarget::parse(&parsed.to_string()), Ok(parsed));
    }
    for (name, args) in [
        ("workflows", vec!["extra"]),
        ("workflow", vec![]),
        ("facets", vec!["policies"]),
        ("facets", vec!["Policy"]),
        ("facet", vec!["policy"]),
        ("facet", vec!["unknown", "key"]),
        ("diagnostics", vec!["/dir"]),
    ] {
        assert!(
            SubscriptionTarget::from_parts(name, &args).is_err(),
            "{name} {args:?}"
        );
    }
}

#[test]
fn test_notion購読対象_絞り込み無しの引数は往復で同じ対象になる() {
    // Given
    let args = ["/repo", "20"];
    // When
    let plain = SubscriptionTarget::from_parts("notion-tasks", &args).unwrap();
    let parsed = SubscriptionTarget::parse(&plain.to_string()).unwrap();
    // Then
    assert_eq!(
        plain.parts(),
        ("notion-tasks", args.map(String::from).to_vec())
    );
    assert_eq!(parsed, plain);
}

#[test]
fn test_notion購読対象_ラベルの並び順と重複だけが違う引数は同じ対象と同じ文字列になる() {
    // Given
    let first = [
        "/repo",
        "40",
        "title=Task",
        r#"labels={"Tags":["z","a","a"],"Status":["Todo"]}"#,
    ];
    let second = [
        "/repo",
        "40",
        r#"labels={"Status":["Todo"],"Tags":["a","z"]}"#,
        "title=Task",
    ];
    // When
    let a = SubscriptionTarget::from_parts("notion-tasks", &first).unwrap();
    let b = SubscriptionTarget::from_parts("notion-tasks", &second).unwrap();
    let parsed = SubscriptionTarget::parse(&a.to_string()).unwrap();
    // Then
    assert_eq!(a, b);
    assert_eq!(a.to_string(), b.to_string());
    assert_eq!(
        a.parts(),
        (
            "notion-tasks",
            vec![
                "/repo".into(),
                "40".into(),
                "title=Task".into(),
                r#"labels={"Status":["Todo"],"Tags":["a","z"]}"#.into()
            ]
        )
    );
    assert_eq!(parsed, b);
}

#[test]
fn test_notion購読対象_不正な引数はinvalid_idにする() {
    // Given
    let cases = [
        vec!["/repo", "0"],
        vec!["/repo", "020"],
        vec!["/repo", "+20"],
        vec!["/repo", "20", "title="],
        vec!["/repo", "20", "labels={}"],
        vec!["/repo", "20", "labels=invalid"],
        vec!["/repo", "20", "labels={\"Status\":[]}"],
        vec!["/repo", "20", "title=a", "title=b"],
        vec![
            "/repo",
            "20",
            r#"labels={"Status":["Todo"]}"#,
            r#"labels={"Status":["Done"]}"#,
        ],
        vec!["/repo", "20", r#"labels={"":["Todo"]}"#],
        vec!["/repo", "20", r#"labels={"Status":[""]}"#],
        vec!["/repo", "20", "unknown=a"],
    ];
    // When
    let results: Vec<_> = cases
        .iter()
        .map(|args| SubscriptionTarget::from_parts("notion-tasks", args))
        .collect();
    // Then
    for (args, result) in cases.iter().zip(results) {
        assert_eq!(result, Err(SubscriptionError::InvalidId), "{args:?}");
    }
}
