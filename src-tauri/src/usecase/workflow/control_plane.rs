use std::sync::Arc;

use crate::domain::provider_lifecycle::ScopedProviderLifecycleEvent;
use crate::domain::workflow::entities::workflow_execution::{
    ExecutionTree as DomainExecutionTree, ProviderStopRejection, TransitionOutcome,
};
use crate::domain::workflow::services::secret_masker as workflow_secret_masker;
use crate::domain::workflow::{NodeCompletionSignal, WorkflowError, WorkflowEvent};

use super::command::{
    ApprovalCommand, ResumeSessionNodeCommand, RetryNodeCommand, SubmitOutputCommand,
};
use super::output_submission as submission;
use super::runtime_driver::{self, NodeOutcome};
use super::runtime_error::WorkflowRuntimeError;
use super::runtime_snapshot::RuntimeCommitSnapshot;

pub(crate) struct WorkflowControlPlaneCommit {
    pub(crate) execution_id: String,
    pub(crate) before: DomainExecutionTree,
    pub(crate) after: DomainExecutionTree,
    pub(crate) transition_outcome: TransitionOutcome,
    pub(crate) workflow_events: Vec<WorkflowEvent>,
    pub(crate) provider_events: Vec<ScopedProviderLifecycleEvent>,
}

#[async_trait::async_trait]
pub(crate) trait WorkflowControlPlaneGateway: Send + Sync {
    fn current_timestamp(&self) -> f64;

    fn new_node_execution_id(&self) -> String;

    async fn resolve_workflow_execution_id(
        &self,
        node_execution_id: &str,
    ) -> Result<Option<String>, WorkflowError>;

    async fn load_active_execution(
        &self,
        execution_id: &str,
    ) -> Result<Option<DomainExecutionTree>, WorkflowError>;

    fn node_process_presence(
        &self,
        execution: &DomainExecutionTree,
        node_execution_id: &str,
    ) -> Result<crate::domain::workflow::NodeProcessPresence, WorkflowError>;

    fn worktree_exists(&self, worktree_path: &str) -> Result<bool, WorkflowError>;

    async fn session_conversation_exists(&self, session_id: &str) -> Result<bool, WorkflowError>;

    async fn resume_session_process(
        &self,
        execution_id: &str,
        node_execution_id: &str,
        session_id: &str,
    ) -> Result<(), WorkflowError>;

    async fn reserve_started_execution_tree(&self, tree_id: &str) -> Result<(), WorkflowError> {
        let _ = tree_id;
        Ok(())
    }

    async fn register_started_execution_tree(&self, tree_id: &str) -> Result<(), WorkflowError>;

    async fn release_started_execution_tree_reservation(
        &self,
        tree_id: &str,
    ) -> Result<(), WorkflowError> {
        let _ = tree_id;
        Ok(())
    }

    async fn release_deleted_execution_tree(&self, tree_id: &str) -> Result<(), WorkflowError> {
        let _ = tree_id;
        Ok(())
    }

    /// 対象 node への承認が既に事実として永続化されているか（approval の冪等判定）。
    async fn approval_persisted(
        &self,
        execution_id: &str,
        node_name: &str,
        node_execution_id: Option<&str>,
    ) -> Result<bool, WorkflowError>;

    fn configured_secret_values(&self) -> Vec<String>;

    fn approval_auto_approve_enabled(&self) -> bool {
        false
    }

    async fn commit_control_plane(
        &self,
        commit: WorkflowControlPlaneCommit,
    ) -> Result<RuntimeCommitSnapshot, WorkflowError>;

    async fn finish_control_plane_commit(
        &self,
        worktree_path: &str,
        snapshot: &RuntimeCommitSnapshot,
        outcome: Option<NodeOutcome>,
    ) -> Result<(), WorkflowError>;
}

#[derive(Clone)]
pub(crate) struct WorkflowControlPlaneUsecase {
    runtime: Arc<dyn WorkflowControlPlaneGateway>,
    startup: Option<Arc<super::startup::WorkflowStartupUsecase>>,
}

impl WorkflowControlPlaneUsecase {
    fn node_execution_id_source(&self) -> impl FnMut() -> String {
        let runtime = Arc::clone(&self.runtime);
        move || runtime.new_node_execution_id()
    }

    pub(crate) fn new(runtime: Arc<dyn WorkflowControlPlaneGateway>) -> Self {
        Self {
            runtime,
            startup: None,
        }
    }

    pub(crate) fn with_startup(
        mut self,
        startup: Option<Arc<super::startup::WorkflowStartupUsecase>>,
    ) -> Self {
        self.startup = startup;
        self
    }

    pub(crate) async fn recover_startup(&self) -> Result<(), WorkflowError> {
        match &self.startup {
            Some(startup) => startup.execute().await,
            None => Ok(()),
        }
    }

    pub(crate) async fn resolve_approval(
        &self,
        command: ApprovalCommand,
    ) -> Result<(), WorkflowError> {
        super::command::WorkflowRuntimeCommandPreflight.validate_approval(&command)?;
        super::command::retry_control_plane_conflicts(|| {
            self.resolve_approval_once(command.clone())
        })
        .await
    }

    async fn resolve_approval_once(&self, command: ApprovalCommand) -> Result<(), WorkflowError> {
        let current = self
            .runtime
            .load_active_execution(&command.execution_id)
            .await?
            .ok_or_else(|| {
                WorkflowError::NotFound(format!(
                    "Workflow execution not found: {}",
                    command.execution_id
                ))
            })?;
        let target = match current.resolve_approval_attempt_target(
            &command.node_name,
            command.node_execution_id.as_deref(),
        ) {
            Ok(target) => target,
            Err(error) => {
                if self
                    .runtime
                    .approval_persisted(
                        &command.execution_id,
                        &command.node_name,
                        command.node_execution_id.as_deref(),
                    )
                    .await?
                {
                    return Ok(());
                }
                return Err(error);
            }
        };
        let timestamp = self.runtime.current_timestamp();
        let event_comment = command.comment.map(|comment| {
            workflow_secret_masker::mask_sensitive_text(
                &comment,
                &self.runtime.configured_secret_values(),
            )
        });
        let mut candidate = current.clone();
        let mut new_id = self.node_execution_id_source();
        let applied =
            candidate.apply_approval(&target.node_execution_id, &mut new_id, timestamp)?;
        let mut workflow_events = vec![WorkflowEvent::ApprovalResolved {
            execution_id: candidate.id.clone(),
            node_execution_id: target.node_execution_id.clone(),
            node_name: target.node_name.clone(),
            comment: event_comment,
            timestamp,
        }];
        workflow_events.extend(applied.events);
        let outcome = runtime_driver::node_outcome_from_advance(&candidate, applied.decision)
            .map_err(runtime_error_to_workflow_error)?;
        let worktree_path = current.worktree_path.clone();
        let snapshot = self
            .runtime
            .commit_control_plane(WorkflowControlPlaneCommit {
                execution_id: command.execution_id,
                before: current,
                after: candidate,
                transition_outcome: TransitionOutcome::Applied,
                workflow_events,
                provider_events: Vec::new(),
            })
            .await?;
        self.runtime
            .finish_control_plane_commit(&worktree_path, &snapshot, Some(outcome))
            .await?;
        self.auto_approve_if_needed(&snapshot).await
    }

    async fn auto_approve_if_needed(
        &self,
        snapshot: &RuntimeCommitSnapshot,
    ) -> Result<(), WorkflowError> {
        if !self.runtime.approval_auto_approve_enabled() {
            return Ok(());
        }
        let Some(target) = snapshot
			.node_executions
			.iter()
			.find(|attempt| {
				attempt.status
					== crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecutionStatus::WaitingApproval
			})
		else {
			return Ok(());
		};
        Box::pin(self.resolve_approval(ApprovalCommand {
            execution_id: snapshot.execution_id.clone(),
            node_name: target.node_name.clone(),
            node_execution_id: Some(target.id.clone()),
            comment: None,
        }))
        .await
    }

    pub(crate) async fn submit_output(
        &self,
        command: SubmitOutputCommand,
    ) -> Result<(), WorkflowError> {
        super::command::retry_control_plane_conflicts(|| self.submit_output_once(command.clone()))
            .await
    }

    async fn submit_output_once(&self, command: SubmitOutputCommand) -> Result<(), WorkflowError> {
        submission::validate_submit_output_request(&command.node_execution_id)
            .map_err(runtime_error_to_workflow_error)?;

        let execution_id = self
            .runtime
            .resolve_workflow_execution_id(&command.node_execution_id)
            .await?
            .ok_or_else(|| {
                WorkflowError::NotFound(format!(
                    "Node execution not found: {}",
                    command.node_execution_id
                ))
            })?;

        let current = self
            .runtime
            .load_active_execution(&execution_id)
            .await?
            .ok_or_else(|| {
                WorkflowError::NotFound(format!(
                    "Active node execution not found: {}",
                    command.node_execution_id
                ))
            })?;

        let target = submission::validate_submit_target_context(
            &current,
            &execution_id,
            &command.node_execution_id,
        )
        .map_err(runtime_error_to_workflow_error)?;
        let validated_artifact = if let Some(artifact) = command.artifact {
            submission::validate_artifact_contract_for_workflow(
                current.workflow_definition()?,
                &target.node_name,
                &artifact.contract,
            )
            .map_err(runtime_error_to_workflow_error)?;
            let validated = submission::validate_submission_output_with_secrets(
                current.workflow_definition()?,
                &artifact.contract,
                artifact.value,
                &self.runtime.configured_secret_values(),
            )
            .map_err(runtime_error_to_workflow_error)?;
            Some((artifact.contract, validated))
        } else {
            None
        };
        let timestamp = self.runtime.current_timestamp();
        let mut candidate = current.clone();
        let submit_signal_applied = match candidate.record_node_completion_signal(
            &command.node_execution_id,
            NodeCompletionSignal::Submit,
            timestamp,
        ) {
            TransitionOutcome::Applied => true,
            TransitionOutcome::AlreadyApplied => false,
            _ => {
                return Err(WorkflowError::invalid_state(format!(
                    "node execution '{}' cannot accept Submit",
                    command.node_execution_id
                )))
            }
        };
        if !submit_signal_applied && validated_artifact.is_none() {
            return Ok(());
        }
        let mut events = Vec::new();
        if submit_signal_applied {
            events.push(WorkflowEvent::NodeSubmitReceived {
                execution_id: execution_id.clone(),
                node_execution_id: command.node_execution_id.clone(),
                timestamp,
            });
        }
        if let Some((contract, validated)) = validated_artifact {
            let artifact_outcome = candidate.apply_submitted_output(
                target.node_name.clone(),
                &command.node_execution_id,
                target.attempt,
                target.session_id,
                contract.clone(),
                validated.artifact.clone(),
                validated.result,
                timestamp,
            );
            match artifact_outcome {
                TransitionOutcome::Applied => {}
                TransitionOutcome::NotApplicable => {
                    return Err(WorkflowError::invalid_state(format!(
                        "node execution '{}' disappeared during Submit",
                        command.node_execution_id
                    )));
                }
                _ => {
                    return Err(WorkflowError::invalid_state(format!(
                        "node execution '{}' cannot accept Artifact in its current state",
                        command.node_execution_id
                    )));
                }
            }
            events.push(submission::artifact_produced_event(
                &execution_id,
                &command.node_execution_id,
                &target.node_name,
                contract,
                validated.artifact,
                None,
                None,
                timestamp,
            ));
        }
        let outcome =
            if submit_signal_applied || candidate.is_delegate_parent(&command.node_execution_id) {
                let mut new_id = self.node_execution_id_source();
                let (outcome, handshake_events) = apply_completion_handshake(
                    &mut candidate,
                    &command.node_execution_id,
                    &mut new_id,
                    timestamp,
                )?;
                events.extend(handshake_events);
                outcome
            } else {
                None
            };
        let worktree_path = current.worktree_path.clone();
        let snapshot = self
            .runtime
            .commit_control_plane(WorkflowControlPlaneCommit {
                execution_id,
                before: current,
                after: candidate,
                transition_outcome: TransitionOutcome::Applied,
                workflow_events: events,
                provider_events: Vec::new(),
            })
            .await?;
        self.runtime
            .finish_control_plane_commit(&worktree_path, &snapshot, outcome)
            .await?;
        self.auto_approve_if_needed(&snapshot).await
    }

    pub(crate) async fn retry_node(&self, command: RetryNodeCommand) -> Result<(), WorkflowError> {
        super::command::retry_control_plane_conflicts(|| self.retry_node_once(command.clone()))
            .await
    }

    async fn retry_node_once(&self, command: RetryNodeCommand) -> Result<(), WorkflowError> {
        let current = self
            .runtime
            .load_active_execution(&command.execution_id)
            .await?
            .ok_or_else(|| {
                WorkflowError::NotFound(format!(
                    "Workflow execution not found: {}",
                    command.execution_id
                ))
            })?;
        let node = current
            .node_execution(&command.node_execution_id)
            .ok_or_else(|| {
                WorkflowError::NotFound(format!(
                    "Node execution not found: {}",
                    command.node_execution_id
                ))
            })?;
        let presence = self
            .runtime
            .node_process_presence(&current, &command.node_execution_id)?;
        if !node.can_retry(presence) {
            return Err(WorkflowError::invalid_state(
                "Only an unfinished Command without a process can be retried",
            ));
        }
        let path = current
            .execution_worktree_path(&command.node_execution_id)
            .ok_or_else(|| WorkflowError::invalid_state("execution worktree is unavailable"))?;
        if !self.runtime.worktree_exists(path)? {
            return Err(WorkflowError::invalid_state(
                "execution worktree does not exist",
            ));
        }
        self.restart_node_attempt(current, command.execution_id, command.node_execution_id)
            .await
    }

    pub(crate) async fn resume_session_node(
        &self,
        command: ResumeSessionNodeCommand,
    ) -> Result<(), WorkflowError> {
        crate::domain::workflow::ExecutionTreeId::new(command.execution_id.clone())?;
        if command.node_execution_id.trim().is_empty() {
            return Err(WorkflowError::validation(
                "node_execution_id must not be empty",
            ));
        }
        super::command::retry_control_plane_conflicts(|| {
            self.resume_session_node_once(command.clone())
        })
        .await
    }

    async fn resume_session_node_once(
        &self,
        command: ResumeSessionNodeCommand,
    ) -> Result<(), WorkflowError> {
        let current = self
            .runtime
            .load_active_execution(&command.execution_id)
            .await?
            .ok_or_else(|| {
                WorkflowError::NotFound(format!(
                    "Workflow execution not found: {}",
                    command.execution_id
                ))
            })?;
        let node = current
            .node_execution(&command.node_execution_id)
            .ok_or_else(|| {
                WorkflowError::NotFound(format!(
                    "Node execution not found: {}",
                    command.node_execution_id
                ))
            })?;
        let presence = self
            .runtime
            .node_process_presence(&current, &command.node_execution_id)?;
        if !current.is_active() || !node.can_resume_session(presence) {
            return Err(WorkflowError::invalid_state(
                "Only an unfinished Session without a process can be resumed",
            ));
        }
        let path = current
            .execution_worktree_path(&command.node_execution_id)
            .ok_or_else(|| WorkflowError::invalid_state("execution worktree is unavailable"))?;
        let worktree_exists = self.runtime.worktree_exists(path)?;
        let conversation_exists = match node.session_id.as_deref() {
            Some(id) => self.runtime.session_conversation_exists(id).await?,
            None => false,
        };
        if node.requires_new_session_attempt(conversation_exists, worktree_exists) {
            self.restart_node_attempt(current, command.execution_id, command.node_execution_id)
                .await
        } else {
            self.runtime
                .resume_session_process(
                    &command.execution_id,
                    &command.node_execution_id,
                    node.session_id.as_deref().ok_or_else(|| {
                        WorkflowError::invalid_state("Session conversation has no owner")
                    })?,
                )
                .await
        }
    }

    async fn restart_node_attempt(
        &self,
        current: DomainExecutionTree,
        execution_id: String,
        node_execution_id: String,
    ) -> Result<(), WorkflowError> {
        let timestamp = self.runtime.current_timestamp();
        let mut candidate = current.clone();
        let restarted = candidate
            .restart_node_attempt_at(
                &node_execution_id,
                self.runtime.new_node_execution_id(),
                timestamp,
            )
            .ok_or_else(|| {
                WorkflowError::invalid_state(format!(
                    "node execution '{}' is not retryable",
                    node_execution_id
                ))
            })?;
        let events = vec![
            WorkflowEvent::NodeRetryRequested {
                execution_id: execution_id.clone(),
                node_execution_id,
                timestamp,
            },
            WorkflowEvent::NodeStarted {
                worktree: restarted.attempt.worktree.clone(),
                execution_id: execution_id.clone(),
                node_execution_id: restarted.attempt.id.clone(),
                node_name: restarted.attempt.node_name.clone(),
                kind: restarted.attempt.kind,
                attempt: restarted.attempt.attempt,
                parent: restarted.attempt.parent.clone(),
                timestamp,
            },
        ];
        let worktree_path = current.worktree_path.clone();
        let snapshot = self
            .runtime
            .commit_control_plane(WorkflowControlPlaneCommit {
                execution_id,
                before: current,
                after: candidate,
                transition_outcome: TransitionOutcome::Applied,
                workflow_events: events,
                provider_events: Vec::new(),
            })
            .await?;
        self.runtime
            .finish_control_plane_commit(
                &worktree_path,
                &snapshot,
                Some(NodeOutcome::StartNodes(
                    Box::new(snapshot.clone()),
                    vec![
                        crate::domain::workflow::entities::workflow_execution::NodeStart::Leaf(
                            restarted.leaf,
                        ),
                    ],
                )),
            )
            .await?;
        Ok(())
    }

    pub(crate) async fn record_provider_stop(
        &self,
        command: crate::usecase::provider_lifecycle::ProviderExecutionTreeStopCommand,
        lifecycle_events: Vec<ScopedProviderLifecycleEvent>,
    ) -> Result<(), WorkflowError> {
        super::command::retry_control_plane_conflicts(|| {
            self.record_provider_stop_once(command.clone(), lifecycle_events.clone())
        })
        .await
    }

    async fn record_provider_stop_once(
        &self,
        command: crate::usecase::provider_lifecycle::ProviderExecutionTreeStopCommand,
        lifecycle_events: Vec<ScopedProviderLifecycleEvent>,
    ) -> Result<(), WorkflowError> {
        let mut active = self.runtime.load_active_execution(&command.tree_id).await?;
        if active.is_none() {
            self.recover_startup().await?;
            active = self.runtime.load_active_execution(&command.tree_id).await?;
        }
        let Some(current) = active else {
            if lifecycle_events.is_empty() {
                return Ok(());
            }
            return Err(WorkflowError::NotFound(format!(
                "Workflow execution not found: {}",
                command.tree_id
            )));
        };
        let timestamp = self.runtime.current_timestamp();
        let mut candidate = current.clone();
        let (mut workflow_events, provider_stop_outcome) = match candidate.record_provider_stop(
            &command.node_execution_id,
            &command.agent_session_id,
            timestamp,
        ) {
            Ok(crate::domain::workflow::entities::workflow_execution::TransitionOutcome::Applied) => {
                (
                    vec![WorkflowEvent::NodeStopReceived {
                        execution_id: command.tree_id.clone(),
                        node_execution_id: command.node_execution_id.clone(),
                        timestamp,
                    }],
                    TransitionOutcome::Applied,
                )
            }
            Ok(crate::domain::workflow::entities::workflow_execution::TransitionOutcome::AlreadyApplied) => {
                (
                    vec![WorkflowEvent::NodeStopReceived {
                        execution_id: command.tree_id.clone(),
                        node_execution_id: command.node_execution_id.clone(),
                        timestamp,
                    }],
                    TransitionOutcome::AlreadyApplied,
                )
            }
            Ok(crate::domain::workflow::entities::workflow_execution::TransitionOutcome::NotApplicable) => {
                (Vec::new(), TransitionOutcome::NotApplicable)
            }
            Err(ProviderStopRejection::NodeExecutionNotFound) => {
                return Err(WorkflowError::invalid_state(format!(
                    "node execution '{}' is not part of workflow '{}'",
                    command.node_execution_id, command.tree_id
                )))
            }
            Err(ProviderStopRejection::SessionDoesNotOwnAttempt) => {
                return Err(WorkflowError::invalid_state(format!(
                    "AgentSession '{}' does not own node execution '{}'",
                    command.agent_session_id, command.node_execution_id
                )))
            }
            _ => {
                return Err(WorkflowError::invalid_state(format!(
                    "node execution '{}' cannot accept Provider Stop",
                    command.node_execution_id
                )))
            }
        };
        if workflow_events.is_empty() && lifecycle_events.is_empty() {
            return Ok(());
        }
        let outcome = if provider_stop_outcome == TransitionOutcome::Applied {
            let mut new_id = self.node_execution_id_source();
            let (outcome, events) = apply_completion_handshake(
                &mut candidate,
                &command.node_execution_id,
                &mut new_id,
                timestamp,
            )?;
            workflow_events.extend(events);
            outcome
        } else {
            None
        };
        let worktree_path = current.worktree_path.clone();
        let snapshot = self
            .runtime
            .commit_control_plane(WorkflowControlPlaneCommit {
                execution_id: command.tree_id,
                before: current,
                after: candidate,
                transition_outcome: provider_stop_outcome,
                workflow_events,
                provider_events: lifecycle_events,
            })
            .await?;
        self.runtime
            .finish_control_plane_commit(&worktree_path, &snapshot, outcome)
            .await?;
        self.auto_approve_if_needed(&snapshot).await
    }
}

fn apply_completion_handshake(
    execution: &mut DomainExecutionTree,
    node_execution_id: &str,
    new_id: &mut dyn FnMut() -> String,
    timestamp: f64,
) -> Result<(Option<NodeOutcome>, Vec<WorkflowEvent>), WorkflowError> {
    let applied =
        execution.apply_node_completion_handshake(node_execution_id, new_id, timestamp)?;
    let outcome = applied
        .advance
        .map(|advance| runtime_driver::node_outcome_from_advance(execution, advance))
        .transpose()
        .map_err(runtime_error_to_workflow_error)?;
    Ok((outcome, applied.events))
}

fn runtime_error_to_workflow_error(error: WorkflowRuntimeError) -> WorkflowError {
    match error {
        WorkflowRuntimeError::InvalidWorkflow(message)
        | WorkflowRuntimeError::ValidationError(message) => WorkflowError::validation(message),
        WorkflowRuntimeError::ExecutionNotFound(message)
        | WorkflowRuntimeError::SessionNotFound(message) => WorkflowError::NotFound(message),
        WorkflowRuntimeError::AlreadyActive(message)
        | WorkflowRuntimeError::InvalidState(message) => WorkflowError::InvalidState(message),
        WorkflowRuntimeError::Conflict(message) => WorkflowError::Conflict(message),
        WorkflowRuntimeError::UnauthorizedWorktree(message) => WorkflowError::validation(message),
        WorkflowRuntimeError::UnauthorizedApprovalTarget(message) => {
            WorkflowError::UnauthorizedApprovalTarget(message)
        }
        WorkflowRuntimeError::SessionStore(message)
        | WorkflowRuntimeError::AgentSession(message) => WorkflowError::external(message),
    }
}
