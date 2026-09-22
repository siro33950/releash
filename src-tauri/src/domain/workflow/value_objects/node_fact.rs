//! 統一 Node の純粋事実語彙。
//!
//! 外部入力・人間の行動・実行した副作用と、実行木の終端を記録する。
//! 終端事実は保存定義や現在の完了規則より優先される。
//!

use crate::domain::provider_lifecycle::ProviderKind;

use super::{
    ContractViolationRecord, ExecutionOrigin, ExecutionParentRef, NodeCompletion, NodeDefinition,
    NodeExecutionFailureKind, NodeKind, NodeKindName, SessionSpec, TokenUsage, WorkflowDefinition,
};

#[cfg(test)]
#[path = "node_fact_test.rs"]
mod node_fact_test;

/// node_events 行の同定カラム（tree / node / kind / attempt の絞り込み用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeFactMeta {
    pub tree_id: String,
    pub node_execution_id: String,
    /// 親（合成子インスタンス）の node_execution_id。root は None。
    pub parent_id: Option<String>,
    pub node_name: String,
    pub kind: NodeKindName,
    pub attempt: u32,
}

/// 永続化された1事実 = node_events 1行。
#[derive(Debug, Clone, PartialEq)]
pub struct NodeFactRecord {
    pub meta: NodeFactMeta,
    /// tree 内の追記順（store が払い出す）。
    pub seq: i64,
    pub timestamp_ms: i64,
    pub fact: NodeFact,
}

/// 純粋事実の語彙。
#[derive(Debug, Clone, PartialEq)]
pub enum NodeFact {
    /// 副作用: node 実行（attempt）を開始した。
    Started(StartedFact),
    RepositoryRootObserved(String),
    /// 副作用: provider session を起動して node に attach した。
    SessionAttached(SessionAttachedFact),
    /// 副作用: command プロセスを起動した。
    CommandSpawned(CommandSpawnedFact),
    /// 観測: プロセスが終了した（突合で発見した喪失も exit_code: None で表す）。
    ProcessExited(ProcessExitedFact),
    /// 観測: provider process の終了とは別に runtime が Node の失敗を確定した。
    RuntimeFailureObserved(RuntimeFailureObservedFact),
    /// 観測: AgentSession の provider 活動状態が変わった。
    AgentActivityObserved(AgentActivityObservedFact),
    /// 人間の行動: Session Node の表示名を変更した。
    SessionNodeRenamed(SessionNodeRenamedFact),
    /// 外部の観測: provider session のタイトルが変わった。
    ProviderSessionTitleObserved(ProviderSessionTitleObservedFact),
    /// 外部入力: 受理された Submit。
    SubmitReceived(SubmitReceivedFact),
    DelegateResultInjected(String),
    /// 副作用: 親 Session が delegate child の結果の続行指示を受理した。
    SessionContinuationAdmitted(SessionContinuationAdmittedFact),
    /// 副作用: Contract 違反として Submit を拒否した。
    SubmitRejected(SubmitRejectedFact),
    /// 外部入力: provider の Stop。
    StopReceived(StopReceivedFact),
    /// 外部入力の記録: Submit に添付された Artifact（参照と値のみ）。
    ArtifactProduced(ArtifactProducedFact),
    /// 人間の行動: 承認（承認主体は human のみ・却下操作は無い）。
    ApprovalGranted(ApprovalGrantedFact),
    /// 人間の行動: 再実行の指示。
    RetryRequested,
    /// 人間の行動: 再開の指示。
    ResumeRequested,
    ExecutionCompleted,
    /// 人間または起動時処理による中止。
    AbortRequested(AbortRequestedFact),
    /// 人間の行動: 木の archive（root にのみ受理される）。
    ArchiveRequested(ArchiveRequestedFact),
    /// 人間の行動: 木の restore（root にのみ受理される）。
    RestoreRequested,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArchiveRequestedFact {
    pub reason: String,
    pub archived_at: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StartedFact {
    pub worktree: Option<super::IsolatedWorktree>,
    /// 実行木上の親参照。root の started のみ None。
    pub parent: Option<ExecutionParentRef>,
    /// root の started のみが持つ、木の実行構成。
    pub root: Option<Box<TreeRootFact>>,
}

/// 木の実行構成。root node の started に記録され、fold が木全体を導出する
/// 唯一の入力になる（定義 snapshot / worktree 参照 / 実行設定）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionTreeLaunch {
    Workflow,
    Session,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TreeRootFact {
    pub repository_root: Option<String>,
    /// workspace の同定子。terminal surface の owner 鍵になるため、呼び出し側が
    /// 指定した値を保持し、worktree_path から復元時に導出しない。
    pub workspace_identity: String,
    /// 実行木が所属する worktree の正規化済みパス。
    pub worktree_path: String,
    pub created_from: ExecutionOrigin,
    pub request: String,
    pub workflow_name: String,
    pub definition: Option<WorkflowDefinition>,
    pub launched_as: ExecutionTreeLaunch,
}

impl TreeRootFact {
    pub fn without_definition(&self) -> Self {
        Self {
            repository_root: self.repository_root.clone(),
            workspace_identity: self.workspace_identity.clone(),
            worktree_path: self.worktree_path.clone(),
            created_from: self.created_from,
            request: self.request.clone(),
            workflow_name: self.workflow_name.clone(),
            definition: None,
            launched_as: self.launched_as,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionExecutionTreeRootFactsError {
    SessionId,
    WorkspaceIdentity,
    WorktreePath,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionExecutionTreeRootFacts {
    pub meta: NodeFactMeta,
    pub started: NodeFact,
    pub attached: NodeFact,
}

impl SessionExecutionTreeRootFacts {
    pub fn new(
        session_id: impl Into<String>,
        workspace_identity: impl Into<String>,
        worktree_path: impl Into<String>,
        provider: ProviderKind,
        repository_root: Option<String>,
    ) -> Result<Self, SessionExecutionTreeRootFactsError> {
        let session_id = session_id.into();
        let session_id = session_id.trim();
        if session_id.is_empty() {
            return Err(SessionExecutionTreeRootFactsError::SessionId);
        }
        let workspace_identity = workspace_identity.into();
        if workspace_identity.trim().is_empty() {
            return Err(SessionExecutionTreeRootFactsError::WorkspaceIdentity);
        }
        let worktree_path = worktree_path.into();
        if worktree_path.trim().is_empty() {
            return Err(SessionExecutionTreeRootFactsError::WorktreePath);
        }
        let node_name = "session".to_string();
        let meta = NodeFactMeta {
            tree_id: session_id.to_string(),
            node_execution_id: session_id.to_string(),
            parent_id: None,
            node_name: node_name.clone(),
            kind: NodeKindName::Session,
            attempt: 1,
        };
        Ok(Self {
            meta,
            started: NodeFact::Started(StartedFact {
                worktree: None,
                parent: None,
                root: Some(Box::new(TreeRootFact {
                    repository_root,
                    workspace_identity,
                    worktree_path,
                    created_from: ExecutionOrigin::DesktopUi,
                    request: String::new(),
                    workflow_name: node_name.clone(),
                    definition: Some(WorkflowDefinition {
                        name: node_name.clone(),
                        description: String::new(),
                        builtin: false,
                        schemas: Default::default(),
                        nodes: vec![NodeDefinition {
                            name: node_name.clone(),
                            kind: NodeKind::Session(SessionSpec {
                                provider,
                                model: None,
                                permission: None,
                                facets: Default::default(),
                            }),
                            artifact: None,
                            input: Vec::new(),
                            completion: NodeCompletion::default(),
                            worktree: None,
                        }],
                        entry: node_name,
                    }),
                    launched_as: ExecutionTreeLaunch::Session,
                })),
            }),
            attached: NodeFact::SessionAttached(SessionAttachedFact {
                session_id: session_id.to_string(),
                provider_session_id: None,
                transcript_ref: None,
                initial_instruction_admitted: false,
            }),
        })
    }

    pub fn into_facts(self) -> [(NodeFactMeta, NodeFact); 2] {
        [
            (self.meta.clone(), self.started),
            (self.meta, self.attached),
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionAttachedFact {
    pub session_id: String,
    /// provider CLI 側の session 識別子（実世界突合の鍵）。
    pub provider_session_id: Option<String>,
    /// 会話の正本（provider transcript）への参照。
    pub transcript_ref: Option<String>,
    /// attach 時に初回指示の送信が受理済みか（workflow の子 node のみ真になりうる）。
    pub initial_instruction_admitted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionContinuationAdmittedFact {
    pub session_id: String,
    /// 続行指示の識別子。同じ識別子の再送を session 側で拒む鍵。
    pub request_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpawnedFact {
    pub display_command: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessExitedFact {
    /// OS の exit code。reconciliation の突合で喪失を発見した場合は None。
    pub exit_code: Option<i32>,
    /// 正常終了した command の結果 summary（出力本体は store が所有）。
    pub result_summary: Option<String>,
    pub failure_reason: Option<String>,
    pub failure_kind: Option<NodeExecutionFailureKind>,
}

impl ProcessExitedFact {
    pub fn is_abnormal(&self) -> bool {
        self.exit_code != Some(0) || self.failure_reason.is_some() || self.failure_kind.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeFailureObservedFact {
    pub reason: String,
    pub failure_kind: NodeExecutionFailureKind,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AgentSessionActivity {
    Working,
    AwaitingAnswer,
    #[default]
    AwaitingInstruction,
}

impl AgentSessionActivity {
    pub fn after_fact(self, fact: &NodeFact) -> Self {
        match fact {
            NodeFact::AgentActivityObserved(fact) => fact.activity,
            NodeFact::ProcessExited(_)
            | NodeFact::StopReceived(_)
            | NodeFact::AbortRequested(_)
            | NodeFact::RestoreRequested => Self::AwaitingInstruction,
            _ => self,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentActivityObservedFact {
    pub activity: AgentSessionActivity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionNodeRenamedFact {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderSessionTitleObservedFact {
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmitReceivedFact {
    pub request_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmitRejectedFact {
    pub violations: Vec<ContractViolationRecord>,
    pub repair_attempt: u32,
    pub request_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StopReceivedFact {
    pub result_summary: Option<String>,
    pub token_usage: Option<TokenUsage>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArtifactProducedFact {
    pub contract: Option<String>,
    pub value: serde_json::Value,
    pub request_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalGrantedFact {
    pub comment: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AbortRequestedFact {
    pub reason: Option<String>,
}

impl NodeFact {
    pub fn terminal_state(&self) -> Option<super::RuntimeExecutionState> {
        match self {
            Self::ExecutionCompleted => Some(super::RuntimeExecutionState::Completed),
            Self::AbortRequested(_) => Some(super::RuntimeExecutionState::Aborted),
            _ => None,
        }
    }
}
