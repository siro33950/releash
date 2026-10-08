//! 画面に出す木の読み取り。
//!
//! どの節を出すか、どの id と題で出すか、どの順に並べるかは集約が決める。
//! 読む側はここが返す並びをたどるだけでよい。

use std::collections::{HashMap, HashSet};

use super::{
    WorkspaceExecution, WorkspaceNodeKind, WorkspacePublicRoot, WorkspaceTree, WorkspaceTreeNode,
    WorkspaceTreeVisibilityPolicy,
};
use crate::domain::agent_session::aggregates::{AgentSession, AgentSessionLifecycle};
use crate::domain::workflow::ExecutionTreeLaunch;

/// archive 済みの実行と内部の節を除いた、画面に出す木。
pub struct WorkspaceVisibleTree<'a> {
    tree: &'a WorkspaceTree,
    children: HashMap<Option<&'a str>, Vec<&'a WorkspaceTreeNode>>,
    by_id: HashMap<&'a str, &'a WorkspaceTreeNode>,
    /// 実行を代表する節の id から、その実行の公開情報。
    roots: HashMap<&'a str, WorkspacePublicRoot<'a>>,
    hidden: HashSet<String>,
}

/// 画面に出す木の 1 項目。Workflow を表す節は現れず、最初の子が実行を代表する。
#[derive(Clone, Copy)]
pub struct WorkspaceVisibleNode<'a> {
    visible: &'a WorkspaceVisibleTree<'a>,
    node: &'a WorkspaceTreeNode,
    root: Option<WorkspacePublicRoot<'a>>,
}

impl WorkspaceTree {
    pub fn visible(&self) -> WorkspaceVisibleTree<'_> {
        let mut children: HashMap<Option<&str>, Vec<&WorkspaceTreeNode>> = HashMap::new();
        for node in self.nodes() {
            if !node.is_internal_rule_record() && !node.is_retry_history {
                children
                    .entry(node.parent_id.as_deref())
                    .or_default()
                    .push(node);
            }
        }
        for siblings in children.values_mut() {
            siblings.sort_by_key(|node| (node.sibling_order, node.id.as_str()));
        }
        WorkspaceVisibleTree {
            tree: self,
            children,
            by_id: self
                .nodes()
                .iter()
                .map(|node| (node.id.as_str(), node))
                .collect(),
            roots: WorkspacePublicRoot::all(self.nodes())
                .into_iter()
                .map(|root| (root.node().id.as_str(), root))
                .collect(),
            hidden: WorkspaceTreeVisibilityPolicy::hidden_branch_ids(
                self,
                self.executions()
                    .iter()
                    .filter(|execution| execution.archive.is_some())
                    .map(|execution| execution.execution_id.as_str()),
            ),
        }
    }

    /// archive 済みの session を持つ実行（session の識別子順）。
    pub fn archived_sessions(&self) -> Vec<&WorkspaceExecution> {
        let mut archived = self
            .executions()
            .iter()
            .filter(|execution| {
                execution.launched_as == ExecutionTreeLaunch::Session
                    && execution.session.as_ref().is_some_and(|session| {
                        session.lifecycle() == AgentSessionLifecycle::Archived
                    })
            })
            .collect::<Vec<_>>();
        archived.sort_by_key(|execution| execution.session.as_ref().map(AgentSession::id));
        archived
    }

    /// archive 済みの workflow の実行（archive の新しい順）。
    pub fn archived_workflows(&self) -> Vec<&WorkspaceExecution> {
        let mut archived = self
            .executions()
            .iter()
            .filter(|execution| {
                execution.launched_as == ExecutionTreeLaunch::Workflow
                    && execution.worktree_path == self.workspace_identity().as_str()
                    && execution.archive.is_some()
            })
            .collect::<Vec<_>>();
        archived.sort_by(|left, right| {
            let archived_at = |execution: &WorkspaceExecution| {
                execution
                    .archive
                    .as_ref()
                    .map_or(f64::NEG_INFINITY, |archive| archive.archived_at)
            };
            archived_at(right)
                .total_cmp(&archived_at(left))
                .then_with(|| left.execution_id.cmp(&right.execution_id))
        });
        archived
    }
}

impl<'a> WorkspaceVisibleTree<'a> {
    pub fn roots(&'a self) -> Vec<WorkspaceVisibleNode<'a>> {
        self.items(None)
    }

    fn items(&'a self, parent: Option<&str>) -> Vec<WorkspaceVisibleNode<'a>> {
        self.children
            .get(&parent)
            .into_iter()
            .flatten()
            .filter(|node| !self.hidden.contains(&node.id))
            .flat_map(|node| match node.kind {
                WorkspaceNodeKind::Workflow => self.items(Some(&node.id)),
                _ => vec![WorkspaceVisibleNode {
                    visible: self,
                    node,
                    root: self.roots.get(node.id.as_str()).copied(),
                }],
            })
            .collect()
    }

    /// 選択先にできる節が、画面に出す木にあるか。
    pub(crate) fn contains(&'a self, node_id: &str) -> bool {
        contains(&self.roots(), node_id)
    }

    /// 既定の選択先。実行中か待ちの葉を優先する。
    pub(crate) fn preferred_node_id(&self) -> Option<String> {
        self.tree.preferred_node_id(&self.hidden).map(|node_id| {
            self.roots
                .get(node_id.as_str())
                .map_or(node_id.clone(), |root| root.public_id().to_string())
        })
    }
}

fn contains(items: &[WorkspaceVisibleNode<'_>], node_id: &str) -> bool {
    items.iter().any(|item| match item.node.kind {
        WorkspaceNodeKind::Sequence | WorkspaceNodeKind::Fanout => {
            contains(&item.children(), node_id)
        }
        _ => {
            item.id() == node_id
                || contains(&item.children(), node_id)
                || item
                    .past_attempts()
                    .iter()
                    .any(|past| past.id() == node_id || contains(&past.children(), node_id))
        }
    })
}

impl<'a> WorkspaceVisibleNode<'a> {
    pub fn node(&self) -> &'a WorkspaceTreeNode {
        self.node
    }

    /// 公開する識別子。実行を代表する節は、実行の識別子で公開する。
    pub fn id(&self) -> &'a str {
        self.root
            .map_or(self.node.id.as_str(), |root| root.public_id())
    }

    pub fn title(&self) -> &'a str {
        self.root
            .map_or(self.node.title.as_str(), |root| root.public_title())
    }

    /// 実行を代表する節のとき、その実行の Workflow 節。
    pub(crate) fn execution_owner(&self) -> Option<&'a WorkspaceTreeNode> {
        self.root.map(|root| root.owner())
    }

    /// Session として起動した実行を代表する節のとき、その session。
    pub(crate) fn session(&self) -> Option<&'a AgentSession> {
        let root = self.root?;
        let session_id = self.node.session_id.as_deref()?;
        self.visible
            .tree
            .executions()
            .iter()
            .find(|execution| {
                execution.execution_id == root.public_id()
                    && execution.launched_as == ExecutionTreeLaunch::Session
            })?
            .session
            .as_ref()
            .filter(|session| session.id() == session_id)
    }

    pub fn children(&self) -> Vec<WorkspaceVisibleNode<'a>> {
        self.visible.items(Some(&self.node.id))
    }

    /// retry で置き換えられた過去の試行（古い順）。
    pub(crate) fn past_attempts(&self) -> Vec<WorkspaceVisibleNode<'a>> {
        self.node
            .past_attempt_ids
            .iter()
            .filter_map(|id| self.visible.by_id.get(id.as_str()).copied())
            .map(|node| WorkspaceVisibleNode {
                visible: self.visible,
                node,
                root: None,
            })
            .collect()
    }
}

#[cfg(test)]
#[path = "visible_test.rs"]
mod visible_tests;
