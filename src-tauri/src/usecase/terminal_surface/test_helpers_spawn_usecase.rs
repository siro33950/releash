use crate::domain::terminal_surface::TerminalSurfaceOwner;
use crate::domain::workspace_tree::WorkspaceIdentity;
pub fn workspace_owner(path: &str) -> TerminalSurfaceOwner {
    TerminalSurfaceOwner::workspace(WorkspaceIdentity::new(path)).unwrap()
}
