use crate::domain::workflow::value_objects::ContractViolationRecord;
use crate::domain::workflow::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NodeFactDecodeError {
    #[error("unknown node fact event type: {0}")]
    UnknownEventType(String),
    #[error("node fact detail does not match event type {event_type}: {reason}")]
    DetailMismatch { event_type: String, reason: String },
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredArchiveFact {
    #[serde(default = "manual_archive_reason")]
    reason: String,
    archived_at: f64,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredRepositoryRootFact {
    repository_root: String,
}

fn manual_archive_reason() -> String {
    "manual".to_string()
}

pub(crate) fn terminal_event_types() -> &'static [&'static str] {
    &["execution_completed", "abort_requested"]
}

const PROCESS_EXITED_EVENT_TYPE: &str = "process_exited";
const AGENT_ACTIVITY_OBSERVED_EVENT_TYPE: &str = "agent_activity_observed";
const STOP_RECEIVED_EVENT_TYPE: &str = "stop_received";

pub(crate) fn activity_replay_event_types() -> &'static [&'static str] {
    &[
        PROCESS_EXITED_EVENT_TYPE,
        AGENT_ACTIVITY_OBSERVED_EVENT_TYPE,
        STOP_RECEIVED_EVENT_TYPE,
    ]
}

/// event_type カラムの値。語彙の正はこの列挙のみが持つ。
pub(crate) fn event_type(fact: &NodeFact) -> &'static str {
    match fact {
        NodeFact::Started(_) => "started",
        NodeFact::RepositoryRootObserved(_) => "repository_root_observed",
        NodeFact::SessionAttached(_) => "session_attached",
        NodeFact::CommandSpawned(_) => "command_spawned",
        NodeFact::ProcessExited(_) => PROCESS_EXITED_EVENT_TYPE,
        NodeFact::RuntimeFailureObserved(_) => "runtime_failure_observed",
        NodeFact::AgentActivityObserved(_) => AGENT_ACTIVITY_OBSERVED_EVENT_TYPE,
        NodeFact::SessionNodeRenamed(_) => "session_node_renamed",
        NodeFact::ProviderSessionTitleObserved(_) => "provider_session_title_observed",
        NodeFact::SubmitReceived(_) => "submit_received",
        NodeFact::SubmitRejected(_) => "submit_rejected",
        NodeFact::StopReceived(_) => STOP_RECEIVED_EVENT_TYPE,
        NodeFact::DelegateResultInjected(_) => "delegate_result_injected",
        NodeFact::SessionContinuationAdmitted(_) => "session_continuation_admitted",
        NodeFact::ArtifactProduced(_) => "artifact_produced",
        NodeFact::ApprovalGranted(_) => "approval_granted",
        NodeFact::RetryRequested => "retry_requested",
        NodeFact::ResumeRequested => "resume_requested",
        NodeFact::ExecutionCompleted => "execution_completed",
        NodeFact::AbortRequested(_) => "abort_requested",
        NodeFact::ArchiveRequested(_) => "archive_requested",
        NodeFact::RestoreRequested => "restore_requested",
    }
}

/// detail カラムの JSON。payload を持たない事実は空 object。
pub(crate) fn encode_detail(fact: &NodeFact) -> Result<String, serde_json::Error> {
    match fact {
        NodeFact::Started(fact) => serde_json::to_string(fact),
        NodeFact::RepositoryRootObserved(root) => {
            serde_json::to_string(&StoredRepositoryRootFact {
                repository_root: root.clone(),
            })
        }
        NodeFact::SessionAttached(fact) => serde_json::to_string(fact),
        NodeFact::CommandSpawned(fact) => serde_json::to_string(fact),
        NodeFact::ProcessExited(fact) => serde_json::to_string(fact),
        NodeFact::RuntimeFailureObserved(fact) => serde_json::to_string(fact),
        NodeFact::AgentActivityObserved(fact) => serde_json::to_string(fact),
        NodeFact::SessionNodeRenamed(fact) => serde_json::to_string(fact),
        NodeFact::ProviderSessionTitleObserved(fact) => serde_json::to_string(fact),
        NodeFact::SubmitReceived(fact) => serde_json::to_string(fact),
        NodeFact::SubmitRejected(fact) => serde_json::to_string(fact),
        NodeFact::StopReceived(fact) => serde_json::to_string(fact),
        NodeFact::DelegateResultInjected(child) => {
            serde_json::to_string(&serde_json::json!({"childExecutionId": child}))
        }
        NodeFact::SessionContinuationAdmitted(fact) => serde_json::to_string(fact),
        NodeFact::ArtifactProduced(fact) => serde_json::to_string(fact),
        NodeFact::ApprovalGranted(fact) => serde_json::to_string(fact),
        NodeFact::AbortRequested(fact) => serde_json::to_string(fact),
        NodeFact::ArchiveRequested(fact) => serde_json::to_string(&StoredArchiveFact {
            reason: fact.reason.clone(),
            archived_at: fact.archived_at,
        }),
        NodeFact::ExecutionCompleted
        | NodeFact::RetryRequested
        | NodeFact::ResumeRequested
        | NodeFact::RestoreRequested => Ok("{}".to_string()),
    }
}

/// (event_type, detail) からの復元。
pub(crate) fn decode(event_type: &str, detail: &str) -> Result<NodeFact, NodeFactDecodeError> {
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
        "started" => parse(event_type, detail).map(NodeFact::Started),
        "repository_root_observed" => {
            let fact: StoredRepositoryRootFact = parse(event_type, detail)?;
            if fact.repository_root.trim().is_empty() {
                return Err(NodeFactDecodeError::DetailMismatch {
                    event_type: event_type.into(),
                    reason: "repositoryRoot is required".into(),
                });
            }
            Ok(NodeFact::RepositoryRootObserved(fact.repository_root))
        }
        "session_attached" => parse(event_type, detail).map(NodeFact::SessionAttached),
        "command_spawned" => parse(event_type, detail).map(NodeFact::CommandSpawned),
        PROCESS_EXITED_EVENT_TYPE => parse(event_type, detail).map(NodeFact::ProcessExited),
        "runtime_failure_observed" => {
            parse(event_type, detail).map(NodeFact::RuntimeFailureObserved)
        }
        AGENT_ACTIVITY_OBSERVED_EVENT_TYPE => {
            parse(event_type, detail).map(NodeFact::AgentActivityObserved)
        }
        "session_node_renamed" => parse(event_type, detail).map(NodeFact::SessionNodeRenamed),
        "provider_session_title_observed" => {
            parse(event_type, detail).map(NodeFact::ProviderSessionTitleObserved)
        }
        "submit_received" => parse(event_type, detail).map(NodeFact::SubmitReceived),
        "submit_rejected" => parse(event_type, detail).map(NodeFact::SubmitRejected),
        STOP_RECEIVED_EVENT_TYPE => parse(event_type, detail).map(NodeFact::StopReceived),
        "delegate_result_injected" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Detail {
                child_execution_id: String,
            }
            let detail: Detail = parse(event_type, detail)?;
            (!detail.child_execution_id.is_empty())
                .then_some(NodeFact::DelegateResultInjected(detail.child_execution_id))
                .ok_or_else(|| NodeFactDecodeError::DetailMismatch {
                    event_type: event_type.into(),
                    reason: "childExecutionId is required".into(),
                })
        }
        "session_continuation_admitted" => {
            parse(event_type, detail).map(NodeFact::SessionContinuationAdmitted)
        }
        "artifact_produced" => parse(event_type, detail).map(NodeFact::ArtifactProduced),
        "approval_granted" => parse(event_type, detail).map(NodeFact::ApprovalGranted),
        "retry_requested" => empty(event_type, detail).map(|()| NodeFact::RetryRequested),
        "resume_requested" => empty(event_type, detail).map(|()| NodeFact::ResumeRequested),
        "execution_completed" => empty(event_type, detail).map(|()| NodeFact::ExecutionCompleted),
        "abort_requested" => {
            empty(event_type, detail)?;
            parse(event_type, detail).map(NodeFact::AbortRequested)
        }
        "archive_requested" => {
            empty(event_type, detail)?;
            parse::<StoredArchiveFact>(event_type, detail).map(|fact| {
                NodeFact::ArchiveRequested(ArchiveRequestedFact {
                    reason: fact.reason,
                    archived_at: fact.archived_at,
                })
            })
        }
        "restore_requested" => empty(event_type, detail).map(|()| NodeFact::RestoreRequested),
        other => Err(NodeFactDecodeError::UnknownEventType(other.to_string())),
    }
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "StartedFact")]
#[serde(rename_all = "camelCase")]
struct StartedFactDetail {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub worktree: Option<IsolatedWorktree>,
    /// 実行木上の親参照。root の started のみ None。
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub parent: Option<ExecutionParentRef>,
    /// root の started のみが持つ、木の実行構成。
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub root: Option<Box<TreeRootFact>>,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "ExecutionTreeLaunch")]
#[serde(rename_all = "snake_case")]
enum ExecutionTreeLaunchDetail {
    Workflow,
    Session,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "TreeRootFact")]
#[serde(rename_all = "camelCase")]
struct TreeRootFactDetail {
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
    pub definition: Option<WorkflowDefinition>,
    #[serde(default)]
    pub workflow_name: String,
    pub launched_as: ExecutionTreeLaunch,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "SessionAttachedFact")]
#[serde(rename_all = "camelCase")]
struct SessionAttachedFactDetail {
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

#[derive(Serialize, Deserialize)]
#[serde(remote = "SessionContinuationAdmittedFact")]
#[serde(rename_all = "camelCase")]
struct SessionContinuationAdmittedFactDetail {
    pub session_id: String,
    /// 続行指示の識別子。同じ識別子の再送を session 側で拒む鍵。
    pub request_id: String,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "CommandSpawnedFact")]
#[serde(rename_all = "camelCase")]
struct CommandSpawnedFactDetail {
    pub display_command: String,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "ProcessExitedFact")]
#[serde(rename_all = "camelCase")]
struct ProcessExitedFactDetail {
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

#[derive(Serialize, Deserialize)]
#[serde(remote = "RuntimeFailureObservedFact")]
#[serde(rename_all = "camelCase")]
struct RuntimeFailureObservedFactDetail {
    pub reason: String,
    pub failure_kind: NodeExecutionFailureKind,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "AgentSessionActivity")]
#[serde(rename_all = "snake_case")]
enum AgentSessionActivityDetail {
    Working,
    AwaitingAnswer,
    AwaitingInstruction,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "AgentActivityObservedFact")]
#[serde(rename_all = "camelCase")]
struct AgentActivityObservedFactDetail {
    pub activity: AgentSessionActivity,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "SessionNodeRenamedFact")]
struct SessionNodeRenamedFactDetail {
    pub name: String,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "ProviderSessionTitleObservedFact")]
struct ProviderSessionTitleObservedFactDetail {
    pub title: String,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "SubmitReceivedFact")]
#[serde(rename_all = "camelCase")]
struct SubmitReceivedFactDetail {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub request_id: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "SubmitRejectedFact")]
#[serde(rename_all = "camelCase")]
struct SubmitRejectedFactDetail {
    pub violations: Vec<ContractViolationRecord>,
    pub repair_attempt: u32,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub request_id: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "StopReceivedFact")]
#[serde(rename_all = "camelCase")]
struct StopReceivedFactDetail {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub result_summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub token_usage: Option<TokenUsage>,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "ArtifactProducedFact")]
#[serde(rename_all = "camelCase")]
struct ArtifactProducedFactDetail {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub contract: Option<String>,
    pub value: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub request_id: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "ApprovalGrantedFact")]
#[serde(rename_all = "camelCase")]
struct ApprovalGrantedFactDetail {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub comment: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "AbortRequestedFact")]
struct AbortRequestedFactDetail {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub reason: Option<String>,
}

macro_rules! payload_serde {
    ($payload:ty, $detail:ident) => {
        impl Serialize for $payload {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                $detail::serialize(self, serializer)
            }
        }
        impl<'de> Deserialize<'de> for $payload {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                $detail::deserialize(deserializer)
            }
        }
    };
}
payload_serde!(StartedFact, StartedFactDetail);
payload_serde!(ExecutionTreeLaunch, ExecutionTreeLaunchDetail);
impl Serialize for TreeRootFact {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        TreeRootFactDetail::serialize(self, serializer)
    }
}
impl<'de> Deserialize<'de> for TreeRootFact {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut root = TreeRootFactDetail::deserialize(deserializer)?;
        if root.workflow_name.is_empty() {
            root.workflow_name = root
                .definition
                .as_ref()
                .map(|definition| definition.name.clone())
                .unwrap_or_default();
        }
        Ok(root)
    }
}
payload_serde!(SessionAttachedFact, SessionAttachedFactDetail);
payload_serde!(
    SessionContinuationAdmittedFact,
    SessionContinuationAdmittedFactDetail
);
payload_serde!(CommandSpawnedFact, CommandSpawnedFactDetail);
payload_serde!(ProcessExitedFact, ProcessExitedFactDetail);
payload_serde!(RuntimeFailureObservedFact, RuntimeFailureObservedFactDetail);
payload_serde!(AgentSessionActivity, AgentSessionActivityDetail);
payload_serde!(AgentActivityObservedFact, AgentActivityObservedFactDetail);
payload_serde!(SessionNodeRenamedFact, SessionNodeRenamedFactDetail);
payload_serde!(
    ProviderSessionTitleObservedFact,
    ProviderSessionTitleObservedFactDetail
);
payload_serde!(SubmitReceivedFact, SubmitReceivedFactDetail);
payload_serde!(SubmitRejectedFact, SubmitRejectedFactDetail);
payload_serde!(StopReceivedFact, StopReceivedFactDetail);
payload_serde!(ArtifactProducedFact, ArtifactProducedFactDetail);
payload_serde!(ApprovalGrantedFact, ApprovalGrantedFactDetail);
payload_serde!(AbortRequestedFact, AbortRequestedFactDetail);
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
        definition: &Option<WorkflowDefinition>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let definition = definition.as_ref().ok_or_else(|| {
            serde::ser::Error::custom("started fact requires a workflow definition")
        })?;
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

    pub(super) fn deserialize<'de, D>(
        deserializer: D,
    ) -> Result<Option<WorkflowDefinition>, D::Error>
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
        Ok(Some(definition))
    }
}

#[cfg(test)]
#[path = "fact_codec_test.rs"]
mod fact_codec_tests;

#[derive(Serialize, Deserialize)]
#[serde(remote = "IsolatedWorktree")]
struct IsolatedWorktreeDetail {
    branch: String,
    path: String,
}
payload_serde!(IsolatedWorktree, IsolatedWorktreeDetail);
