use crate::adaptor::gateway::workflow::schema::Workflow;

#[derive(Debug)]
pub(crate) enum WorkflowDefinitionResolverError {
    InvalidWorkflow(String),
    Infrastructure(String),
}

impl std::fmt::Display for WorkflowDefinitionResolverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidWorkflow(message) | Self::Infrastructure(message) => {
                write!(f, "{message}")
            }
        }
    }
}

#[derive(Debug)]
pub(crate) enum ManagedWorktreeResolverError {
    Validation(String),
}

impl std::fmt::Display for ManagedWorktreeResolverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Validation(message) => write!(f, "{message}"),
        }
    }
}

/// WorkflowRuntimeService core が workflow 定義の保存形式や builtin 解決方法を知らずに済むよう、
/// YAML / builtin / facet 解決を担う境界。
#[async_trait::async_trait]
pub(crate) trait WorkflowDefinitionResolver: Send + Sync {
    async fn resolve(
        &self,
        workflow_name: &str,
    ) -> Result<Workflow, WorkflowDefinitionResolverError>;
}

/// WorkflowRuntimeService core が AppConfig / filesystem canonicalize / Git worktree 列挙を
/// 直接知らずに済むよう、managed worktree 解決を担う境界。
#[async_trait::async_trait]
pub(crate) trait ManagedWorktreeResolver: Send + Sync {
    async fn resolve(&self, worktree_path: String) -> Result<String, ManagedWorktreeResolverError>;
}
