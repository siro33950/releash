//! 統一 Node の純粋事実語彙。
//!
//! 事実は外部入力・人間の行動・実行した副作用の記録のみで構成され、遷移
//! イベント（completed 等）や導出結果は存在しない。状態は読み取り側の
//! tree fold が導出する。
//!
//! serde 形が `node_events` テーブルの永続形そのもの: `event_type()` が
//! event_type カラム、`encode_detail()` / `decode()` が detail カラムの JSON。
//! 行メタ（tree / node の同定）は [`NodeFactMeta`] としてカラム側が持つ。

use serde::{Deserialize, Serialize};

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
    /// 人間の行動: 中止の指示。
    AbortRequested,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartedFact {
    /// 実行木上の親参照。root の started のみ None。
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub parent: Option<ExecutionParentRef>,
    /// root の started のみが持つ、木の実行構成。
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub root: Option<Box<TreeRootFact>>,
}

/// 木の実行構成。root node の started に記録され、fold が木全体を導出する
/// 唯一の入力になる（定義 snapshot / worktree 参照 / 実行設定）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionTreeLaunch {
    Workflow,
    Session,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeRootFact {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub repository_root: Option<String>,
    /// workspace の同定子。terminal surface の owner 鍵になるため、呼び出し側が
    /// 指定した値を保持し、worktree_path から復元時に導出しない。
    pub workspace_identity: String,
    /// 実行木が所属する worktree の正規化済みパス。
    pub worktree_path: String,
    #[serde(with = "execution_origin_serde")]
    pub created_from: ExecutionOrigin,
    pub request: String,
    #[serde(with = "workflow_definition_snapshot_serde")]
    pub definition: WorkflowDefinition,
    #[serde(skip)]
    pub definition_resolution: Box<super::DefinitionResolution>,
    pub launched_as: ExecutionTreeLaunch,
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
                parent: None,
                root: Some(Box::new(TreeRootFact {
                    repository_root,
                    definition_resolution: Default::default(),
                    workspace_identity,
                    worktree_path,
                    created_from: ExecutionOrigin::DesktopUi,
                    request: String::new(),
                    definition: WorkflowDefinition {
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
                    },
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionAttachedFact {
    pub session_id: String,
    /// provider CLI 側の session 識別子（実世界突合の鍵）。
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub provider_session_id: Option<String>,
    /// 会話の正本（provider transcript）への参照。
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub transcript_ref: Option<String>,
    /// attach 時に初回指示の送信が受理済みか（workflow の子 node のみ真になりうる）。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub initial_instruction_admitted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionContinuationAdmittedFact {
    pub session_id: String,
    /// 続行指示の識別子。同じ識別子の再送を session 側で拒む鍵。
    pub request_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandSpawnedFact {
    pub display_command: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessExitedFact {
    /// OS の exit code。reconciliation の突合で喪失を発見した場合は None。
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub exit_code: Option<i32>,
    /// 正常終了した command の結果 summary（出力本体は store が所有）。
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub result_summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub failure_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub failure_kind: Option<NodeExecutionFailureKind>,
}

impl ProcessExitedFact {
    pub fn is_abnormal(&self) -> bool {
        self.exit_code != Some(0) || self.failure_reason.is_some() || self.failure_kind.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeFailureObservedFact {
    pub reason: String,
    pub failure_kind: NodeExecutionFailureKind,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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
            | NodeFact::AbortRequested
            | NodeFact::RestoreRequested => Self::AwaitingInstruction,
            _ => self,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentActivityObservedFact {
    pub activity: AgentSessionActivity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionNodeRenamedFact {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderSessionTitleObservedFact {
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitReceivedFact {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub request_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitRejectedFact {
    pub violations: Vec<ContractViolationRecord>,
    pub repair_attempt: u32,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub request_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StopReceivedFact {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub result_summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub token_usage: Option<TokenUsage>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactProducedFact {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub contract: Option<String>,
    pub value: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub request_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalGrantedFact {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NodeFactDecodeError {
    #[error("unknown node fact event type: {0}")]
    UnknownEventType(String),
    #[error("node fact detail does not match event type {event_type}: {reason}")]
    DetailMismatch { event_type: String, reason: String },
}

impl NodeFact {
    pub(crate) const PROCESS_EXITED_EVENT_TYPE: &'static str = "process_exited";
    pub(crate) const AGENT_ACTIVITY_OBSERVED_EVENT_TYPE: &'static str = "agent_activity_observed";
    pub(crate) const STOP_RECEIVED_EVENT_TYPE: &'static str = "stop_received";

    pub(crate) fn activity_replay_event_types() -> &'static [&'static str] {
        &[
            Self::PROCESS_EXITED_EVENT_TYPE,
            Self::AGENT_ACTIVITY_OBSERVED_EVENT_TYPE,
            Self::STOP_RECEIVED_EVENT_TYPE,
        ]
    }

    /// event_type カラムの値。語彙の正はこの列挙のみが持つ。
    pub fn event_type(&self) -> &'static str {
        match self {
            Self::Started(_) => "started",
            Self::RepositoryRootObserved(_) => "repository_root_observed",
            Self::SessionAttached(_) => "session_attached",
            Self::CommandSpawned(_) => "command_spawned",
            Self::ProcessExited(_) => Self::PROCESS_EXITED_EVENT_TYPE,
            Self::RuntimeFailureObserved(_) => "runtime_failure_observed",
            Self::AgentActivityObserved(_) => Self::AGENT_ACTIVITY_OBSERVED_EVENT_TYPE,
            Self::SessionNodeRenamed(_) => "session_node_renamed",
            Self::ProviderSessionTitleObserved(_) => "provider_session_title_observed",
            Self::SubmitReceived(_) => "submit_received",
            Self::SubmitRejected(_) => "submit_rejected",
            Self::StopReceived(_) => Self::STOP_RECEIVED_EVENT_TYPE,
            Self::DelegateResultInjected(_) => "delegate_result_injected",
            Self::SessionContinuationAdmitted(_) => "session_continuation_admitted",
            Self::ArtifactProduced(_) => "artifact_produced",
            Self::ApprovalGranted(_) => "approval_granted",
            Self::RetryRequested => "retry_requested",
            Self::ResumeRequested => "resume_requested",
            Self::AbortRequested => "abort_requested",
            Self::ArchiveRequested(_) => "archive_requested",
            Self::RestoreRequested => "restore_requested",
        }
    }
}

mod execution_origin_serde {
    use serde::{Deserialize, Deserializer, Serializer};

    use super::ExecutionOrigin;

    pub(super) fn serialize<S>(value: &ExecutionOrigin, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(value.as_public_value())
    }

    pub(super) fn deserialize<'de, D>(deserializer: D) -> Result<ExecutionOrigin, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        ExecutionOrigin::from_public_value(&value).map_err(serde::de::Error::custom)
    }
}

mod workflow_definition_snapshot_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::WorkflowDefinition;

    pub(super) fn serialize<S>(
        definition: &WorkflowDefinition,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        #[derive(Serialize)]
        struct Snapshot<'a> {
            #[serde(flatten)]
            definition: &'a WorkflowDefinition,
            entry: &'a str,
        }
        Snapshot {
            definition,
            entry: &definition.entry,
        }
        .serialize(serializer)
    }

    pub(super) fn deserialize<'de, D>(deserializer: D) -> Result<WorkflowDefinition, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Snapshot {
            entry: String,
            #[serde(flatten)]
            definition: WorkflowDefinition,
        }
        let Snapshot {
            mut definition,
            entry,
        } = Snapshot::deserialize(deserializer)?;
        definition.entry = entry;
        Ok(definition)
    }
}
