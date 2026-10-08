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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalSurfaceOwnerError {
    WorkspacePathMissing,
    SessionIdMissing,
}

impl TerminalSurfaceOwner {
    pub fn workspace(workspace: WorkspaceIdentity) -> Result<Self, TerminalSurfaceOwnerError> {
        Self::validate_workspace(&workspace)?;
        Ok(Self::Workspace { workspace })
    }

    pub fn session(
        workspace: WorkspaceIdentity,
        session_id: impl Into<String>,
    ) -> Result<Self, TerminalSurfaceOwnerError> {
        Self::validate_workspace(&workspace)?;
        let session_id = session_id.into();
        if session_id.trim().is_empty() {
            return Err(TerminalSurfaceOwnerError::SessionIdMissing);
        }
        Ok(Self::Session {
            workspace,
            session_id,
        })
    }

    /// 集約不変条件で非空が保証済みの値からの構築。domain内部専用。
    pub(in crate::domain) fn session_from_validated(
        workspace: WorkspaceIdentity,
        session_id: impl Into<String>,
    ) -> Self {
        Self::Session {
            workspace,
            session_id: session_id.into(),
        }
    }

    fn validate_workspace(workspace: &WorkspaceIdentity) -> Result<(), TerminalSurfaceOwnerError> {
        if workspace.as_str().trim().is_empty() {
            return Err(TerminalSurfaceOwnerError::WorkspacePathMissing);
        }
        Ok(())
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
