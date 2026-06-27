mod builder;
mod instruction_resolver;
mod ports;

#[cfg(test)]
pub(crate) use builder::ContextEpochState;
pub(crate) use builder::{
    build_system_context, stable_content_fingerprint, BuiltSystemContext,
    SystemContextBuildRequest, SystemContextEditorInput,
};
pub(crate) use instruction_resolver::InstructionSourcePort;
pub(crate) use instruction_resolver::{
    file_system_instruction_cache_key, invalidate_instruction_resolution_cache_for_path,
};
#[cfg(test)]
pub(crate) use instruction_resolver::{InstructionResolutionRequest, InstructionResolver};
pub(crate) use ports::{
    BranchDiffContextChangedFile, BranchDiffContextPort, BranchDiffContextStats,
    BranchDiffContextSummary,
};
