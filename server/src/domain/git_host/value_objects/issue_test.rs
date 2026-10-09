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
    let mut other = issue.clone();
    other.labels.push(IssueLabel {
        name: "feature".into(),
        color: "blue".into(),
    });
    other.milestone = Some(Milestone { title: "v2".into() });
    let options = IssueOptions::from_issues(&[other, issue.clone(), issue.clone()]);
    assert_eq!(options.labels, ["bug", "feature"]);
    assert_eq!(options.milestones, ["v1", "v2"]);
    assert_eq!(IssueOptions::from_issues(&[]), IssueOptions::default());
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
