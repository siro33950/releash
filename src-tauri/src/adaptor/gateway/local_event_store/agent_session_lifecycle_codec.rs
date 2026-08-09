use serde::{Deserialize, Serialize};

use crate::adaptor::gateway::local_event_store::canonical_cbor::CborValue;
use crate::adaptor::gateway::local_event_store::envelope::{
    EventCodecError, LocalEventPayloadCodec,
};
use crate::domain::agent_session::aggregates::{
    AgentSession, AgentSessionLifecycle, AgentSessionLifecycleEvent, AgentSessionOrigin,
    AgentSessionRemovalAuthorization,
};
use crate::domain::local_event::LocalDomainEvent;
use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::workspace_tree::WorkspaceIdentity;

pub(crate) const AGENT_SESSION_LIFECYCLE_EVENT_TYPE: &str = "agent_session.lifecycle";
pub(crate) const AGENT_SESSION_LIFECYCLE_PAYLOAD_VERSION: i64 = 1;

pub(crate) struct AgentSessionLifecycleEventCodec;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
enum StoredAgentSessionLifecycleEventV1 {
    Created {
        id: String,
        workspace: String,
        worktree_path: String,
        provider: String,
        origin: String,
        workflow_execution_id: Option<String>,
        node_execution_id: Option<String>,
    },
    ProviderSessionAssociated {
        provider_session_id: String,
        transcript_ref: Option<String>,
    },
    LifecycleChanged {
        lifecycle: String,
        #[serde(default)]
        last_exit_abnormal: bool,
    },
    InitialInstructionAdmitted,
    Tombstoned {
        reason: String,
    },
}

fn malformed() -> EventCodecError {
    EventCodecError::MalformedPayload {
        event_type: AGENT_SESSION_LIFECYCLE_EVENT_TYPE.to_string(),
    }
}

fn provider_label(provider: ProviderKind) -> &'static str {
    match provider {
        ProviderKind::Claude => "claude",
        ProviderKind::Codex => "codex",
    }
}

fn parse_provider(provider: &str) -> Result<ProviderKind, EventCodecError> {
    match provider {
        "claude" => Ok(ProviderKind::Claude),
        "codex" => Ok(ProviderKind::Codex),
        _ => Err(malformed()),
    }
}

fn lifecycle_label(lifecycle: AgentSessionLifecycle) -> &'static str {
    match lifecycle {
        AgentSessionLifecycle::Open => "open",
        AgentSessionLifecycle::Paused => "paused",
        AgentSessionLifecycle::Archived => "archived",
    }
}

fn parse_lifecycle(lifecycle: &str) -> Result<AgentSessionLifecycle, EventCodecError> {
    match lifecycle {
        "open" => Ok(AgentSessionLifecycle::Open),
        "paused" => Ok(AgentSessionLifecycle::Paused),
        "archived" => Ok(AgentSessionLifecycle::Archived),
        _ => Err(malformed()),
    }
}

fn removal_label(reason: AgentSessionRemovalAuthorization) -> &'static str {
    match reason {
        AgentSessionRemovalAuthorization::ArchiveFallbackDelete => "archive_fallback_delete",
        AgentSessionRemovalAuthorization::ExplicitDelete => "explicit_delete",
        AgentSessionRemovalAuthorization::GarbageCollection => "garbage_collection",
        AgentSessionRemovalAuthorization::WorkflowLaunchRollback => "workflow_launch_rollback",
    }
}

fn parse_removal(reason: &str) -> Result<AgentSessionRemovalAuthorization, EventCodecError> {
    match reason {
        "archive_fallback_delete" => Ok(AgentSessionRemovalAuthorization::ArchiveFallbackDelete),
        "explicit_delete" => Ok(AgentSessionRemovalAuthorization::ExplicitDelete),
        "garbage_collection" => Ok(AgentSessionRemovalAuthorization::GarbageCollection),
        "workflow_launch_rollback" => Ok(AgentSessionRemovalAuthorization::WorkflowLaunchRollback),
        _ => Err(malformed()),
    }
}

fn stored_event(event: &AgentSessionLifecycleEvent) -> StoredAgentSessionLifecycleEventV1 {
    match event {
        AgentSessionLifecycleEvent::Created {
            id,
            workspace,
            worktree_path,
            provider,
            origin,
        } => StoredAgentSessionLifecycleEventV1::Created {
            id: id.clone(),
            workspace: workspace.as_str().to_string(),
            worktree_path: worktree_path.clone(),
            provider: provider_label(*provider).to_string(),
            origin: if origin.is_standalone() {
                "standalone".to_string()
            } else {
                "workflow_node".to_string()
            },
            workflow_execution_id: origin.workflow_execution_id().map(str::to_string),
            node_execution_id: origin.node_execution_id().map(str::to_string),
        },
        AgentSessionLifecycleEvent::ProviderSessionAssociated {
            provider_session_id,
            transcript_ref,
        } => StoredAgentSessionLifecycleEventV1::ProviderSessionAssociated {
            provider_session_id: provider_session_id.clone(),
            transcript_ref: transcript_ref.clone(),
        },
        AgentSessionLifecycleEvent::LifecycleChanged {
            lifecycle,
            last_exit_abnormal,
        } => StoredAgentSessionLifecycleEventV1::LifecycleChanged {
            lifecycle: lifecycle_label(*lifecycle).to_string(),
            last_exit_abnormal: *last_exit_abnormal,
        },
        AgentSessionLifecycleEvent::InitialInstructionAdmitted => {
            StoredAgentSessionLifecycleEventV1::InitialInstructionAdmitted
        }
        AgentSessionLifecycleEvent::Tombstoned { reason } => {
            StoredAgentSessionLifecycleEventV1::Tombstoned {
                reason: removal_label(*reason).to_string(),
            }
        }
    }
}

fn domain_event(
    event: StoredAgentSessionLifecycleEventV1,
) -> Result<AgentSessionLifecycleEvent, EventCodecError> {
    match event {
        StoredAgentSessionLifecycleEventV1::Created {
            id,
            workspace,
            worktree_path,
            provider,
            origin,
            workflow_execution_id,
            node_execution_id,
        } => {
            let origin = match (origin.as_str(), workflow_execution_id, node_execution_id) {
                ("standalone", None, None) => AgentSessionOrigin::Standalone,
                ("workflow_node", Some(workflow_execution_id), Some(node_execution_id)) => {
                    AgentSessionOrigin::workflow_node(workflow_execution_id, node_execution_id)
                        .map_err(|_| malformed())?
                }
                _ => return Err(malformed()),
            };
            let session = AgentSession::create(
                id,
                WorkspaceIdentity::new(workspace),
                worktree_path,
                parse_provider(&provider)?,
                origin,
            )
            .map_err(|_| malformed())?;
            session
                .uncommitted_events()
                .first()
                .cloned()
                .ok_or_else(malformed)
        }
        StoredAgentSessionLifecycleEventV1::ProviderSessionAssociated {
            provider_session_id,
            transcript_ref,
        } => {
            let mut session = AgentSession::create(
                "codec-validation",
                WorkspaceIdentity::new("/codec-validation"),
                "/codec-validation",
                ProviderKind::Codex,
                AgentSessionOrigin::Standalone,
            )
            .map_err(|_| malformed())?;
            session.take_uncommitted_events();
            session
                .associate_provider_session(provider_session_id, transcript_ref.as_deref())
                .map_err(|_| malformed())?;
            session
                .take_uncommitted_events()
                .into_iter()
                .next()
                .ok_or_else(malformed)
        }
        StoredAgentSessionLifecycleEventV1::LifecycleChanged {
            lifecycle,
            last_exit_abnormal,
        } => Ok(AgentSessionLifecycleEvent::LifecycleChanged {
            lifecycle: parse_lifecycle(&lifecycle)?,
            last_exit_abnormal,
        }),
        StoredAgentSessionLifecycleEventV1::InitialInstructionAdmitted => {
            Ok(AgentSessionLifecycleEvent::InitialInstructionAdmitted)
        }
        StoredAgentSessionLifecycleEventV1::Tombstoned { reason } => {
            Ok(AgentSessionLifecycleEvent::Tombstoned {
                reason: parse_removal(&reason)?,
            })
        }
    }
}

impl LocalEventPayloadCodec for AgentSessionLifecycleEventCodec {
    fn event_type(&self) -> &'static str {
        AGENT_SESSION_LIFECYCLE_EVENT_TYPE
    }

    fn payload_version(&self) -> i64 {
        AGENT_SESSION_LIFECYCLE_PAYLOAD_VERSION
    }

    fn handles(&self, event: &LocalDomainEvent) -> bool {
        matches!(event, LocalDomainEvent::AgentSessionLifecycle(_))
    }

    fn encode(&self, event: &LocalDomainEvent) -> Result<CborValue, EventCodecError> {
        let LocalDomainEvent::AgentSessionLifecycle(event) = event else {
            return Err(malformed());
        };
        serde_json::to_string(&stored_event(event))
            .map(CborValue::Text)
            .map_err(|_| malformed())
    }

    fn decode(
        &self,
        payload_version: i64,
        value: &CborValue,
    ) -> Result<Option<LocalDomainEvent>, EventCodecError> {
        if payload_version != AGENT_SESSION_LIFECYCLE_PAYLOAD_VERSION {
            return Ok(None);
        }
        let CborValue::Text(raw) = value else {
            return Err(malformed());
        };
        let stored = serde_json::from_str(raw).map_err(|_| malformed())?;
        domain_event(stored).map(|event| Some(LocalDomainEvent::AgentSessionLifecycle(event)))
    }
}

#[cfg(test)]
#[path = "agent_session_lifecycle_codec_test.rs"]
mod agent_session_lifecycle_codec_tests;
