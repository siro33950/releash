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
    /// 副作用: provider session を起動して node に attach した。
    SessionAttached(SessionAttachedFact),
    /// 副作用: command プロセスを起動した。
    CommandSpawned(CommandSpawnedFact),
    /// 観測: プロセスが終了した（突合で発見した喪失も exit_code: None で表す）。
    ProcessExited(ProcessExitedFact),
    /// 外部入力: 受理された Submit。
    SubmitReceived(SubmitReceivedFact),
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
    ArchiveRequested,
    /// 人間の行動: 木の restore（root にのみ受理される）。
    RestoreRequested,
    /// 副作用: Node attempt 用の隔離 worktree を生成した。
    IsolatedWorktreeCreated(IsolatedWorktreeCreatedFact),
    /// 副作用: Node attempt が所有する隔離 worktree を解放した。
    IsolatedWorktreeReleased,
    /// 観測: Node attempt が所有する隔離 worktree の実体を喪失した。
    IsolatedWorktreeLost,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartedFact {
    /// 実行木上の親参照。root の started のみ None。
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub parent: Option<ExecutionParentRef>,
    /// root の started のみが持つ、木の実行構成。
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub root: Option<TreeRootFact>,
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
                root: Some(TreeRootFact {
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
                            completion: NodeCompletion::Auto,
                            worktree: None,
                        }],
                        entry: node_name,
                    },
                    launched_as: ExecutionTreeLaunch::Session,
                }),
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IsolatedWorktreeCreatedFact {
    pub repository_root: String,
    pub worktree_path: String,
    pub branch: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NodeFactDecodeError {
    #[error("unknown node fact event type: {0}")]
    UnknownEventType(String),
    #[error("node fact detail does not match event type {event_type}: {reason}")]
    DetailMismatch { event_type: String, reason: String },
}

impl NodeFact {
    /// event_type カラムの値。語彙の正はこの列挙のみが持つ。
    pub fn event_type(&self) -> &'static str {
        match self {
            Self::Started(_) => "started",
            Self::SessionAttached(_) => "session_attached",
            Self::CommandSpawned(_) => "command_spawned",
            Self::ProcessExited(_) => "process_exited",
            Self::SubmitReceived(_) => "submit_received",
            Self::SubmitRejected(_) => "submit_rejected",
            Self::StopReceived(_) => "stop_received",
            Self::ArtifactProduced(_) => "artifact_produced",
            Self::ApprovalGranted(_) => "approval_granted",
            Self::RetryRequested => "retry_requested",
            Self::ResumeRequested => "resume_requested",
            Self::AbortRequested => "abort_requested",
            Self::ArchiveRequested => "archive_requested",
            Self::RestoreRequested => "restore_requested",
            Self::IsolatedWorktreeCreated(_) => "isolated_worktree_created",
            Self::IsolatedWorktreeReleased => "isolated_worktree_released",
            Self::IsolatedWorktreeLost => "isolated_worktree_lost",
        }
    }

    /// detail カラムの JSON。payload を持たない事実は空 object。
    pub fn encode_detail(&self) -> Result<String, serde_json::Error> {
        match self {
            Self::Started(fact) => serde_json::to_string(fact),
            Self::SessionAttached(fact) => serde_json::to_string(fact),
            Self::CommandSpawned(fact) => serde_json::to_string(fact),
            Self::ProcessExited(fact) => serde_json::to_string(fact),
            Self::SubmitReceived(fact) => serde_json::to_string(fact),
            Self::SubmitRejected(fact) => serde_json::to_string(fact),
            Self::StopReceived(fact) => serde_json::to_string(fact),
            Self::ArtifactProduced(fact) => serde_json::to_string(fact),
            Self::ApprovalGranted(fact) => serde_json::to_string(fact),
            Self::IsolatedWorktreeCreated(fact) => serde_json::to_string(fact),
            Self::RetryRequested
            | Self::ResumeRequested
            | Self::AbortRequested
            | Self::ArchiveRequested
            | Self::RestoreRequested
            | Self::IsolatedWorktreeReleased
            | Self::IsolatedWorktreeLost => Ok("{}".to_string()),
        }
    }

    /// (event_type, detail) からの復元。
    pub fn decode(event_type: &str, detail: &str) -> Result<Self, NodeFactDecodeError> {
        fn parse<T: serde::de::DeserializeOwned>(
            event_type: &str,
            detail: &str,
        ) -> Result<T, NodeFactDecodeError> {
            serde_json::from_str(detail).map_err(|error| NodeFactDecodeError::DetailMismatch {
                event_type: event_type.to_string(),
                reason: error.to_string(),
            })
        }

        /// payload を持たない事実の `detail` 契約は JSON object である
        /// （`encode_detail` は `{}` を書く）。object 以外は破損として拒否する。
        fn empty(event_type: &str, detail: &str) -> Result<(), NodeFactDecodeError> {
            parse::<serde_json::Map<String, serde_json::Value>>(event_type, detail).map(|_| ())
        }

        match event_type {
            "started" => parse(event_type, detail).map(Self::Started),
            "session_attached" => parse(event_type, detail).map(Self::SessionAttached),
            "command_spawned" => parse(event_type, detail).map(Self::CommandSpawned),
            "process_exited" => parse(event_type, detail).map(Self::ProcessExited),
            "submit_received" => parse(event_type, detail).map(Self::SubmitReceived),
            "submit_rejected" => parse(event_type, detail).map(Self::SubmitRejected),
            "stop_received" => parse(event_type, detail).map(Self::StopReceived),
            "artifact_produced" => parse(event_type, detail).map(Self::ArtifactProduced),
            "approval_granted" => parse(event_type, detail).map(Self::ApprovalGranted),
            "retry_requested" => empty(event_type, detail).map(|()| Self::RetryRequested),
            "resume_requested" => empty(event_type, detail).map(|()| Self::ResumeRequested),
            "abort_requested" => empty(event_type, detail).map(|()| Self::AbortRequested),
            "archive_requested" => empty(event_type, detail).map(|()| Self::ArchiveRequested),
            "restore_requested" => empty(event_type, detail).map(|()| Self::RestoreRequested),
            "isolated_worktree_created" => {
                parse(event_type, detail).map(Self::IsolatedWorktreeCreated)
            }
            "isolated_worktree_released" => {
                empty(event_type, detail).map(|()| Self::IsolatedWorktreeReleased)
            }
            "isolated_worktree_lost" => {
                empty(event_type, detail).map(|()| Self::IsolatedWorktreeLost)
            }
            other => Err(NodeFactDecodeError::UnknownEventType(other.to_string())),
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
        let mut value = serde_json::to_value(definition).map_err(serde::ser::Error::custom)?;
        let fields = value
            .as_object_mut()
            .ok_or_else(|| serde::ser::Error::custom("workflow definition must be an object"))?;
        fields.insert(
            "entry".to_string(),
            serde_json::Value::String(definition.entry.clone()),
        );
        value.serialize(serializer)
    }

    pub(super) fn deserialize<'de, D>(deserializer: D) -> Result<WorkflowDefinition, D::Error>
    where
        D: Deserializer<'de>,
    {
        let mut value = serde_json::Value::deserialize(deserializer)?;
        let fields = value
            .as_object_mut()
            .ok_or_else(|| serde::de::Error::custom("workflow definition must be an object"))?;
        let entry = fields
            .remove("entry")
            .and_then(|value| value.as_str().map(ToOwned::to_owned))
            .ok_or_else(|| serde::de::Error::custom("workflow definition entry is required"))?;
        let mut definition: WorkflowDefinition =
            serde_json::from_value(value).map_err(serde::de::Error::custom)?;
        definition.entry = entry;
        Ok(definition)
    }
}
