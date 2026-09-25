//! Bounded Workspace/Session query domain.
//!
//! WorkspaceTree is restored from canonical execution/node/session records and
//! has no dedicated persistence or CAS lifecycle. WorkspaceListRefresh owns the
//! in-memory list refresh lifecycle: retained results, failures, generations,
//! and coalescing of pending full refresh requests.

pub(crate) mod background_failure;
mod entities;
mod projection;
mod refresh;
mod repository;
mod services;
mod value_objects;

pub use entities::{WorkspaceTree, WorkspaceTreeProjector};
pub use projection::{runtime_snapshot_nodes, RuntimeSnapshotNodeProjection};
pub(crate) use refresh::{
    WorkspaceListEntry, WorkspaceListFailure, WorkspaceListRefresh, WorkspaceListState,
};
pub use repository::WorkspaceTreeRepository;
pub use services::{WorkspacePublicRoot, WorkspaceTreeVisibilityPolicy};
pub use value_objects::{
    WorkspaceCommandResult, WorkspaceIdentity, WorkspaceNodeKind, WorkspaceNodeStatus,
    WorkspaceNodeStatusClassification, WorkspaceStructureFact, WorkspaceTreeNode,
};
