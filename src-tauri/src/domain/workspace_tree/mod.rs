//! Bounded Workspace/Session query domain.
//!
//! WorkspaceTree is restored from canonical execution/node/session records and
//! has no dedicated persistence or CAS lifecycle.

mod entities;
mod projection;
mod repository;
mod services;
mod value_objects;
mod visible;

pub use entities::{WorkspaceTree, WorkspaceTreeProjector};
pub use projection::{runtime_snapshot_nodes, RuntimeSnapshotNodeProjection};
pub use repository::WorkspaceTreeRepository;
pub use services::{WorkspacePublicRoot, WorkspaceTreeVisibilityPolicy};
pub(crate) use value_objects::WorkspaceExecution;
pub use value_objects::{
    WorkspaceCommandResult, WorkspaceIdentity, WorkspaceNodeKind, WorkspaceNodeStatus,
    WorkspaceNodeStatusClassification, WorkspaceStructureFact, WorkspaceTreeNode,
};
pub(crate) use visible::WorkspaceVisibleNode;
