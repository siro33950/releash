use super::*;
use crate::domain::workflow::{NodeProcessPresence, NodeProcessReader};
use crate::usecase::workflow::node_startup::NodeStartupGateway;

pub(super) struct HostNodeStartup<'a> {
    pub host: &'a WorkflowRuntimeHost,
    pub app: &'a WorkflowRuntimeDependencies,
    pub execution_id: &'a str,
    pub worktree_path: &'a str,
    pub cancelled: tokio::sync::watch::Receiver<bool>,
}

pub(super) struct NodeStartupTask {
    pub execution_id: String,
    pub cancel: tokio::sync::watch::Sender<bool>,
    pub handle: tokio::task::JoinHandle<()>,
}

#[async_trait::async_trait]
impl NodeStartupGateway for HostNodeStartup<'_> {
    async fn start(&self, starts: Vec<NodeStart>) -> Result<Vec<String>, WorkflowRuntimeError> {
        if *self.cancelled.borrow() {
            return Ok(Vec::new());
        }
        self.host
            .start_nodes_once(self.app, self.execution_id, self.worktree_path, starts)
            .await
    }

    async fn restart(
        &self,
        node_execution_id: &str,
    ) -> Result<Option<NodeStart>, WorkflowRuntimeError> {
        if *self.cancelled.borrow() {
            return Ok(None);
        }
        self.host
            .restart_node_attempt(self.app, self.execution_id, node_execution_id)
            .await
    }

    async fn wait(&self, duration: std::time::Duration) -> bool {
        let mut cancelled = self.cancelled.clone();
        if *cancelled.borrow() {
            return false;
        }
        tokio::select! {
            biased;
            _ = cancelled.changed() => false,
            _ = tokio::time::sleep(duration) => true,
        }
    }
}

impl WorkflowRuntimeHost {
    pub(super) fn schedule_startup_retries<'a>(
        &'a self,
        app: &'a WorkflowRuntimeDependencies,
        execution_id: &'a str,
        worktree_path: &'a str,
        failed: Vec<String>,
    ) -> futures_util::future::BoxFuture<'a, ()> {
        Box::pin(async move {
            if failed.is_empty() {
                return;
            }
            let admission = self.command_admission.read().await;
            if !admission.accepts_start() {
                return;
            }
            let mut tasks = self.startup_retries.lock().await;
            if !self
                .executions
                .lock()
                .await
                .get(execution_id)
                .is_some_and(|execution| execution.is_active())
            {
                return;
            }
            let task_id = uuid::Uuid::new_v4().to_string();
            let (cancel, cancelled) = tokio::sync::watch::channel(false);
            let host = self.clone();
            let app = app.clone();
            let execution_id = execution_id.to_string();
            let worktree_path = worktree_path.to_string();
            let owner = execution_id.clone();
            let key = task_id.clone();
            let handle = tokio::spawn(async move {
                let gateway = HostNodeStartup {
                    host: &host,
                    app: &app,
                    execution_id: &execution_id,
                    worktree_path: &worktree_path,
                    cancelled,
                };
                if let Err(error) =
                    crate::usecase::workflow::node_startup::retry_failed_nodes(&gateway, failed)
                        .await
                {
                    log::warn!("workflow {execution_id}: startup retries failed: {error}");
                    if let Err(settle_error) = host
                        .settle_runtime_failure(&app, &execution_id, &error)
                        .await
                    {
                        log::warn!("workflow {execution_id}: startup failure settlement failed: {settle_error}");
                    }
                }
                host.startup_retries.lock().await.remove(&key);
            });
            tasks.insert(
                task_id,
                NodeStartupTask {
                    execution_id: owner,
                    cancel,
                    handle,
                },
            );
        })
    }

    pub(super) async fn cancel_startup_retries(&self, execution_id: &str) {
        for task in self.startup_retries.lock().await.values() {
            if task.execution_id == execution_id {
                task.cancel.send_replace(true);
            }
        }
    }

    pub(super) async fn shutdown_startup_retries(&self) {
        let tasks = std::mem::take(&mut *self.startup_retries.lock().await);
        for task in tasks.values() {
            task.cancel.send_replace(true);
        }
        for (_, task) in tasks {
            if let Err(error) = task.handle.await {
                log::warn!(
                    "workflow {}: startup task failed during shutdown: {error}",
                    task.execution_id
                );
            }
        }
    }

    pub(crate) async fn session_conversation_exists(
        &self,
        session_id: &str,
    ) -> Result<bool, WorkflowRuntimeError> {
        self.workflow_agent_sessions
            .has_recoverable_conversation(session_id)
            .await
    }

    pub(crate) async fn resume_session_process(
        &self,
        app: &WorkflowRuntimeDependencies,
        execution_id: &str,
        node_execution_id: &str,
        session_id: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        let gate = self.runtime_activation_gate(execution_id).await;
        let guard = gate.lock.lock().await;
        let current = self
            .load_control_plane_execution(execution_id)
            .await
            .ok_or_else(|| WorkflowRuntimeError::ExecutionNotFound(execution_id.into()))?;
        let node = current.node_execution(node_execution_id).ok_or_else(|| {
            WorkflowRuntimeError::InvalidState("Session attempt no longer exists".into())
        })?;
        let presence = self
            .node_processes
            .presence(
                &current.worktree_path,
                node_execution_id,
                node.kind,
                node.session_id.as_deref(),
            )
            .map_err(|error| WorkflowRuntimeError::InvalidState(error.to_string()))?;
        if !current.is_active()
            || !node.can_resume_session(presence)
            || node.session_id.as_deref() != Some(session_id)
        {
            return Err(WorkflowRuntimeError::InvalidState(
                "Session attempt cannot be resumed".into(),
            ));
        }
        let injection = current.pending_delegate_injection(node_execution_id);
        if injection.is_none() {
            run_runtime_activation(
                &gate,
                execution_id,
                "session",
                self.workflow_agent_sessions
                    .recover_workflow_agent_session_provider(session_id, node_execution_id),
            )
            .await?;
        }
        drop(guard);
        if let Some(injection) = injection {
            self.inject_delegate_result(app, execution_id, &injection)
                .await?;
        }
        let current = self
            .load_control_plane_execution(execution_id)
            .await
            .ok_or_else(|| WorkflowRuntimeError::ExecutionNotFound(execution_id.into()))?;
        let snapshot = RuntimeCommitSnapshot::from_execution(&current)?;
        workflow_runtime_session::broadcast_state(app, &current.worktree_path, snapshot).await;
        Ok(())
    }

    pub(super) async fn restart_node_attempt(
        &self,
        app: &WorkflowRuntimeDependencies,
        execution_id: &str,
        node_execution_id: &str,
    ) -> Result<Option<NodeStart>, WorkflowRuntimeError> {
        let gate = self.runtime_activation_gate(execution_id).await;
        let _guard = gate.lock.lock().await;
        let (before, mut candidate) = {
            let executions = self.executions.lock().await;
            let Some(current) = executions.get(execution_id) else {
                return Ok(None);
            };
            (current.clone(), current.clone())
        };
        let Some(node) = before.node_execution(node_execution_id) else {
            return Ok(None);
        };
        if self
            .node_processes
            .presence(
                &before.worktree_path,
                node_execution_id,
                node.kind,
                node.session_id.as_deref(),
            )
            .map_err(|error| WorkflowRuntimeError::InvalidState(error.to_string()))?
            != NodeProcessPresence::ConfirmedAbsent
        {
            return Ok(None);
        }
        let timestamp = current_timestamp();
        let Some(restarted) = candidate.restart_node_attempt_at(
            node_execution_id,
            new_node_execution_id(),
            timestamp,
        ) else {
            return Ok(None);
        };
        let events = [
            WorkflowEvent::NodeRetryRequested {
                execution_id: execution_id.into(),
                node_execution_id: node_execution_id.into(),
                timestamp,
            },
            WorkflowEvent::NodeStarted {
                worktree: restarted.attempt.worktree.clone(),
                execution_id: execution_id.into(),
                node_execution_id: restarted.attempt.id,
                node_name: restarted.attempt.node_name,
                kind: restarted.attempt.kind,
                attempt: restarted.attempt.attempt,
                parent: restarted.attempt.parent,
                timestamp,
            },
        ];
        let snapshot = self
            .commit_control_plane_candidate(
                app,
                ControlPlaneCommitCandidate {
                    execution_id,
                    snapshot_before: before,
                    candidate,
                    transition_outcome: TransitionOutcome::Applied,
                    events: &events,
                    provider_events: Vec::new(),
                },
            )
            .await?;
        workflow_runtime_session::broadcast_state(app, &snapshot.worktree_path, snapshot.clone())
            .await;
        Ok(Some(NodeStart::Leaf(restarted.leaf)))
    }
}
