use super::*;

#[test]
fn test_issue絞り込み_labelとmilestoneを同時に満たすものを選ぶ() {
    // Given
    let issue = IssueInfo {
        number: 1,
        title: "title".into(),
        state: "open".into(),
        url: "url".into(),
        author: PrAuthor {
            login: "author".into(),
        },
        created_at: "date".into(),
        updated_at: "date".into(),
        labels: vec![IssueLabel {
            name: "bug".into(),
            color: "red".into(),
        }],
        assignees: vec![],
        body: "".into(),
        milestone: Some(Milestone { title: "v1".into() }),
    };
    // When / Then
    assert!(IssueFilter::default().matches(&issue));
    assert!(IssueFilter {
        labels: vec!["bug".into()],
        milestone: Some("v1".into())
    }
    .matches(&issue));
    assert!(!IssueFilter {
        labels: vec!["feature".into()],
        milestone: None
    }
    .matches(&issue));
    assert!(!IssueFilter {
        labels: vec![],
        milestone: Some("v2".into())
    }
    .matches(&issue));
    assert!(!IssueFilter {
        labels: vec!["bug".into(), "feature".into()],
        milestone: None
    }
    .matches(&issue));
}

#[test]
fn test_issue選択肢_未絞り込みの全件から重複を除き辞書順に並べる() {
    // Given
    let issue = |number, labels: &[&str], milestone: &str| IssueInfo {
        number,
        title: "issue".into(),
        state: "OPEN".into(),
        url: "url".into(),
        author: PrAuthor {
            login: "author".into(),
        },
        created_at: "created".into(),
        updated_at: "updated".into(),
        labels: labels
            .iter()
            .map(|label| IssueLabel {
                name: (*label).into(),
                color: "color".into(),
            })
            .collect(),
        assignees: vec![],
        body: String::new(),
        milestone: Some(Milestone {
            title: milestone.into(),
        }),
    };
    let first = issue(1, &["z", "a", "a"], "later");
    let other = issue(2, &["b", "z"], "earlier");
    // When
    let options = IssueOptions::from_issues(&[first, other, issue(3, &["a"], "later")]);
    // Then
    assert_eq!(options.labels, ["a", "b", "z"]);
    assert_eq!(options.milestones, ["earlier", "later"]);
    assert_eq!(IssueOptions::from_issues(&[]), IssueOptions::default());
}
