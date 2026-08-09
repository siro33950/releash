use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::domain::git_host::{
    issue_branch_name, IssueInfo, IssueLabel, Milestone, PrAuthor, PrInfo, PrStatus,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrInfoDto {
    pub number: u64,
    pub url: String,
}

impl From<PrInfo> for PrInfoDto {
    fn from(pr: PrInfo) -> Self {
        Self {
            number: pr.number,
            url: pr.url,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrStatusDto {
    pub open_prs: HashMap<String, PrInfoDto>,
    pub merged_branches: Vec<String>,
}

impl From<PrStatus> for PrStatusDto {
    fn from(status: PrStatus) -> Self {
        Self {
            open_prs: status
                .open_prs
                .into_iter()
                .map(|(branch, pr)| (branch, pr.into()))
                .collect(),
            merged_branches: status.merged_branches,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrAuthorDto {
    pub login: String,
}

impl From<PrAuthor> for PrAuthorDto {
    fn from(author: PrAuthor) -> Self {
        Self {
            login: author.login,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MilestoneDto {
    pub title: String,
}

impl From<Milestone> for MilestoneDto {
    fn from(milestone: Milestone) -> Self {
        Self {
            title: milestone.title,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueLabelDto {
    pub name: String,
    pub color: String,
}

impl From<IssueLabel> for IssueLabelDto {
    fn from(label: IssueLabel) -> Self {
        Self {
            name: label.name,
            color: label.color,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueInfoDto {
    pub number: u64,
    pub default_branch_name: String,
    pub title: String,
    pub state: String,
    pub url: String,
    pub author: PrAuthorDto,
    pub created_at: String,
    pub updated_at: String,
    pub labels: Vec<IssueLabelDto>,
    pub assignees: Vec<PrAuthorDto>,
    pub body: String,
    pub milestone: Option<MilestoneDto>,
}

impl From<IssueInfo> for IssueInfoDto {
    fn from(issue: IssueInfo) -> Self {
        Self {
            number: issue.number,
            default_branch_name: issue_branch_name(issue.number),
            title: issue.title,
            state: issue.state,
            url: issue.url,
            author: issue.author.into(),
            created_at: issue.created_at,
            updated_at: issue.updated_at,
            labels: issue.labels.into_iter().map(Into::into).collect(),
            assignees: issue.assignees.into_iter().map(Into::into).collect(),
            body: issue.body,
            milestone: issue.milestone.map(Into::into),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn pr_status_dto_serializes_existing_wire_shape() {
        let dto = PrStatusDto::from(PrStatus {
            open_prs: HashMap::from([(
                "feat/test".to_string(),
                PrInfo {
                    number: 42,
                    url: "https://github.com/owner/repo/pull/42".to_string(),
                },
            )]),
            merged_branches: vec!["feat/done".to_string()],
        });

        assert_eq!(
            serde_json::to_value(dto).unwrap(),
            json!({
                "open_prs": {
                    "feat/test": {
                        "number": 42,
                        "url": "https://github.com/owner/repo/pull/42"
                    }
                },
                "merged_branches": ["feat/done"]
            })
        );
    }

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
