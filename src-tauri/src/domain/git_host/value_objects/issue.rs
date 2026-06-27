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
