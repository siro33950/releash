use crate::domain::workspace_tree::WorkspaceIdentity;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TerminalSurfaceOwner {
    Workspace {
        workspace: WorkspaceIdentity,
    },
    Session {
        workspace: WorkspaceIdentity,
        session_id: String,
    },
}

impl TerminalSurfaceOwner {
    pub fn workspace(workspace: WorkspaceIdentity) -> Self {
        Self::Workspace { workspace }
    }

    pub fn session(workspace: WorkspaceIdentity, session_id: impl Into<String>) -> Self {
        Self::Session {
            workspace,
            session_id: session_id.into(),
        }
    }

    pub fn workspace_identity(&self) -> &WorkspaceIdentity {
        match self {
            Self::Workspace { workspace } | Self::Session { workspace, .. } => workspace,
        }
    }

    pub fn stable_key(&self) -> String {
        let workspace = self.workspace_identity().as_str();
        match self {
            Self::Workspace { .. } => format!("workspace:{}:{workspace}", workspace.len()),
            Self::Session { session_id, .. } => format!(
                "session:{}:{workspace}:{}:{session_id}",
                workspace.len(),
                session_id.len()
            ),
        }
    }
}

#[cfg(test)]
#[path = "terminal_surface_owner_test.rs"]
mod terminal_surface_owner_tests;
