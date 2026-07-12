#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    Running,
    WaitingApproval,
    Completed,
    Failed,
    Aborted,
    Interrupted,
}

impl RunStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Aborted | Self::Interrupted
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerSource {
    DesktopUi,
    Remote,
    Cli,
    Agent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatusFilter {
    Active,
    Terminal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunListFilter {
    pub status: Option<RunStatusFilter>,
    pub worktree_path: Option<String>,
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowRunRecord {
    pub run_id: String,
    pub workflow_name: String,
    pub task: Option<String>,
    pub status: RunStatus,
    pub worktree_path: String,
    pub current_node_name: Option<String>,
    pub trigger_source: TriggerSource,
    pub started_at: f64,
    pub updated_at: f64,
    pub completed_at: Option<f64>,
    pub error_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowRunSummary {
    pub run_id: String,
    pub workflow_name: String,
    pub task: Option<String>,
    pub status: RunStatus,
    pub worktree_path: String,
    pub current_node_name: Option<String>,
    pub trigger_source: TriggerSource,
    pub started_at: f64,
    pub updated_at: f64,
    pub completed_at: Option<f64>,
    pub error_reason: Option<String>,
}

#[cfg(test)]
mod run_tests {
    use super::*;

    #[test]
    fn test_run_status_terminal判定() {
        assert!(!RunStatus::Running.is_terminal());
        assert!(!RunStatus::WaitingApproval.is_terminal());
        assert!(RunStatus::Completed.is_terminal());
        assert!(RunStatus::Failed.is_terminal());
        assert!(RunStatus::Aborted.is_terminal());
        assert!(RunStatus::Interrupted.is_terminal());
    }
}
