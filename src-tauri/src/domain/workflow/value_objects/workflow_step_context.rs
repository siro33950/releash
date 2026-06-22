#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowStepContext {
    pub run_id: String,
    pub workflow_name: String,
    pub step_name: String,
    pub run_index: u32,
    pub parent_step_name: Option<String>,
    pub parent_run_index: Option<u32>,
    pub order: u32,
}

impl WorkflowStepContext {
    pub fn group_step_name(&self) -> &str {
        self.parent_step_name.as_deref().unwrap_or(&self.step_name)
    }

    pub fn group_run_index(&self) -> u32 {
        self.parent_run_index.unwrap_or(self.run_index)
    }
}
