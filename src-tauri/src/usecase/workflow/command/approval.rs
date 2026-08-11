#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalCommand {
    pub execution_id: String,
    pub node_name: String,
    pub node_execution_id: Option<String>,
    pub comment: Option<String>,
}
