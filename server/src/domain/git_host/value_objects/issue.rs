#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrAuthor {
    pub login: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Milestone {
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueLabel {
    pub name: String,
    pub color: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueInfo {
    pub number: u64,
    pub title: String,
    pub state: String,
    pub url: String,
    pub author: PrAuthor,
    pub created_at: String,
    pub updated_at: String,
    pub labels: Vec<IssueLabel>,
    pub assignees: Vec<PrAuthor>,
    pub body: String,
    pub milestone: Option<Milestone>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct IssueFilter {
    pub labels: Vec<String>,
    pub milestone: Option<String>,
}

impl IssueFilter {
    pub fn matches(&self, issue: &IssueInfo) -> bool {
        self.labels
            .iter()
            .all(|wanted| issue.labels.iter().any(|label| &label.name == wanted))
            && self.milestone.as_ref().is_none_or(|wanted| {
                issue
                    .milestone
                    .as_ref()
                    .is_some_and(|milestone| &milestone.title == wanted)
            })
    }
}

#[cfg(test)]
#[path = "issue_test.rs"]
mod issue_tests;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IssueOptions {
    pub labels: Vec<String>,
    pub milestones: Vec<String>,
}
impl IssueOptions {
    pub fn from_issues(issues: &[IssueInfo]) -> Self {
        Self {
            labels: issues
                .iter()
                .flat_map(|issue| issue.labels.iter().map(|label| label.name.clone()))
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect(),
            milestones: issues
                .iter()
                .filter_map(|issue| {
                    issue
                        .milestone
                        .as_ref()
                        .map(|milestone| milestone.title.clone())
                })
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect(),
        }
    }
}
