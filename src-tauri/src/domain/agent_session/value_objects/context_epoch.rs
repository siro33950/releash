use std::path::PathBuf;

/// Context freshness generation for one agent session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContextEpochId(pub u64);

impl ContextEpochId {
    pub fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

impl Default for ContextEpochId {
    fn default() -> Self {
        Self(1)
    }
}

/// Per-source revision inside an epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContextRevision(pub u64);

impl ContextRevision {
    pub fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

impl Default for ContextRevision {
    fn default() -> Self {
        Self(1)
    }
}

/// Agent system context source kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContextSourceKind {
    RepoSummary,
    DiffReviewSnapshot,
    OpenEditorSelection,
    Mentions,
    TerminalLogSummary,
    WorkflowState,
    ProjectInstructions,
    /// Backend/model identity only. The runtime system prompt body is managed
    /// outside context epochs by the system_prompt_fingerprint path.
    BackendModelIdentity,
}

impl ContextSourceKind {
    pub const MINIMUM_SYSTEM_CONTEXT_SOURCES: [Self; 7] = [
        Self::RepoSummary,
        Self::DiffReviewSnapshot,
        Self::OpenEditorSelection,
        Self::Mentions,
        Self::TerminalLogSummary,
        Self::WorkflowState,
        Self::ProjectInstructions,
    ];

    pub const ALL: [Self; 8] = [
        Self::RepoSummary,
        Self::DiffReviewSnapshot,
        Self::OpenEditorSelection,
        Self::Mentions,
        Self::TerminalLogSummary,
        Self::WorkflowState,
        Self::ProjectInstructions,
        Self::BackendModelIdentity,
    ];

    pub fn definition(self) -> ContextSourceDefinition {
        match self {
            Self::RepoSummary => ContextSourceDefinition {
                meaning: "repository summary",
                source: "existing repository summary input",
                refresh_trigger: "worktree change or explicit rebuild",
            },
            Self::DiffReviewSnapshot => ContextSourceDefinition {
                meaning: "diff or review snapshot",
                source: "existing diff/review snapshot input",
                refresh_trigger: "worktree change or explicit rebuild",
            },
            Self::OpenEditorSelection => ContextSourceDefinition {
                meaning: "open editor paths and current selection",
                source: "frontend raw editor context input",
                refresh_trigger: "editor input change or worktree change",
            },
            Self::Mentions => ContextSourceDefinition {
                meaning: "resolved file mentions",
                source: "frontend raw mention input resolved by Rust",
                refresh_trigger: "mention input change or worktree change",
            },
            Self::TerminalLogSummary => ContextSourceDefinition {
                meaning: "terminal log summary",
                source: "existing terminal summary input",
                refresh_trigger: "terminal summary input change",
            },
            Self::WorkflowState => ContextSourceDefinition {
                meaning: "workflow execution and node state",
                source: "workflow node context input",
                refresh_trigger: "workflow state input change",
            },
            Self::ProjectInstructions => ContextSourceDefinition {
                meaning: "project instructions from AGENTS.md and CLAUDE.md",
                source: "instruction resolver",
                refresh_trigger: "backend/model/worktree/instruction file change",
            },
            Self::BackendModelIdentity => ContextSourceDefinition {
                meaning: "backend/model identity payload",
                source: "backend_id/model_id identity strings",
                refresh_trigger: "backend or model change",
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextSourceDefinition {
    pub meaning: &'static str,
    pub source: &'static str,
    pub refresh_trigger: &'static str,
}

/// Epoch identity. A change here creates a new epoch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextEpoch {
    pub id: ContextEpochId,
    pub backend_id: Option<String>,
    pub model_id: Option<String>,
    pub worktree_path: String,
}

impl ContextEpoch {
    pub fn identity_matches(&self, other: &ContextEpochIdentity) -> bool {
        self.backend_id == other.backend_id
            && self.model_id == other.model_id
            && self.worktree_path == other.worktree_path
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextEpochIdentity {
    pub backend_id: Option<String>,
    pub model_id: Option<String>,
    pub worktree_path: String,
}

/// One source payload at one epoch/revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextSnapshot {
    pub kind: ContextSourceKind,
    pub epoch_id: ContextEpochId,
    pub revision: ContextRevision,
    pub fingerprint: String,
    pub payload: String,
}

/// Latest known state for a source. Missing sources are represented by `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextSourceState {
    pub kind: ContextSourceKind,
    pub latest: Option<ContextSnapshot>,
    pub revision_counter: ContextRevision,
}

impl ContextSourceState {
    pub fn new(kind: ContextSourceKind) -> Self {
        Self {
            kind,
            latest: None,
            revision_counter: ContextRevision::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplacementTrigger {
    None,
    BackendChanged,
    ModelChanged,
    WorktreeChanged,
    InstructionFileChanged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplacementAction {
    Retain,
    Rebuild,
    Discard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InstructionOrigin {
    RepoHierarchy,
    FileNeighbor,
    WorkflowFacet,
}

/// Resolved instruction before deduplication.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedInstruction {
    pub origin: InstructionOrigin,
    pub source_path: Option<PathBuf>,
    pub content: String,
    pub fingerprint: String,
    pub scope_depth: usize,
}

impl ResolvedInstruction {
    pub fn new(
        origin: InstructionOrigin,
        source_path: Option<PathBuf>,
        content: impl Into<String>,
        fingerprint: impl Into<String>,
        scope_depth: usize,
    ) -> Self {
        Self {
            origin,
            source_path,
            content: content.into(),
            fingerprint: fingerprint.into(),
            scope_depth,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimum_context_sources_are_listed_with_definitions() {
        assert_eq!(ContextSourceKind::MINIMUM_SYSTEM_CONTEXT_SOURCES.len(), 7);
        let definitions = ContextSourceKind::MINIMUM_SYSTEM_CONTEXT_SOURCES
            .into_iter()
            .map(ContextSourceKind::definition)
            .collect::<Vec<_>>();

        assert!(definitions
            .iter()
            .all(|definition| !definition.meaning.is_empty()
                && !definition.source.is_empty()
                && !definition.refresh_trigger.is_empty()));
    }

    #[test]
    fn workflow_context_source_uses_execution_and_node_vocabulary() {
        let definition = ContextSourceKind::WorkflowState.definition();

        assert_eq!(definition.meaning, "workflow execution and node state");
        assert_eq!(definition.source, "workflow node context input");
    }
}
