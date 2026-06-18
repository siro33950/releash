use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    Approve {
        #[serde(default)]
        comment: Option<String>,
    },
    Reject {
        reason: String,
    },
    Abort,
}
