//! Workspaces の一覧・実行木・ブランチの状態を転送の形へ変える。
//!
//! どの節をどの順に出すかは domain の読み取りが決める。ここは、その並びをたどって詰め替えるだけ。

use super as wire;
use super::conversions::cv;
use crate::domain::git::Branch;
use crate::domain::workspace_tree::{WorkspaceNodeKind, WorkspaceTree, WorkspaceVisibleNode};
use crate::usecase::fetched::{FetchState, Fetched};
use crate::usecase::workspace_tree::{
    WorkspaceList, WorkspaceListRepository, WorkspaceListWorktree,
};

impl TryFrom<&WorkspaceList> for wire::WorkspaceListSnapshot {
    type Error = String;
    fn try_from(list: &WorkspaceList) -> Result<Self, String> {
        Ok(Self {
            status: Some(status(&Fetched::ready(()), list.repositories.is_empty())),
            repositories: Some(wire::ListWorkspaceRepositoryList {
                items: list
                    .repositories
                    .iter()
                    .map(repository)
                    .collect::<Result<_, _>>()?,
            }),
        })
    }
}

pub(crate) fn branch_status(branches: &[(Branch, bool)]) -> wire::ListBranchStatus {
    wire::ListBranchStatus {
        items: branches
            .iter()
            .map(|(branch, has_worktree)| wire::BranchStatus {
                name: Some(branch.name.clone()),
                has_worktree: Some(*has_worktree),
            })
            .collect(),
    }
}

fn status<T>(fetched: &Fetched<T>, empty: bool) -> wire::WorkspaceListStatus {
    wire::WorkspaceListStatus {
        state: Some(
            match fetched.state(empty) {
                FetchState::Loading => "loading",
                FetchState::InitialFailed => "initialFailed",
                FetchState::Empty => "empty",
                FetchState::Ready => "ready",
                FetchState::RefreshFailed => "refreshFailed",
            }
            .to_owned(),
        ),
        error: fetched.error.as_ref().map(ToString::to_string),
    }
}

fn repository(
    repository: &WorkspaceListRepository,
) -> Result<wire::WorkspaceRepositoryList, String> {
    let worktrees = repository.worktrees.value.as_deref().unwrap_or_default();
    Ok(wire::WorkspaceRepositoryList {
        path: Some(repository.path.clone()),
        status: Some(status(&repository.worktrees, worktrees.is_empty())),
        branches: Some(wire::ListWorkspaceBranch {
            items: worktrees.iter().map(branch).collect(),
        }),
        worktrees: Some(wire::ListWorkspaceWorktreeList {
            items: worktrees.iter().map(worktree).collect::<Result<_, _>>()?,
        }),
        name: Some(repository.name.clone()),
    })
}

fn branch(row: &WorkspaceListWorktree) -> wire::WorkspaceBranch {
    wire::WorkspaceBranch {
        removal_requires_force: row.worktree.removal_requires_force(row.dirty_count.value),
        pr_state_number: row.state_pull_request.as_ref().map(|pr| pr.number),
        pr_state: row.state_pull_request.as_ref().map(|pr| {
            match pr.classification() {
                crate::domain::git_host::value_objects::pr::PrClassification::Draft => "draft",
                crate::domain::git_host::value_objects::pr::PrClassification::Open => "open",
                crate::domain::git_host::value_objects::pr::PrClassification::Merged => "merged",
                crate::domain::git_host::value_objects::pr::PrClassification::Closed => "closed",
            }
            .to_owned()
        }),
        name: Some(row.worktree.branch.clone()),
        is_main_worktree: Some(row.worktree.is_main),
        is_deleting: Some(row.deleting),
        worktree_path: Some(row.worktree.path.clone()),
        dirty_count: row.dirty_count.value.map(|v| v as u64),
        dirty_count_error: row.dirty_count.error.as_ref().map(ToString::to_string),
        pull_request_error: row.pull_request_error.as_ref().map(ToString::to_string),
    }
}

fn worktree(row: &WorkspaceListWorktree) -> Result<wire::WorkspaceWorktreeList, String> {
    let snapshot = row.tree.value.as_ref().map(tree_snapshot).transpose()?;
    let empty = snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.nodes.as_ref())
        .is_none_or(|nodes| nodes.items.is_empty());
    Ok(wire::WorkspaceWorktreeList {
        executions: row
            .tree
            .value
            .iter()
            .flat_map(WorkspaceTree::card_executions)
            .map(|(execution, nodes)| wire::WorktreeExecutionSummary {
                id: execution.execution_id.clone(),
                title: nodes.node().title.clone(),
                is_workflow: nodes.is_workflow(),
                status: nodes
                    .node()
                    .status_classification
                    .as_public_str()
                    .to_owned(),
                node_count: nodes.node_count() as u64,
            })
            .collect(),
        path: Some(row.worktree.path.clone()),
        status: Some(status(&row.tree, empty)),
        snapshot,
    })
}

fn tree_snapshot(tree: &WorkspaceTree) -> Result<wire::WorkspaceTreeSnapshot, String> {
    let visible = tree.visible();
    Ok(wire::WorkspaceTreeSnapshot {
        nodes: Some(tree_items(&visible.roots())?),
    })
}

fn tree_items(items: &[WorkspaceVisibleNode<'_>]) -> Result<wire::ListWorkspaceTreeItem, String> {
    Ok(wire::ListWorkspaceTreeItem {
        items: items.iter().map(tree_item).collect::<Result<_, _>>()?,
    })
}

fn tree_item(item: &WorkspaceVisibleNode<'_>) -> Result<wire::WorkspaceTreeItem, String> {
    use wire::workspace_tree_item::Variant;
    let node = item.node();
    Ok(wire::WorkspaceTreeItem {
        variant: Some(match node.kind {
            WorkspaceNodeKind::Fanout => Variant::Fanout(wire::WorkspaceFanout {
                can_approve: Some(node.can_approve),
                id: Some(item.id().to_owned()),
                title: Some(item.title().to_owned()),
                status: Some(cv(node.status_classification.as_public_str())?),
                workflow_capabilities: workflow_capabilities(item),
                children: Some(tree_items(&item.children())?),
                updated_at: Some(node.updated_at()),
                error_reason: node.failure_reason(),
            }),
            WorkspaceNodeKind::Sequence => Variant::Sequence(wire::WorkspaceSequence {
                can_approve: Some(node.can_approve),
                id: Some(item.id().to_owned()),
                title: Some(item.title().to_owned()),
                status: Some(cv(node.status_classification.as_public_str())?),
                workflow_capabilities: workflow_capabilities(item),
                children: Some(tree_items(&item.children())?),
                updated_at: Some(node.updated_at()),
                error_reason: node.failure_reason(),
            }),
            _ => Variant::Node(tree_node(item)?),
        }),
    })
}

fn tree_node(item: &WorkspaceVisibleNode<'_>) -> Result<wire::WorkspaceNode, String> {
    let node = item.node();
    let past_attempts = item
        .past_attempts()
        .iter()
        .map(|past| {
            Ok(wire::WorkspacePastAttempt {
                variant: Some(wire::workspace_past_attempt::Variant::Node(tree_node(
                    past,
                )?)),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(wire::WorkspaceNode {
        session_id: node.session_id.clone(),
        execution_id: node.execution_id.clone(),
        node_execution_id: node.node_execution_id.clone(),
        node_name: node.node_name.clone(),
        artifact_count: Some(item.artifact_count() as u32),
        model: node.model.clone(),
        provider: item.provider().map(Into::into),
        process_presence: Some(cv(node.process_presence.as_str())?),
        id: Some(item.id().to_owned()),
        title: Some(item.title().to_owned()),
        status: Some(cv(node.status_classification.as_public_str())?),
        error_reason: node.failure_reason(),
        content_kind: Some(cv(if node.kind == WorkspaceNodeKind::WorkflowCommand {
            "command"
        } else {
            "session"
        })?),
        capabilities: Some(wire::WorkspaceNodeCapabilitiesDto {
            can_rename: Some(node.can_rename),
            can_approve: Some(node.can_approve),
            can_retry: Some(node.can_retry),
            can_resume_session: Some(node.can_resume_session),
        }),
        workflow_capabilities: workflow_capabilities(item),
        children: Some(tree_items(&item.children())?),
        past_attempts: Some(wire::ListWorkspaceNode {
            items: past_attempts,
        }),
        updated_at: Some(node.updated_at()),
    })
}

fn workflow_capabilities(
    item: &WorkspaceVisibleNode<'_>,
) -> Option<wire::WorkspaceWorkflowCapabilities> {
    item.execution_owner()
        .map(|owner| wire::WorkspaceWorkflowCapabilities {
            can_abort: Some(owner.can_abort),
            can_archive: Some(owner.can_archive),
        })
}

#[cfg(test)]
#[path = "workspace_test.rs"]
mod workspace_tests;
