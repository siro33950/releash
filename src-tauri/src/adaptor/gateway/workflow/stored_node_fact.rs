use crate::domain::workflow::value_objects::{ArchiveRequestedFact, NodeFact, NodeFactDecodeError};
use serde::{Deserialize, Serialize};

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

impl NodeFact {
    /// detail カラムの JSON。payload を持たない事実は空 object。
    pub fn encode_detail(&self) -> Result<String, serde_json::Error> {
        match self {
            Self::Started(fact) => serde_json::to_string(fact),
            Self::RepositoryRootObserved(root) => {
                serde_json::to_string(&StoredRepositoryRootFact {
                    repository_root: root.clone(),
                })
            }
            Self::SessionAttached(fact) => serde_json::to_string(fact),
            Self::CommandSpawned(fact) => serde_json::to_string(fact),
            Self::ProcessExited(fact) => serde_json::to_string(fact),
            Self::RuntimeFailureObserved(fact) => serde_json::to_string(fact),
            Self::AgentActivityObserved(fact) => serde_json::to_string(fact),
            Self::SessionNodeRenamed(fact) => serde_json::to_string(fact),
            Self::ProviderSessionTitleObserved(fact) => serde_json::to_string(fact),
            Self::SubmitReceived(fact) => serde_json::to_string(fact),
            Self::SubmitRejected(fact) => serde_json::to_string(fact),
            Self::StopReceived(fact) => serde_json::to_string(fact),
            Self::DelegateResultInjected(child) => {
                serde_json::to_string(&serde_json::json!({"childExecutionId": child}))
            }
            Self::SessionContinuationAdmitted(fact) => serde_json::to_string(fact),
            Self::ArtifactProduced(fact) => serde_json::to_string(fact),
            Self::ApprovalGranted(fact) => serde_json::to_string(fact),
            Self::ArchiveRequested(fact) => serde_json::to_string(&StoredArchiveFact {
                reason: fact.reason.clone(),
                archived_at: fact.archived_at,
            }),
            Self::RetryRequested
            | Self::ResumeRequested
            | Self::AbortRequested
            | Self::RestoreRequested => Ok("{}".to_string()),
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
            "repository_root_observed" => {
                let fact: StoredRepositoryRootFact = parse(event_type, detail)?;
                if fact.repository_root.trim().is_empty() {
                    return Err(NodeFactDecodeError::DetailMismatch {
                        event_type: event_type.into(),
                        reason: "repositoryRoot is required".into(),
                    });
                }
                Ok(Self::RepositoryRootObserved(fact.repository_root))
            }
            "session_attached" => parse(event_type, detail).map(Self::SessionAttached),
            "command_spawned" => parse(event_type, detail).map(Self::CommandSpawned),
            Self::PROCESS_EXITED_EVENT_TYPE => parse(event_type, detail).map(Self::ProcessExited),
            "runtime_failure_observed" => {
                parse(event_type, detail).map(Self::RuntimeFailureObserved)
            }
            Self::AGENT_ACTIVITY_OBSERVED_EVENT_TYPE => {
                parse(event_type, detail).map(Self::AgentActivityObserved)
            }
            "session_node_renamed" => parse(event_type, detail).map(Self::SessionNodeRenamed),
            "provider_session_title_observed" => {
                parse(event_type, detail).map(Self::ProviderSessionTitleObserved)
            }
            "submit_received" => parse(event_type, detail).map(Self::SubmitReceived),
            "submit_rejected" => parse(event_type, detail).map(Self::SubmitRejected),
            Self::STOP_RECEIVED_EVENT_TYPE => parse(event_type, detail).map(Self::StopReceived),
            "delegate_result_injected" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Detail {
                    child_execution_id: String,
                }
                let detail: Detail = parse(event_type, detail)?;
                (!detail.child_execution_id.is_empty())
                    .then_some(Self::DelegateResultInjected(detail.child_execution_id))
                    .ok_or_else(|| NodeFactDecodeError::DetailMismatch {
                        event_type: event_type.into(),
                        reason: "childExecutionId is required".into(),
                    })
            }
            "session_continuation_admitted" => {
                parse(event_type, detail).map(Self::SessionContinuationAdmitted)
            }
            "artifact_produced" => parse(event_type, detail).map(Self::ArtifactProduced),
            "approval_granted" => parse(event_type, detail).map(Self::ApprovalGranted),
            "retry_requested" => empty(event_type, detail).map(|()| Self::RetryRequested),
            "resume_requested" => empty(event_type, detail).map(|()| Self::ResumeRequested),
            "abort_requested" => empty(event_type, detail).map(|()| Self::AbortRequested),
            "archive_requested" => {
                empty(event_type, detail)?;
                parse::<StoredArchiveFact>(event_type, detail).map(|fact| {
                    Self::ArchiveRequested(ArchiveRequestedFact {
                        reason: fact.reason,
                        archived_at: fact.archived_at,
                    })
                })
            }
            "restore_requested" => empty(event_type, detail).map(|()| Self::RestoreRequested),
            other => Err(NodeFactDecodeError::UnknownEventType(other.to_string())),
        }
    }
}

#[cfg(test)]
#[path = "stored_node_fact_test.rs"]
mod stored_node_fact_tests;
