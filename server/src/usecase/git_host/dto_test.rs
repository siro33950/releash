pub(crate) mod tests {
    use serde_json::json;

    use super::super::*;

    #[test]
    fn issue_info_dto_serializes_existing_wire_shape() {
        let dto = IssueInfoDto::from(IssueInfo {
            number: 305,
            title: "Add issue panel".to_string(),
            state: "OPEN".to_string(),
            url: "https://github.com/owner/repo/issues/305".to_string(),
            author: PrAuthor {
                login: "user1".to_string(),
            },
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-02T00:00:00Z".to_string(),
            labels: vec![IssueLabel {
                name: "enhancement".to_string(),
                color: "a2eeef".to_string(),
            }],
            assignees: vec![PrAuthor {
                login: "user2".to_string(),
            }],
            body: "Issue body".to_string(),
            milestone: Some(Milestone {
                title: "Milestone".to_string(),
            }),
        });

        assert_eq!(
            serde_json::to_value(dto).unwrap(),
            json!({
                "number": 305,
                "default_branch_name": "feat/issues/305",
                "title": "Add issue panel",
                "state": "OPEN",
                "url": "https://github.com/owner/repo/issues/305",
                "author": {"login": "user1"},
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-02T00:00:00Z",
                "labels": [{"name": "enhancement", "color": "a2eeef"}],
                "assignees": [{"login": "user2"}],
                "body": "Issue body",
                "milestone": {"title": "Milestone"}
            })
        );
    }
}
