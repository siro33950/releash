#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowNodeContext {
    pub execution_id: String,
    pub node_execution_id: String,
    pub workflow_name: String,
    pub node_name: String,
    pub attempt: u32,
    pub parent_node_name: Option<String>,
    pub parent_attempt: Option<u32>,
    pub order: u32,
    pub startup_timeout_secs: Option<u64>,
    pub startup_max_retries: Option<u32>,
    pub stale_timeout_secs: Option<u64>,
}
