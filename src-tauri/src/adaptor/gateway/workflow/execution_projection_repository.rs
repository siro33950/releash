#[cfg(test)]
use std::path::PathBuf;
use std::sync::Arc;

#[cfg(test)]
use serde::Deserialize;

use crate::adaptor::gateway::workflow::event::WorkflowEvent;
#[cfg(test)]
use crate::adaptor::gateway::workflow::event::{FanoutParentRef, TokenUsage};
use crate::adaptor::gateway::workflow::log::WorkflowEventLog;
#[cfg(test)]
use crate::adaptor::gateway::workflow::schema::{NodeKindName, WorkflowDefinitionYaml};
use crate::domain::local_event::LocalEventTransactionRepository;
#[cfg(test)]
use crate::domain::workflow::{
    ExecutionInterruptionReason, ExecutionOrigin, NodeExecutionFailureKind,
};
use crate::domain::workflow::{WorkflowError, WorkflowExecution, WorkflowExecutionId};
use crate::usecase::workflow::ports::{
    WorkflowExecutionProjection, WorkflowExecutionProjectionRepository,
};

/// Payload-stripped event mirror used exclusively by Workspace tree replay.
///
/// Fields absent from this enum are intentionally consumed by Serde as unknown
/// fields. In particular, request bodies, prepared commands, Artifact values,
/// completion summaries, and diagnostic bodies are never allocated as owned
/// strings/JSON values on this path.
#[derive(Deserialize)]
#[serde(rename_all = "snake_case", tag = "event")]
#[cfg(test)]
enum WorkspaceSummaryEvent {
    ExecutionStarted {
        execution_id: String,
        workflow_name: String,
        worktree_path: String,
        created_from: String,
        definition: WorkflowDefinitionYaml,
        timestamp: f64,
    },
    NodeStarted {
        execution_id: String,
        node_execution_id: String,
        node_name: String,
        kind: NodeKindName,
        attempt: u32,
        fanout_parent: Option<FanoutParentRef>,
        timestamp: f64,
    },
    SessionAttached {
        execution_id: String,
        node_execution_id: String,
        session_id: String,
        timestamp: f64,
    },
    CommandPrepared {
        execution_id: String,
        node_execution_id: String,
        timestamp: f64,
    },
    ArtifactProduced {
        execution_id: String,
        node_execution_id: String,
        node_name: String,
        contract: Option<String>,
        timestamp: f64,
    },
    NodeCompleted {
        execution_id: String,
        node_execution_id: String,
        node_name: String,
        attempt: u32,
        timestamp: f64,
    },
    NodeFailed {
        execution_id: String,
        node_execution_id: String,
        node_name: String,
        attempt: u32,
        failure_kind: NodeExecutionFailureKind,
        timestamp: f64,
    },
    ApprovalRequested {
        execution_id: String,
        node_execution_id: String,
        node_name: String,
        timestamp: f64,
    },
    ApprovalResolved {
        execution_id: String,
        node_execution_id: String,
        node_name: String,
        timestamp: f64,
    },
    ContractViolated {
        execution_id: String,
        timestamp: f64,
    },
    StallObserved {
        execution_id: String,
        timestamp: f64,
    },
    StallCleared {
        execution_id: String,
        timestamp: f64,
    },
    ExecutionCompleted {
        execution_id: String,
        timestamp: f64,
    },
    ExecutionFailed {
        execution_id: String,
        failure_kind: NodeExecutionFailureKind,
        timestamp: f64,
    },
    ExecutionAborted {
        execution_id: String,
        timestamp: f64,
    },
    ExecutionInterrupted {
        execution_id: String,
        reason: String,
        timestamp: f64,
    },
    ExecutionResumed {
        execution_id: String,
        resume_from_node: String,
        timestamp: f64,
    },
}

#[cfg(test)]
impl WorkspaceSummaryEvent {
    fn into_canonical(self) -> Result<WorkflowEvent, String> {
        Ok(match self {
            Self::ExecutionStarted {
                execution_id,
                workflow_name,
                worktree_path,
                created_from,
                definition,
                timestamp,
            } => WorkflowEvent::ExecutionStarted {
                execution_id,
                workflow_name,
                worktree_path,
                created_from: ExecutionOrigin::from_public_value(&created_from)
                    .map_err(|error| error.to_string())?,
                request: String::new(),
                permission_mode: String::new(),
                definition,
                timestamp,
            },
            Self::NodeStarted {
                execution_id,
                node_execution_id,
                node_name,
                kind,
                attempt,
                fanout_parent,
                timestamp,
            } => WorkflowEvent::NodeStarted {
                execution_id,
                node_execution_id,
                node_name,
                kind,
                attempt,
                fanout_parent,
                timestamp,
            },
            Self::SessionAttached {
                execution_id,
                node_execution_id,
                session_id,
                timestamp,
            } => WorkflowEvent::SessionAttached {
                execution_id,
                node_execution_id,
                session_id,
                timestamp,
            },
            Self::CommandPrepared {
                execution_id,
                node_execution_id,
                timestamp,
            } => WorkflowEvent::CommandPrepared {
                execution_id,
                node_execution_id,
                display_command: String::new(),
                timestamp,
            },
            Self::ArtifactProduced {
                execution_id,
                node_execution_id,
                node_name,
                contract,
                timestamp,
            } => WorkflowEvent::ArtifactProduced {
                execution_id,
                node_execution_id,
                node_name,
                contract,
                value: serde_json::Value::Null,
                request_id: None,
                submitted_at: None,
                timestamp,
            },
            Self::NodeCompleted {
                execution_id,
                node_execution_id,
                node_name,
                attempt,
                timestamp,
            } => WorkflowEvent::NodeCompleted {
                execution_id,
                node_execution_id,
                node_name,
                attempt,
                result_summary: None,
                token_usage: None,
                timestamp,
            },
            Self::NodeFailed {
                execution_id,
                node_execution_id,
                node_name,
                attempt,
                failure_kind,
                timestamp,
            } => WorkflowEvent::NodeFailed {
                execution_id,
                node_execution_id,
                node_name,
                attempt,
                reason: String::new(),
                failure_kind,
                retry_count: None,
                timestamp,
            },
            Self::ApprovalRequested {
                execution_id,
                node_execution_id,
                node_name,
                timestamp,
            } => WorkflowEvent::ApprovalRequested {
                execution_id,
                node_execution_id,
                node_name,
                timestamp,
            },
            Self::ApprovalResolved {
                execution_id,
                node_execution_id,
                node_name,
                timestamp,
            } => WorkflowEvent::ApprovalResolved {
                execution_id,
                node_execution_id,
                node_name,
                comment: None,
                timestamp,
            },
            Self::ContractViolated {
                execution_id,
                timestamp,
            } => WorkflowEvent::ContractViolated {
                execution_id,
                node_execution_id: String::new(),
                node_name: String::new(),
                violations: Vec::new(),
                repair_attempt: 0,
                request_id: None,
                timestamp,
            },
            Self::StallObserved {
                execution_id,
                timestamp,
            } => WorkflowEvent::StallObserved {
                execution_id,
                node_execution_id: String::new(),
                node_name: String::new(),
                attempt: 0,
                session_id: String::new(),
                turn_phase: String::new(),
                idle_secs: 0,
                signal_count: 0,
                cap_reached: false,
                timestamp,
            },
            Self::StallCleared {
                execution_id,
                timestamp,
            } => WorkflowEvent::StallCleared {
                execution_id,
                node_execution_id: String::new(),
                session_id: String::new(),
                timestamp,
            },
            Self::ExecutionCompleted {
                execution_id,
                timestamp,
            } => WorkflowEvent::ExecutionCompleted {
                execution_id,
                total_token_usage: TokenUsage::default(),
                timestamp,
            },
            Self::ExecutionFailed {
                execution_id,
                failure_kind,
                timestamp,
            } => WorkflowEvent::ExecutionFailed {
                execution_id,
                reason: String::new(),
                failure_kind,
                timestamp,
            },
            Self::ExecutionAborted {
                execution_id,
                timestamp,
            } => WorkflowEvent::ExecutionAborted {
                execution_id,
                aborted_node: None,
                timestamp,
            },
            Self::ExecutionInterrupted {
                execution_id,
                reason,
                timestamp,
            } => WorkflowEvent::ExecutionInterrupted {
                execution_id,
                reason: ExecutionInterruptionReason::from_reason(&reason)
                    .ok_or_else(|| format!("unknown execution interruption reason: {reason}"))?,
                timestamp,
            },
            Self::ExecutionResumed {
                execution_id,
                resume_from_node,
                timestamp,
            } => WorkflowEvent::ExecutionResumed {
                execution_id,
                resume_from_node,
                timestamp,
            },
        })
    }
}

#[derive(Clone)]
pub(crate) struct WorkflowExecutionProjectionLogRepository {
    source: WorkflowProjectionReadSource,
}

#[derive(Clone)]
enum WorkflowProjectionReadSource {
    #[cfg(test)]
    Legacy(PathBuf),
    Canonical {
        repository: Arc<dyn LocalEventTransactionRepository>,
        installation_id: String,
    },
}

impl WorkflowExecutionProjectionLogRepository {
    #[cfg(test)]
    pub(crate) fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            source: WorkflowProjectionReadSource::Legacy(data_dir.into()),
        }
    }

    pub(crate) fn with_authority(
        repository: Arc<dyn LocalEventTransactionRepository>,
        installation_id: String,
    ) -> Self {
        Self {
            source: WorkflowProjectionReadSource::Canonical {
                repository,
                installation_id,
            },
        }
    }

    fn log(&self) -> WorkflowEventLog {
        match &self.source {
            #[cfg(test)]
            WorkflowProjectionReadSource::Legacy(data_dir) => WorkflowEventLog::new(data_dir),
            WorkflowProjectionReadSource::Canonical {
                repository,
                installation_id,
            } => WorkflowEventLog::with_authority(repository.clone(), installation_id.clone()),
        }
    }

    fn read_events(
        &self,
        execution_id: &WorkflowExecutionId,
    ) -> Result<Vec<WorkflowEvent>, String> {
        match &self.source {
            #[cfg(test)]
            WorkflowProjectionReadSource::Legacy(_) => self.log().read_log(execution_id.as_str()),
            _ => self.log().read_log_durable_blocking(execution_id.as_str()),
        }
    }

    fn read_workspace_events(
        &self,
        execution_id: &WorkflowExecutionId,
    ) -> Result<Vec<WorkflowEvent>, String> {
        match &self.source {
            #[cfg(test)]
            WorkflowProjectionReadSource::Legacy(_) => self.log().read_log_mapped(
                execution_id.as_str(),
                |line| {
                    serde_json::from_str::<WorkspaceSummaryEvent>(line)
                        .map_err(|error| error.to_string())?
                        .into_canonical()
                },
                WorkflowEvent::execution_id,
            ),
            _ => self.log().read_log_durable_blocking(execution_id.as_str()),
        }
    }
}

impl WorkflowExecutionProjectionRepository for WorkflowExecutionProjectionLogRepository {
    fn get_execution(
        &self,
        execution_id: &WorkflowExecutionId,
    ) -> Result<Option<WorkflowExecution>, WorkflowError> {
        self.get_execution_with_definition(execution_id)
            .map(|projection| projection.map(|projection| projection.execution))
    }

    fn get_execution_with_definition(
        &self,
        execution_id: &WorkflowExecutionId,
    ) -> Result<Option<WorkflowExecutionProjection>, WorkflowError> {
        let events = self
            .read_events(execution_id)
            .map_err(WorkflowError::external)?;
        let definition = events.iter().find_map(|event| match event {
            WorkflowEvent::ExecutionStarted { definition, .. } => Some(
                super::domain_mapping::workflow_definition_to_domain(definition),
            ),
            _ => None,
        });
        crate::domain::workflow::services::event_replay::project_workflow_execution(
            execution_id.as_str(),
            &events,
        )
        .map(|execution| {
            execution.map(|execution| WorkflowExecutionProjection {
                execution,
                definition,
            })
        })
        .map_err(WorkflowError::external)
    }

    fn get_workspace_execution_with_definition(
        &self,
        execution_id: &WorkflowExecutionId,
    ) -> Result<Option<WorkflowExecutionProjection>, WorkflowError> {
        let events = self
            .read_workspace_events(execution_id)
            .map_err(WorkflowError::external)?;
        let definition = events.iter().find_map(|event| match event {
            WorkflowEvent::ExecutionStarted { definition, .. } => Some(
                super::domain_mapping::workflow_definition_to_domain(definition),
            ),
            _ => None,
        });
        crate::domain::workflow::services::event_replay::project_payload_stripped_workflow_execution(
            execution_id.as_str(),
            &events,
        )
        .map(|execution| {
            execution.map(|execution| WorkflowExecutionProjection {
                execution,
                definition,
            })
        })
        .map_err(WorkflowError::external)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::adaptor::gateway::workflow::event::WorkflowEvent;
    use crate::adaptor::gateway::workflow::schema::{
        CommandSpec, NodeDefinition, NodeKind, Rule, WorkflowDefinitionYaml,
    };
    use crate::domain::workflow::{ExecutionOrigin, ExecutionStatus};

    #[test]
    fn projects_persisted_events_to_the_canonical_read_model() {
        let temp = tempfile::tempdir().unwrap();
        let execution_id =
            WorkflowExecutionId::new("00000000-0000-4000-8000-000000000301").unwrap();
        WorkflowEventLog::new(temp.path())
            .append(&WorkflowEvent::ExecutionStarted {
                execution_id: execution_id.to_string(),
                workflow_name: "review".to_string(),
                worktree_path: "/repo".to_string(),
                created_from: ExecutionOrigin::Cli,
                request: "review this".to_string(),
                permission_mode: "ask".to_string(),
                definition: WorkflowDefinitionYaml {
                    name: "review".to_string(),
                    description: String::new(),
                    builtin: false,
                    schemas: Default::default(),
                    nodes: Vec::new(),
                },
                timestamp: 10.0,
            })
            .unwrap();

        let projection = WorkflowExecutionProjectionLogRepository::new(temp.path())
            .get_execution_with_definition(&execution_id)
            .unwrap()
            .unwrap();
        let execution = projection.execution;

        assert_eq!(execution.id, execution_id.to_string());
        assert_eq!(execution.workflow_name, "review");
        assert_eq!(execution.started_at, 10.0);
        assert_eq!(execution.artifacts[0].node_name, "request");
        assert_eq!(projection.definition.unwrap().name, "review");
    }

    #[test]
    fn workspace_projection_streams_without_retaining_request_command_or_artifact_bodies() {
        const REQUEST_SENTINEL: &str = "RAW_REQUEST_BODY_SENTINEL_1454";
        const COMMAND_SENTINEL: &str = "MASKED_COMMAND_BODY_SENTINEL_1454";
        const OUTPUT_SENTINEL: &str = "COMMAND_OUTPUT_BODY_SENTINEL_1454";

        let temp = tempfile::tempdir().unwrap();
        let execution_id =
            WorkflowExecutionId::new("00000000-0000-4000-8000-000000000302").unwrap();
        let node_execution_id = "00000000-0000-4000-8000-000000000303";
        let definition = WorkflowDefinitionYaml {
            name: "command-review".to_string(),
            description: String::new(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![NodeDefinition {
                name: "check".to_string(),
                kind: NodeKind::Command(CommandSpec {
                    command: "printf static-command-definition".to_string(),
                }),
                ..NodeDefinition::default()
            }],
        };
        WorkflowEventLog::new(temp.path())
            .append_batch(&[
                WorkflowEvent::ExecutionStarted {
                    execution_id: execution_id.to_string(),
                    workflow_name: definition.name.clone(),
                    worktree_path: "/repo".to_string(),
                    created_from: ExecutionOrigin::Cli,
                    request: REQUEST_SENTINEL.to_string(),
                    permission_mode: "ask".to_string(),
                    definition,
                    timestamp: 10.0,
                },
                WorkflowEvent::NodeStarted {
                    execution_id: execution_id.to_string(),
                    node_execution_id: node_execution_id.to_string(),
                    node_name: "check".to_string(),
                    kind: NodeKindName::Command,
                    attempt: 1,
                    fanout_parent: None,
                    timestamp: 11.0,
                },
                WorkflowEvent::CommandPrepared {
                    execution_id: execution_id.to_string(),
                    node_execution_id: node_execution_id.to_string(),
                    display_command: COMMAND_SENTINEL.to_string(),
                    timestamp: 12.0,
                },
                WorkflowEvent::ArtifactProduced {
                    execution_id: execution_id.to_string(),
                    node_execution_id: node_execution_id.to_string(),
                    node_name: "check".to_string(),
                    contract: None,
                    value: serde_json::json!({
                        "exit_code": 0,
                        "duration": 1,
                        "stdout": OUTPUT_SENTINEL,
                        "stderr": ""
                    }),
                    request_id: None,
                    submitted_at: None,
                    timestamp: 13.0,
                },
                WorkflowEvent::NodeCompleted {
                    execution_id: execution_id.to_string(),
                    node_execution_id: node_execution_id.to_string(),
                    node_name: "check".to_string(),
                    attempt: 1,
                    result_summary: Some("exit_code=0".to_string()),
                    token_usage: None,
                    timestamp: 14.0,
                },
            ])
            .unwrap();

        let repository = WorkflowExecutionProjectionLogRepository::new(temp.path());
        let full = repository
            .get_execution_with_definition(&execution_id)
            .unwrap()
            .unwrap();
        let full_debug = format!("{full:?}");
        assert!(full_debug.contains(REQUEST_SENTINEL));
        assert!(full_debug.contains(COMMAND_SENTINEL));
        assert!(full_debug.contains(OUTPUT_SENTINEL));

        let workspace = repository
            .get_workspace_execution_with_definition(&execution_id)
            .unwrap()
            .unwrap();
        let workspace_debug = format!("{workspace:?}");
        assert!(!workspace_debug.contains(REQUEST_SENTINEL));
        assert!(!workspace_debug.contains(COMMAND_SENTINEL));
        assert!(!workspace_debug.contains(OUTPUT_SENTINEL));
        let node = &workspace.execution.node_executions[0];
        assert_eq!(node.display_command.as_deref(), Some(""));
        assert_eq!(
            node.artifact.as_ref().map(|artifact| &artifact.value),
            Some(&serde_json::Value::Null)
        );
    }

    #[test]
    fn full_and_workspace_projections_preserve_the_same_node_started_order() {
        let temp = tempfile::tempdir().unwrap();
        let execution_id =
            WorkflowExecutionId::new("00000000-0000-4000-8000-000000000307").unwrap();
        let definition = WorkflowDefinitionYaml {
            name: "loop-order".to_string(),
            description: String::new(),
            builtin: false,
            schemas: Default::default(),
            nodes: Vec::new(),
        };
        let node_started =
            |node_execution_id: &str, node_name: &str, kind: NodeKindName, attempt: u32| {
                WorkflowEvent::NodeStarted {
                    execution_id: execution_id.to_string(),
                    node_execution_id: node_execution_id.to_string(),
                    node_name: node_name.to_string(),
                    kind,
                    attempt,
                    fanout_parent: None,
                    timestamp: 11.0,
                }
            };
        WorkflowEventLog::new(temp.path())
            .append_batch(&[
                WorkflowEvent::ExecutionStarted {
                    execution_id: execution_id.to_string(),
                    workflow_name: definition.name.clone(),
                    worktree_path: "/repo".to_string(),
                    created_from: ExecutionOrigin::Cli,
                    request: String::new(),
                    permission_mode: "ask".to_string(),
                    definition,
                    timestamp: 10.0,
                },
                node_started("a-1", "A", NodeKindName::Session, 1),
                node_started("b-1", "B", NodeKindName::Command, 1),
                node_started("a-2", "A", NodeKindName::Session, 2),
                node_started("c-1", "C", NodeKindName::Command, 1),
            ])
            .unwrap();

        let repository = WorkflowExecutionProjectionLogRepository::new(temp.path());
        let full = repository
            .get_execution_with_definition(&execution_id)
            .unwrap()
            .unwrap();
        let workspace = repository
            .get_workspace_execution_with_definition(&execution_id)
            .unwrap()
            .unwrap();
        let order = |projection: &WorkflowExecutionProjection| {
            projection
                .execution
                .node_executions
                .iter()
                .map(|node| (node.node_name.clone(), node.id.clone()))
                .collect::<Vec<_>>()
        };
        let expected = vec![
            ("A".to_string(), "a-1".to_string()),
            ("B".to_string(), "b-1".to_string()),
            ("A".to_string(), "a-2".to_string()),
            ("C".to_string(), "c-1".to_string()),
        ];
        assert_eq!(order(&full), expected);
        assert_eq!(order(&workspace), expected);
    }

    #[test]
    fn workspace_projection_replays_resume_without_artifact_dependent_checkpoint() {
        let temp = tempfile::tempdir().unwrap();
        let execution_id =
            WorkflowExecutionId::new("00000000-0000-4000-8000-000000000304").unwrap();
        let prepare_id = "00000000-0000-4000-8000-000000000305";
        let review_id = "00000000-0000-4000-8000-000000000306";
        let definition = WorkflowDefinitionYaml {
            name: "conditional-resume".to_string(),
            description: String::new(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![
                NodeDefinition {
                    name: "prepare".to_string(),
                    kind: NodeKind::default(),
                    rules: vec![Rule::When {
                        on: "approved".to_string(),
                        then: "review".to_string(),
                        next: "fix".to_string(),
                    }],
                    ..NodeDefinition::default()
                },
                NodeDefinition {
                    name: "review".to_string(),
                    kind: NodeKind::default(),
                    ..NodeDefinition::default()
                },
                NodeDefinition {
                    name: "fix".to_string(),
                    kind: NodeKind::default(),
                    ..NodeDefinition::default()
                },
            ],
        };
        WorkflowEventLog::new(temp.path())
            .append_batch(&[
                WorkflowEvent::ExecutionStarted {
                    execution_id: execution_id.to_string(),
                    workflow_name: definition.name.clone(),
                    worktree_path: "/repo".to_string(),
                    created_from: ExecutionOrigin::Cli,
                    request: "review".to_string(),
                    permission_mode: "ask".to_string(),
                    definition,
                    timestamp: 10.0,
                },
                WorkflowEvent::NodeStarted {
                    execution_id: execution_id.to_string(),
                    node_execution_id: prepare_id.to_string(),
                    node_name: "prepare".to_string(),
                    kind: NodeKindName::Session,
                    attempt: 1,
                    fanout_parent: None,
                    timestamp: 11.0,
                },
                WorkflowEvent::ArtifactProduced {
                    execution_id: execution_id.to_string(),
                    node_execution_id: prepare_id.to_string(),
                    node_name: "prepare".to_string(),
                    contract: None,
                    value: serde_json::json!({"approved": true}),
                    request_id: None,
                    submitted_at: None,
                    timestamp: 12.0,
                },
                WorkflowEvent::NodeCompleted {
                    execution_id: execution_id.to_string(),
                    node_execution_id: prepare_id.to_string(),
                    node_name: "prepare".to_string(),
                    attempt: 1,
                    result_summary: None,
                    token_usage: None,
                    timestamp: 13.0,
                },
                WorkflowEvent::ExecutionInterrupted {
                    execution_id: execution_id.to_string(),
                    reason: ExecutionInterruptionReason::Crash,
                    timestamp: 14.0,
                },
                WorkflowEvent::ExecutionResumed {
                    execution_id: execution_id.to_string(),
                    resume_from_node: "review".to_string(),
                    timestamp: 15.0,
                },
                WorkflowEvent::NodeStarted {
                    execution_id: execution_id.to_string(),
                    node_execution_id: review_id.to_string(),
                    node_name: "review".to_string(),
                    kind: NodeKindName::Session,
                    attempt: 1,
                    fanout_parent: None,
                    timestamp: 16.0,
                },
            ])
            .unwrap();

        let execution = WorkflowExecutionProjectionLogRepository::new(temp.path())
            .get_workspace_execution_with_definition(&execution_id)
            .unwrap()
            .unwrap()
            .execution;

        assert_eq!(execution.status, ExecutionStatus::Running);
        assert_eq!(execution.current_node.as_deref(), Some("review"));
        assert_eq!(execution.resume_from_node, None);
        assert_eq!(execution.node_executions.len(), 2);
    }
}
