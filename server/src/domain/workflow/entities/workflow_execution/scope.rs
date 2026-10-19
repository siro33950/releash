//! Composite scope state inside the execution tree.
//!
//! 合成子（sequence / fanout）の実行インスタンスごとに、進行カーソル・子の
//! 実行カウント・子の Artifact をスコープとして所有する。loop_guard のカウント
//! 範囲と兄弟参照の解決空間はこのスコープに閉じ、同じ部品 sequence を再訪する
//! たび新しいスコープが生まれてカウントはフレッシュになる。

use std::collections::{HashMap, HashSet};

use crate::domain::workflow::services::routing::{route_in_scope, RouteDecision};
use crate::domain::workflow::value_objects::RuntimeArtifact;
use crate::domain::workflow::{
    NodeDefinition, NodeExecutionStatus, NodeKind, WorkflowDefinition, WorkflowError,
};

use super::{ExecutionTree, FanoutChildRuntime, RuntimeNodeExecution};

/// アクティブな合成子実行インスタンスのスコープ状態。
///
/// 合成子インスタンスが完了・失敗・中断で確定するとスコープは実行木から
/// 取り除かれる（確定した事実は node_executions とイベントが持つ）。
#[derive(Debug, Clone, PartialEq)]
pub struct ScopeRuntime {
    /// この合成子インスタンスの NodeExecution id。
    pub node_execution_id: String,
    /// 定義 node 名。
    pub node_name: String,
    /// 親スコープ（合成子インスタンス）の NodeExecution id。root は None。
    pub parent_scope_id: Option<String>,
    /// 起動時に親スコープの解決空間から束縛された input パラメータ値。
    pub parameters: Vec<(String, serde_json::Value)>,
    pub kind: ScopeRuntimeKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ScopeRuntimeKind {
    Sequence(SequenceScopeRuntime),
    Fanout(FanoutScopeRuntime),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SequenceScopeRuntime {
    /// スコープ内カーソル: 現在アクティブな子の定義名。
    pub current_child: Option<String>,
    /// スコープ内の子ごとの開始回数（attempt 採番と loop_guard 判定）。
    pub child_counts: HashMap<String, u32>,
    /// スコープ内で子が確定させた Artifact（兄弟参照・統合 map の解決空間）。
    pub artifacts: HashMap<String, RuntimeArtifact>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct FanoutScopeRuntime {
    /// 展開に使った items（スコープ生成時に確定。slot の item 復元に使う）。
    pub items: Option<Vec<serde_json::Value>>,
    /// 展開された子スロット（宣言順 = items 行 × children 列）。
    /// attempt は slot が所有する（lane ごとに 1 始まり）。
    pub children: Vec<FanoutChildRuntime>,
}

impl ScopeRuntime {
    pub fn sequence(&self) -> Option<&SequenceScopeRuntime> {
        match &self.kind {
            ScopeRuntimeKind::Sequence(scope) => Some(scope),
            ScopeRuntimeKind::Fanout(_) => None,
        }
    }

    pub fn sequence_mut(&mut self) -> Option<&mut SequenceScopeRuntime> {
        match &mut self.kind {
            ScopeRuntimeKind::Sequence(scope) => Some(scope),
            ScopeRuntimeKind::Fanout(_) => None,
        }
    }

    pub fn fanout(&self) -> Option<&FanoutScopeRuntime> {
        match &self.kind {
            ScopeRuntimeKind::Fanout(scope) => Some(scope),
            ScopeRuntimeKind::Sequence(_) => None,
        }
    }

    pub fn fanout_mut(&mut self) -> Option<&mut FanoutScopeRuntime> {
        match &mut self.kind {
            ScopeRuntimeKind::Fanout(scope) => Some(scope),
            ScopeRuntimeKind::Sequence(_) => None,
        }
    }
}

impl SequenceScopeRuntime {
    pub(crate) fn replay_child_start(&mut self, name: &str, attempt: u32) {
        self.raise_child_count_to(name, attempt);
        self.current_child = Some(name.into());
        self.artifacts.remove(name);
    }

    pub(crate) fn record_child_result(&mut self, artifact: RuntimeArtifact) {
        self.artifacts.insert(artifact.node_name.clone(), artifact);
    }

    pub(crate) fn artifact_field(
        &self,
        name: &str,
        path: &crate::domain::workflow::FieldPath,
    ) -> Option<&serde_json::Value> {
        self.artifacts
            .get(name)
            .and_then(|artifact| artifact.artifact.as_ref())
            .and_then(|artifact| {
                crate::domain::workflow::services::reference::resolve_value_at_path(artifact, path)
            })
    }

    /// 子の開始を記録し、その attempt（スコープ内 1 始まり）を返す。
    /// loop_guard のカウント範囲もこの値で決まる。fanout の子の attempt は
    /// slot が所有するため、このカウントは sequence スコープ専用。
    pub fn record_child_start(&mut self, child_name: &str) -> u32 {
        let count = self.child_counts.entry(child_name.to_string()).or_insert(0);
        *count += 1;
        *count
    }

    /// 子の attempt カウントを少なくとも `attempt` まで進める（retry / replay 用）。
    pub fn raise_child_count_to(&mut self, child_name: &str, attempt: u32) {
        self.child_counts
            .entry(child_name.to_string())
            .and_modify(|current| *current = (*current).max(attempt))
            .or_insert(attempt);
    }
}

pub(super) enum CompletionParent<'a> {
    Root,
    Delegate(&'a str),
    Scope(&'a str),
}

impl ExecutionTree {
    fn scope_children(&self, scope_id: &str) -> Vec<&RuntimeNodeExecution> {
        let mut children: Vec<_> = self
            .runtime
            .node_executions
            .iter()
            .filter(|child| {
                child
                    .parent
                    .as_ref()
                    .is_some_and(|parent| parent.parent_id == scope_id)
            })
            .collect();
        let replaced: HashSet<_> = children
            .iter()
            .filter_map(|child| self.runtime.retry_predecessors.get(&child.id))
            .collect();
        children.retain(|child| !replaced.contains(&child.id));
        children
    }

    pub(super) fn completion_parent<'a>(&self, parent: Option<&'a str>) -> CompletionParent<'a> {
        match parent {
            None => CompletionParent::Root,
            Some(id) if self.is_delegate_parent(id) => CompletionParent::Delegate(id),
            Some(id) => CompletionParent::Scope(id),
        }
    }

    pub(super) fn advance_composite(
        &self,
        scope_id: &str,
        workflow: &WorkflowDefinition,
        node: &NodeDefinition,
        completed_child: &str,
        counts: &HashMap<String, u32>,
        item_count: Option<usize>,
    ) -> Result<Option<RouteDecision>, WorkflowError> {
        let children = self.scope_children(scope_id);
        match &node.kind {
            NodeKind::Sequence(sequence) => {
                let artifact = children
                    .iter()
                    .rev()
                    .find(|child| child.node_name == completed_child)
                    .and_then(|child| child.artifact.as_ref());
                route_in_scope(workflow, sequence, completed_child, artifact, counts)
                    .map(Some)
                    .map_err(Into::into)
            }
            NodeKind::Fanout(fanout) => Ok(item_count
                .filter(|count| {
                    children.len() >= count * fanout.children.len()
                        && children
                            .iter()
                            .all(|child| child.status == NodeExecutionStatus::Succeeded)
                })
                .map(|_| RouteDecision::Completed)),
            _ => Ok(None),
        }
    }

    pub(super) fn finish_composite(
        &self,
        scope_id: &str,
        node: &NodeDefinition,
        status: NodeExecutionStatus,
        approved: bool,
        item_count: Option<usize>,
    ) -> Result<Option<(NodeExecutionStatus, Option<serde_json::Value>)>, WorkflowError> {
        let children = self.scope_children(scope_id);
        use NodeExecutionStatus as S;
        if status == S::Aborted || status == S::WaitingApproval && !approved {
            return Ok(None);
        }
        if let NodeKind::Fanout(fanout) = &node.kind {
            if item_count.is_none_or(|count| children.len() < count * fanout.children.len()) {
                return Ok(None);
            }
        }
        let status = if !approved && node.requires_approval_completion() {
            S::WaitingApproval
        } else {
            S::Succeeded
        };
        if status == S::WaitingApproval {
            return Ok(Some((status, None)));
        }
        let mut artifacts = serde_json::Map::new();
        for child in children {
            let key = match &node.kind {
                NodeKind::Sequence(sequence) => sequence
                    .children
                    .iter()
                    .find(|entry| entry.name == child.node_name)
                    .map(|entry| entry.name.clone()),
                NodeKind::Fanout(fanout) => child
                    .parent
                    .as_ref()
                    .and_then(|parent| parent.fanout_slot())
                    .and_then(|slot| fanout.artifact_key(slot)),
                _ => None,
            };
            let Some(key) = key else {
                if matches!(node.kind, NodeKind::Sequence(_)) {
                    continue;
                }
                return Err(WorkflowError::invalid_state(match node.kind {
                    NodeKind::Fanout(_) => {
                        format!("fanout child '{}' has no valid slot coordinates", child.id)
                    }
                    _ => format!("sequence child '{}' is undefined", child.id),
                }));
            };
            match &node.kind {
                NodeKind::Sequence(_) => {
                    artifacts.remove(&key);
                    if child.status == S::Succeeded {
                        if let Some(value) = &child.artifact {
                            artifacts.insert(key, value.clone());
                        }
                    }
                }
                NodeKind::Fanout(_) => {
                    artifacts.insert(
                        key,
                        child.artifact.clone().unwrap_or(serde_json::Value::Null),
                    );
                }
                _ => {
                    return Err(WorkflowError::invalid_state(format!(
                        "node '{}' is not composite",
                        node.name
                    )))
                }
            }
        }
        Ok(Some((status, Some(serde_json::Value::Object(artifacts)))))
    }
}

#[cfg(test)]
#[path = "scope_test.rs"]
mod scope_tests;
