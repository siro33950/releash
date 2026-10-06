use super::*;
use crate::adaptor::gateway::workflow::node_session_boundary::NodeSessionInfo;
use crate::domain::provider_lifecycle::ProviderKind;
pub const EFFECT_AGENT_SESSION_ID: &str = "agent-session-effect-test";

pub struct RecordingWorkflowAgentSessions {
    pub stop_calls: Arc<std::sync::Mutex<Vec<(String, String)>>>,
    pub prepare_calls: Arc<std::sync::Mutex<Vec<(String, String, WorkflowSessionLaunchConfig)>>>,
    pub provider_running_checks: Arc<std::sync::Mutex<Vec<(String, String)>>>,
    pub recovery_fails: Arc<std::sync::atomic::AtomicBool>,
    pub failing_agent_session_id: String,
}

#[async_trait::async_trait]
impl WorkflowAgentSessionPort for RecordingWorkflowAgentSessions {
    async fn has_recoverable_conversation(&self, _id: &str) -> Result<bool, WorkflowRuntimeError> {
        Ok(true)
    }

    fn is_provider_available(&self, _provider: ProviderKind) -> bool {
        true
    }

    async fn prepare_workflow_agent_session(
        &self,
        _workspace_worktree_path: &str,
        _worktree_path: &str,
        config: WorkflowSessionLaunchConfig,
        workflow_execution_id: &str,
        node_execution_id: &str,
        _initial_instruction: &str,
    ) -> Result<NodeSessionInfo, WorkflowRuntimeError> {
        self.prepare_calls.lock().unwrap().push((
            workflow_execution_id.to_string(),
            node_execution_id.to_string(),
            config,
        ));
        Ok(NodeSessionInfo {
            id: EFFECT_AGENT_SESSION_ID.to_string(),
        })
    }

    async fn activate_workflow_agent_session(
        &self,
        _node_session_id: &str,
        _node_execution_id: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        Ok(())
    }

    async fn confirm_workflow_agent_session_attachment(
        &self,
        _node_session_id: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        Ok(())
    }

    async fn dispatch_continuation(
        &self,
        _node_session_id: &str,
        _child_execution_id: &str,
        _instruction: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        panic!("unexpected delegate continuation")
    }

    async fn recover_workflow_agent_session_provider(
        &self,
        node_session_id: &str,
        node_execution_id: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        self.provider_running_checks
            .lock()
            .unwrap()
            .push((node_execution_id.to_string(), node_session_id.to_string()));
        if self
            .recovery_fails
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            return Err(WorkflowRuntimeError::AgentSession(
                "intentional provider recovery failure".to_string(),
            ));
        }
        Ok(())
    }

    async fn stop_agent_session_for_terminal_node_preserving_checkpoint(
        &self,
        node_session_id: &str,
        node_execution_id: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        self.stop_calls
            .lock()
            .unwrap()
            .push((node_execution_id.to_string(), node_session_id.to_string()));
        if node_session_id == self.failing_agent_session_id {
            return Err(WorkflowRuntimeError::AgentSession(
                "intentional stop failure".to_string(),
            ));
        }
        Ok(())
    }

    async fn rollback_workflow_agent_session(
        &self,
        _node_session_id: &str,
        _node_execution_id: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        Ok(())
    }
}
