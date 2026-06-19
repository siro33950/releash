#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalDecision {
    Approve { comment: Option<String> },
    Reject { reason: String },
    Abort,
}
