use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::domain::agent_session::aggregates::ManagedPtyPresence;
use crate::domain::agent_session::ProviderAgentTerminalGateway;
use crate::domain::terminal_surface::TerminalSurfaceOwner;
use crate::domain::workflow::{
    NodeKindName, NodeProcessPresence, NodeProcessReader, WorkflowError,
};
use crate::domain::workspace_tree::WorkspaceIdentity;
use crate::infrastructure::process::command_runner::ActiveCommandHandle;

#[derive(Default)]
pub(crate) struct WorkflowNodeProcesses {
    pub(super) active_commands: Mutex<HashMap<String, ActiveCommandHandle>>,
    terminal: Option<Arc<dyn ProviderAgentTerminalGateway>>,
}

impl WorkflowNodeProcesses {
    pub(crate) fn new(terminal: Arc<dyn ProviderAgentTerminalGateway>) -> Self {
        Self {
            terminal: Some(terminal),
            ..Self::default()
        }
    }
}

impl NodeProcessReader for WorkflowNodeProcesses {
    fn presence(
        &self,
        workspace: &str,
        node_execution_id: &str,
        kind: NodeKindName,
        session_id: Option<&str>,
    ) -> Result<NodeProcessPresence, WorkflowError> {
        match kind {
            NodeKindName::Command => Ok(
                if self
                    .active_commands
                    .lock()
                    .map_err(|_| {
                        WorkflowError::external("command process registry is unavailable")
                    })?
                    .contains_key(node_execution_id)
                {
                    NodeProcessPresence::Live
                } else {
                    NodeProcessPresence::ConfirmedAbsent
                },
            ),
            NodeKindName::Session => {
                let Some(session_id) = session_id else {
                    return Ok(NodeProcessPresence::ConfirmedAbsent);
                };
                let Some(terminal) = &self.terminal else {
                    return Ok(NodeProcessPresence::Unknown);
                };
                let owner =
                    TerminalSurfaceOwner::session(WorkspaceIdentity::new(workspace), session_id)
                        .map_err(|_| {
                            WorkflowError::invalid_state("invalid Session terminal owner")
                        })?;
                Ok(
                    match terminal.presence(&owner).map_err(|_| {
                        WorkflowError::external("Session process presence is unavailable")
                    })? {
                        ManagedPtyPresence::Live => NodeProcessPresence::Live,
                        ManagedPtyPresence::ConfirmedAbsent => NodeProcessPresence::ConfirmedAbsent,
                        ManagedPtyPresence::Unknown => NodeProcessPresence::Unknown,
                    },
                )
            }
            NodeKindName::Sequence | NodeKindName::Fanout => Ok(NodeProcessPresence::Unknown),
        }
    }
}
