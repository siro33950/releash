pub mod entities;
pub mod events;
pub mod gateway;
pub(crate) mod services;
pub(crate) mod storage;
pub(crate) mod value_objects;

pub(crate) use services::{
    decide_workflow_turn_admission, dedup_instructions, latest_revisions_by_kind,
    next_epoch_for_identity, normalize_path_components, replacement_action, snapshot_is_stale,
    WorkflowTurnAdmissionFacts, WorkflowTurnAdmissionRejection,
};
#[cfg(test)]
pub(crate) use storage::AgentSessionProjectionPreparer;
pub(crate) use storage::{
    AgentSessionProjectedMessage, AgentSessionProjectionCommit, AgentSessionReader,
    AgentSessionStorageTypes,
};
#[cfg(test)]
pub(crate) use storage::{AgentSessionStorage, AgentSessionWriter};
pub(crate) use value_objects::{
    ContextEpoch, ContextEpochId, ContextEpochIdentity, ContextRevision, ContextSnapshot,
    ContextSourceKind, ContextSourceState, InstructionOrigin, InvalidPermissionMode, ModelId,
    PermissionMode, ReplacementAction, ReplacementTrigger, ResolvedInstruction, SkillEntry,
};
