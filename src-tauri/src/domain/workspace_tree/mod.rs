//! Bounded Workspace/Session query domain.
//!
//! The aggregate is restored from canonical execution/node/session records.
//! It deliberately has no dedicated snapshot, revision, or CAS lifecycle.

mod entities;
mod projection;
mod repository;
mod services;
mod value_objects;

pub use entities::{WorkspaceTree, WorkspaceTreeProjector};
pub use projection::{runtime_snapshot_nodes, workflow_fact, RuntimeSnapshotNodeProjection};
pub use repository::WorkspaceTreeRepository;
pub use services::{
    recovery_reason, unresolved_recovery_reason, WorkspaceSessionPublicationPolicy,
    WorkspaceTreeVisibilityPolicy,
};
pub use value_objects::{
    WorkspaceCommandResult, WorkspaceIdentity, WorkspaceNodeKind, WorkspaceNodeStatus,
    WorkspaceSessionListKind, WorkspaceStructureFact, WorkspaceTreeNode,
};
