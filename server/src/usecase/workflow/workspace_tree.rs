use serde::Serialize;

use super::workspace_node_command::{
    WorkspaceNodeActionResolver, WorkspaceNodeApprovalTarget, WorkspaceNodeRetryTarget,
    WorkspaceSessionNodeRenameTarget,
};
use super::WorkflowUsecase;
use crate::domain::workflow::WorkflowError;
use crate::domain::workspace_tree::{WorkspaceIdentity, WorkspaceTree};
use crate::usecase::fetched::Fetched;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceNodeCapabilitiesDto {
    pub can_rename: bool,
    pub can_approve: bool,
    pub can_retry: bool,
    pub can_resume_session: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct NodeWorktreeDto {
    pub branch: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceNodeDetailDto {
    pub classification: String,
    pub node_execution_id: Option<String>,
    pub node_name: Option<String>,
    pub model: Option<String>,
    pub past_attempts: Vec<WorkspaceNodeAttemptDto>,
    pub execution_id: Option<String>,
    pub can_abort: bool,
    pub process_presence: &'static str,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<NodeWorktreeDto>,
    pub id: String,
    pub title: String,
    pub status: String,
    pub submit_received: bool,
    pub stop_received: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting_for: Option<&'static str>,
    pub has_artifact: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_reason: Option<String>,
    pub capabilities: WorkspaceNodeCapabilitiesDto,
    pub updated_at: f64,
    pub content: WorkspaceNodeContentDto,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceNodeAttemptDto {
    pub id: String,
    pub title: String,
    pub error_reason: Option<String>,
    pub updated_at: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum WorkspaceNodeContentDto {
    Session(WorkspaceSessionNodeContentDto),
    Command(WorkspaceCommandNodeContentDto),
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSessionNodeContentDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceCommandNodeContentDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_command: Option<String>,
    pub shell: Option<String>,
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<WorkspaceCommandResultDto>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceCommandResultDto {
    pub exit_code: i64,
    /// Milliseconds, matching the command Artifact reserved field.
    pub duration: u64,
    pub stdout: String,
    pub stderr: String,
}

impl WorkflowUsecase {
    /// worktree の実行木。要対応の失敗を反映した集約を返す。
    pub async fn workspace_tree(
        &self,
        worktree_path: &str,
    ) -> Result<WorkspaceTree, WorkflowError> {
        let identity = WorkspaceIdentity::new(self.resolve_worktree_path(worktree_path)?);
        self.load_workspace_trees(&[identity])
            .await
            .pop()
            .expect("one tree per workspace")
    }

    async fn load_workspace_trees(
        &self,
        identities: &[WorkspaceIdentity],
    ) -> Vec<Result<WorkspaceTree, WorkflowError>> {
        self.workspace_nodes
            .load_trees(identities)
            .await
            .into_iter()
            .map(|tree| {
                tree.map(|mut tree| {
                    self.observe_failures(&mut tree);
                    tree
                })
            })
            .collect()
    }

    /// 一覧に出す worktree ごとの実行木。読めなかった worktree は、最後に読めた木を残す。
    /// 指定に無い worktree の保持値は捨てる。
    ///
    /// 渡すのは Repository の走査で読めた worktree の場所なので、管理対象かは確かめ直さない。
    pub async fn retained_workspace_trees(
        &self,
        worktree_paths: &[String],
    ) -> Vec<Fetched<WorkspaceTree>> {
        let identities = worktree_paths
            .iter()
            .map(WorkspaceIdentity::new)
            .collect::<Vec<_>>();
        let results = self.load_workspace_trees(&identities).await;
        let mut retained = self.retained_trees.lock();
        retained.retain(|path, _| worktree_paths.contains(path));
        worktree_paths
            .iter()
            .zip(results)
            .map(|(path, result)| {
                let entry = retained.entry(path.clone()).or_default();
                entry.record(
                    result.map_err(|error| crate::domain::failure::WorkFailure::from_error(&error)),
                );
                entry.clone()
            })
            .collect()
    }

    fn observe_failures(&self, tree: &mut WorkspaceTree) {
        for (target, message) in
            self.failure_observations(tree.nodes().iter().map(|node| node.classification_input()))
        {
            tree.observe_background_failure(&target, &message);
        }
    }

    fn failure_observations(
        &self,
        nodes: impl IntoIterator<Item = crate::domain::workspace_tree::WorkspaceNodeClassification>,
    ) -> Vec<(String, String)> {
        let targets = nodes
            .into_iter()
            .flat_map(|node| [Some(node.id), node.node_execution_id, node.execution_id])
            .flatten()
            .collect::<std::collections::HashSet<_>>();
        targets
            .into_iter()
            .flat_map(|target| {
                self.failures
                    .attention_messages(&target)
                    .into_iter()
                    .map(move |message| (target.clone(), message))
            })
            .collect()
    }

    pub(crate) async fn load_workspace_state(
        &self,
        repository: &dyn crate::domain::workspace_state::WorkspaceStateRepository,
        name: &str,
        path: &str,
    ) -> Result<
        (
            Option<crate::domain::workspace_state::WorkspaceState>,
            Vec<crate::domain::failure::WorkFailure>,
        ),
        crate::domain::workspace_state::WorkspaceStateError,
    > {
        crate::usecase::workspace_state::usecase::load_workspace_state(
            repository,
            self.workspace_query.as_ref(),
            name,
            path,
        )
        .await
    }

    pub(crate) async fn get_workspace_node_detail(
        &self,
        worktree_path: &str,
        node_id: &str,
    ) -> Result<Option<WorkspaceNodeDetailDto>, WorkflowError> {
        let workspace = crate::domain::workspace_tree::WorkspaceIdentity::new(
            self.resolve_worktree_path(worktree_path)?,
        );
        self.workspace_query.node_detail(&workspace, node_id).await
    }

    pub async fn get_session_node_detail(
        &self,
        session: &crate::usecase::agent_session::AgentSessionItemDto,
    ) -> Result<Option<WorkspaceNodeDetailDto>, WorkflowError> {
        let Some(mut read) = self
            .workspace_query
            .session_node_detail(
                &WorkspaceIdentity::new(session.workspace_worktree_path.clone()),
                &session.tree_location.tree_id,
                &session.tree_location.node_execution_id,
            )
            .await?
        else {
            return Ok(None);
        };
        for (target, message) in self.failure_observations(read.classification.iter().cloned()) {
            for node in &mut read.classification {
                let mut reason = None;
                let reason = if node.id == read.detail.id {
                    &mut read.detail.error_reason
                } else {
                    &mut reason
                };
                node.observe_background_failure(&target, &message, reason);
            }
        }
        read.detail.classification =
            crate::domain::workspace_tree::WorkspaceNodeClassification::aggregate(
                &read.classification,
                &read.detail.id,
            )
            .as_public_str()
            .into();
        Ok(Some(read.detail))
    }

    pub(crate) async fn get_workspace_node_artifact(
        &self,
        node_execution_id: &str,
    ) -> Result<super::query_service::WorkflowGetOutputResult, WorkflowError> {
        self.workspace_query.node_artifact(node_execution_id).await
    }

    /// 実行木と、選択している Node が画面に出す木にあるか。
    pub async fn workspace_tree_selection(
        &self,
        worktree_path: &str,
        selected_node_id: &str,
    ) -> Result<(WorkspaceTree, bool), WorkflowError> {
        let tree = self.workspace_tree(worktree_path).await?;
        let selected = tree.visible().contains(selected_node_id);
        Ok((tree, selected))
    }

    pub(crate) async fn archive_workspace_workflow_execution(
        &self,
        runtime: &super::WorkflowRuntimeUsecase,
        worktree_path: &str,
        execution_id: &str,
    ) -> Result<(), WorkflowError> {
        self.authorize_archive_target(worktree_path, execution_id)
            .await?;
        runtime.archive_execution_tree(execution_id, "manual").await
    }

    pub(crate) async fn restore_workspace_workflow_execution(
        &self,
        runtime: &super::WorkflowRuntimeUsecase,
        worktree_path: &str,
        execution_id: &str,
    ) -> Result<(), WorkflowError> {
        self.authorize_archive_target(worktree_path, execution_id)
            .await?;
        runtime.restore_execution_tree(execution_id).await
    }

    pub async fn authorize_archive_target(
        &self,
        worktree_path: &str,
        execution_id: &str,
    ) -> Result<(), WorkflowError> {
        crate::domain::workflow::ExecutionTreeId::new(execution_id.to_string())?;
        let target = self.execution_archives.target(execution_id).await?;
        if crate::domain::workspace_tree::WorkspaceIdentity::new(
            self.resolve_worktree_path(worktree_path)?,
        ) != crate::domain::workspace_tree::WorkspaceIdentity::new(&target.workspace_identity)
        {
            return Err(WorkflowError::validation(
                "execution tree worktree does not match",
            ));
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl WorkspaceNodeActionResolver for WorkflowUsecase {
    async fn resolve_approval_target(
        &self,
        worktree_path: &str,
        node_id: &str,
    ) -> Result<WorkspaceNodeApprovalTarget, WorkflowError> {
        let workspace = crate::domain::workspace_tree::WorkspaceIdentity::new(
            self.resolve_worktree_path(worktree_path)?,
        );
        let node = self
            .workspace_nodes
            .load_node(&workspace, node_id)
            .await
            .map_err(|error| WorkflowError::external(error.to_string()))?
            .ok_or_else(|| {
                WorkflowError::NotFound(format!("Workspace node not found: {node_id}"))
            })?;
        let (Some(execution_id), Some(node_execution_id), Some(node_name)) =
            (node.execution_id, node.node_execution_id, node.node_name)
        else {
            return Err(WorkflowError::invalid_state(
                "Workspace node is not a Workflow Node",
            ));
        };
        Ok(WorkspaceNodeApprovalTarget {
            execution_id,
            node_name,
            node_execution_id,
        })
    }

    async fn resolve_retry_target(
        &self,
        worktree_path: &str,
        node_id: &str,
    ) -> Result<WorkspaceNodeRetryTarget, WorkflowError> {
        let workspace = crate::domain::workspace_tree::WorkspaceIdentity::new(
            self.resolve_worktree_path(worktree_path)?,
        );
        let node = self
            .workspace_nodes
            .load_node(&workspace, node_id)
            .await
            .map_err(|error| WorkflowError::external(error.to_string()))?
            .ok_or_else(|| {
                WorkflowError::NotFound(format!("Workspace node not found: {node_id}"))
            })?;
        let (Some(execution_id), Some(node_execution_id)) =
            (node.execution_id, node.node_execution_id)
        else {
            return Err(WorkflowError::invalid_state(
                "Workspace node is not a Workflow Node",
            ));
        };
        Ok(WorkspaceNodeRetryTarget {
            execution_id,
            node_execution_id,
        })
    }

    async fn resolve_session_resume_target(
        &self,
        worktree_path: &str,
        node_id: &str,
    ) -> Result<super::command::ResumeSessionNodeCommand, WorkflowError> {
        let workspace = crate::domain::workspace_tree::WorkspaceIdentity::new(
            self.resolve_worktree_path(worktree_path)?,
        );
        let node = self
            .workspace_nodes
            .load_node(&workspace, node_id)
            .await
            .map_err(|error| WorkflowError::external(error.to_string()))?
            .ok_or_else(|| {
                WorkflowError::NotFound(format!("Workspace node not found: {node_id}"))
            })?;
        let (Some(execution_id), Some(node_execution_id)) =
            (node.execution_id, node.node_execution_id)
        else {
            return Err(WorkflowError::invalid_state(
                "Workspace node is not a Workflow Node",
            ));
        };
        Ok(super::command::ResumeSessionNodeCommand {
            execution_id,
            node_execution_id,
        })
    }

    async fn resolve_session_rename_target(
        &self,
        worktree_path: &str,
        node_id: &str,
    ) -> Result<WorkspaceSessionNodeRenameTarget, WorkflowError> {
        let workspace = crate::domain::workspace_tree::WorkspaceIdentity::new(
            self.resolve_worktree_path(worktree_path)?,
        );
        let node = self
            .workspace_nodes
            .load_node(&workspace, node_id)
            .await
            .map_err(|error| WorkflowError::external(error.to_string()))?
            .ok_or_else(|| {
                WorkflowError::NotFound(format!("Workspace node not found: {node_id}"))
            })?;
        if !node.can_rename {
            return Err(WorkflowError::invalid_state(
                "Workspace node cannot be renamed",
            ));
        }
        let agent_session_id = node
            .session_id
            .ok_or_else(|| WorkflowError::invalid_state("Workspace Session Node is not bound"))?;
        Ok(WorkspaceSessionNodeRenameTarget { agent_session_id })
    }
}
