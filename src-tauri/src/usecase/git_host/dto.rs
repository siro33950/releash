use serde::{Deserialize, Serialize};

use crate::domain::git_host::{issue_branch_name, IssueInfo, IssueLabel, Milestone, PrAuthor};

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
#[path = "dto_test.rs"]
mod dto_tests;
