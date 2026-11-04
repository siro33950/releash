use crate::domain::workspace_tree::WorkspaceNodeStatusClassification;

/// 全 worktree の実行木の集約状態と、archive されていない実行木の一覧。
#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionOverviewDto {
    pub status: WorkspaceNodeStatusClassification,
    pub executions: Vec<ExecutionOverviewEntryDto>,
}

/// 実行木の表示名と状態、および選択先の worktree。
#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionOverviewEntryDto {
    pub worktree_path: String,
    pub worktree_name: String,
    pub name: String,
    pub status: WorkspaceNodeStatusClassification,
}
