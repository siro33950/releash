//! Workspaces の一覧・実行木・ブランチの状態を転送の形へ変える。
//!
//! どの節をどの順に出すかは domain の読み取りが決める。ここは、その並びをたどって詰め替えるだけ。

use super as wire;
use super::conversions::cv;
use crate::domain::agent_session::aggregates::{AgentSession, AgentSessionLifecycle};
use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::repository::Branch;
use crate::domain::workspace_tree::{
    WorkspaceExecution, WorkspaceNodeKind, WorkspaceTree, WorkspaceTreeNode, WorkspaceVisibleNode,
};
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

pub(crate) fn selection(
    tree: &WorkspaceTree,
    selection_in_snapshot: bool,
) -> Result<wire::WorkspaceTreeSelectionSnapshot, String> {
    Ok(wire::WorkspaceTreeSelectionSnapshot {
        snapshot: Some(tree_snapshot(tree)?),
        reconciliation: Some(wire::WorkspaceSelectionReconciliation {
            selection_in_snapshot: Some(selection_in_snapshot),
        }),
    })
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
        loaded: Some(fetched.loaded()),
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
    })
}

fn branch(row: &WorkspaceListWorktree) -> wire::WorkspaceBranch {
    wire::WorkspaceBranch {
        upstream: row
            .tracking
            .value
            .as_ref()
            .and_then(|tracking| tracking.as_ref())
            .map(|tracking| tracking.upstream.clone()),
        ahead: row
            .tracking
            .value
            .as_ref()
            .and_then(|tracking| tracking.as_ref())
            .map(|tracking| tracking.ahead as u64),
        behind: row
            .tracking
            .value
            .as_ref()
            .and_then(|tracking| tracking.as_ref())
            .map(|tracking| tracking.behind as u64),
        tracking_error: row.tracking.error.as_ref().map(ToString::to_string),
        removal_requires_force: row.worktree.removal_requires_force(row.dirty_count.value),
        pr_state: row.pull_request.as_ref().map(|pr| {
            match (pr.state, pr.draft) {
                (crate::domain::git_host::PrState::Open, true) => "draft",
                (crate::domain::git_host::PrState::Open, false) => "open",
                (crate::domain::git_host::PrState::Merged, _) => "merged",
                (crate::domain::git_host::PrState::Closed, _) => "closed",
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
        is_merged: Some(row.merged),
        has_pr: row
            .pull_request_loaded
            .then_some(row.pull_request.is_some()),
        pr_number: row.pull_request.as_ref().map(|pr| pr.number),
        pr_url: row.pull_request.as_ref().map(|pr| pr.url.clone()),
    }
}

fn worktree(row: &WorkspaceListWorktree) -> Result<wire::WorkspaceWorktreeList, String> {
    let snapshot = row.tree.value.as_ref().map(tree_snapshot).transpose()?;
    let empty = snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.nodes.as_ref())
        .is_none_or(|nodes| nodes.items.is_empty());
    Ok(wire::WorkspaceWorktreeList {
        aggregate_status: row
            .tree
            .value
            .as_ref()
            .and_then(WorkspaceTree::card_status)
            .map(|status| status.as_public_str().to_owned()),
        executions: row
            .tree
            .value
            .iter()
            .flat_map(WorkspaceTree::card_executions)
            .map(|summary| wire::WorktreeExecutionSummary {
                id: summary.id,
                title: summary.title,
                is_workflow: summary.is_workflow,
                provider: summary.provider.map(|provider| {
                    match provider {
                        ProviderKind::Claude => "claude",
                        ProviderKind::Codex => "codex",
                    }
                    .to_owned()
                }),
                status: summary.status.as_public_str().to_owned(),
                node_count: summary.node_count as u64,
                session_states: summary
                    .session_states
                    .into_iter()
                    .map(|state| state.as_public_str().to_owned())
                    .collect(),
            })
            .collect(),
        path: Some(row.worktree.path.clone()),
        status: Some(status(&row.tree, empty)),
        workflow_history: Some(wire::ListWorkspaceWorkflowHistoryItem {
            items: row
                .tree
                .value
                .iter()
                .flat_map(WorkspaceTree::archived_workflows)
                .map(history_item)
                .collect::<Result<_, _>>()?,
        }),
        snapshot,
    })
}

fn history_item(
    execution: &WorkspaceExecution,
) -> Result<wire::WorkspaceWorkflowHistoryItem, String> {
    let archive = execution.archive.as_ref();
    Ok(wire::WorkspaceWorkflowHistoryItem {
        execution_id: Some(execution.execution_id.clone()),
        worktree_path: Some(execution.worktree_path.clone()),
        title: Some(execution.workflow_name.clone()),
        status: Some(cv(execution.status.as_str())?),
        updated_at: Some(execution.updated_at),
        archived_at: archive.map(|archive| archive.archived_at),
        archive_reason: archive.map(|archive| archive.archive_reason.clone()),
    })
}

fn tree_snapshot(tree: &WorkspaceTree) -> Result<wire::WorkspaceTreeSnapshot, String> {
    let visible = tree.visible();
    Ok(wire::WorkspaceTreeSnapshot {
        nodes: Some(tree_items(&visible.roots())?),
        archived_sessions: Some(wire::ListAgentSessionItemDto {
            items: tree
                .archived_sessions()
                .into_iter()
                .filter_map(|execution| {
                    execution
                        .session
                        .as_ref()
                        .map(|session| session_item(session, &execution.worktree_path))
                })
                .collect::<Result<_, _>>()?,
        }),
        preferred_node_id: visible.preferred_node_id(),
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
                worktree: node_worktree(node),
                id: Some(item.id().to_owned()),
                title: Some(item.title().to_owned()),
                status: Some(cv(node.status_classification.as_public_str())?),
                workflow_capabilities: workflow_capabilities(item),
                children: Some(tree_items(&item.children())?),
                updated_at: Some(node.updated_at()),
            }),
            WorkspaceNodeKind::Sequence => Variant::Sequence(wire::WorkspaceSequence {
                worktree: node_worktree(node),
                id: Some(item.id().to_owned()),
                title: Some(item.title().to_owned()),
                status: Some(cv(node.status_classification.as_public_str())?),
                workflow_capabilities: workflow_capabilities(item),
                children: Some(tree_items(&item.children())?),
                updated_at: Some(node.updated_at()),
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
        process_presence: Some(cv(node.process_presence.as_str())?),
        id: Some(item.id().to_owned()),
        title: Some(item.title().to_owned()),
        status: Some(cv(node.status_classification.as_public_str())?),
        error_reason: node.error_reason.clone(),
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
        session_capabilities: item.session().map(|session| {
            let operations = session.operations();
            wire::WorkspaceSessionCapabilities {
                session_ref: Some(session.id().to_owned()),
                can_archive: Some(operations.can_archive),
                can_delete: Some(operations.can_delete),
            }
        }),
        children: Some(tree_items(&item.children())?),
        past_attempts_collapsed: Some(!past_attempts.is_empty()),
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

fn node_worktree(node: &WorkspaceTreeNode) -> Option<wire::NodeWorktreeDto> {
    node.worktree
        .as_ref()
        .map(|worktree| wire::NodeWorktreeDto {
            branch: Some(worktree.branch.clone()),
            path: Some(worktree.path.clone()),
        })
}

fn session_item(
    session: &AgentSession,
    workspace_worktree_path: &str,
) -> Result<wire::AgentSessionItemDto, String> {
    let location = session.tree_location();
    let operations = session.operations();
    Ok(wire::AgentSessionItemDto {
        id: Some(session.id().to_owned()),
        workspace_identity: Some(session.workspace().as_str().to_owned()),
        worktree_path: Some(session.worktree_path().to_owned()),
        workspace_worktree_path: Some(workspace_worktree_path.to_owned()),
        provider: Some(wire::AgentSessionProviderDto {
            value: Some(match session.provider() {
                ProviderKind::Claude => wire::agent_session_provider_dto::Value::Claude as i32,
                ProviderKind::Codex => wire::agent_session_provider_dto::Value::Codex as i32,
            }),
        }),
        tree_location: Some(wire::AgentSessionTreeLocationDto {
            tree_id: Some(location.tree_id().to_owned()),
            node_execution_id: Some(location.node_execution_id().to_owned()),
        }),
        lifecycle: Some(wire::AgentSessionLifecycleDto {
            value: Some(match session.lifecycle() {
                AgentSessionLifecycle::Open => {
                    wire::agent_session_lifecycle_dto::Value::Open as i32
                }
                AgentSessionLifecycle::Paused => {
                    wire::agent_session_lifecycle_dto::Value::Paused as i32
                }
                AgentSessionLifecycle::Archived => {
                    wire::agent_session_lifecycle_dto::Value::Archived as i32
                }
            }),
        }),
        provider_session_id: session.provider_session_id().map(str::to_owned),
        transcript_ref: session.transcript_ref().map(str::to_owned),
        operations: Some(wire::AgentSessionOperationsDto {
            can_archive: Some(operations.can_archive),
            can_restore: Some(operations.can_restore),
            can_delete: Some(operations.can_delete),
        }),
        last_exit_abnormal: Some(session.last_exit_abnormal()),
        terminal_presence: None,
    })
}

#[cfg(test)]
#[path = "workspace_test.rs"]
mod workspace_tests;
