use std::time::Instant;

use crate::domain::terminal_surface::{
    TerminalProcessState, TerminalRuntimeGeneration, TerminalSurfaceCheckpoint,
    TerminalSurfaceOwner,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalSurface {
    pub session_key: String,
    pub owner: TerminalSurfaceOwner,
    pub worktree_path: Option<String>,
    pub label: Option<String>,
    pub runtime_generation: TerminalRuntimeGeneration,
    pub process_state: TerminalProcessState,
    pub checkpoint: TerminalSurfaceCheckpoint,
    pub(crate) latest_sequence: u64,
    pub(crate) last_output_at: Option<Instant>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalSurfaceNotWritable;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalSurfaceSummary {
    pub session_key: String,
    pub owner: TerminalSurfaceOwner,
    pub worktree_path: Option<String>,
    pub label: Option<String>,
    pub runtime_generation: TerminalRuntimeGeneration,
    pub process_state: TerminalProcessState,
    pub latest_sequence: u64,
    pub last_output_at: Option<Instant>,
}

impl TerminalSurface {
    #[cfg(test)]
    pub fn new(
        runtime_generation: impl Into<TerminalRuntimeGeneration>,
        owner: TerminalSurfaceOwner,
        label: Option<String>,
    ) -> Self {
        Self::with_checkpoint(
            runtime_generation,
            owner,
            label,
            TerminalSurfaceCheckpoint::empty(80, 24),
        )
    }

    pub fn with_checkpoint(
        runtime_generation: impl Into<TerminalRuntimeGeneration>,
        owner: TerminalSurfaceOwner,
        label: Option<String>,
        checkpoint: TerminalSurfaceCheckpoint,
    ) -> Self {
        let session_key = owner.stable_key();
        let worktree_path = Some(owner.workspace_identity().as_str().to_string());
        Self {
            session_key,
            owner,
            worktree_path,
            label,
            runtime_generation: runtime_generation.into(),
            process_state: TerminalProcessState::Running,
            latest_sequence: checkpoint.sequence,
            last_output_at: None,
            checkpoint,
        }
    }

    pub fn latest_sequence(&self) -> u64 {
        self.latest_sequence
    }

    pub fn summary(&self) -> TerminalSurfaceSummary {
        TerminalSurfaceSummary {
            session_key: self.session_key.clone(),
            owner: self.owner.clone(),
            worktree_path: self.worktree_path.clone(),
            label: self.label.clone(),
            runtime_generation: self.runtime_generation,
            process_state: self.process_state.clone(),
            latest_sequence: self.latest_sequence,
            last_output_at: self.last_output_at,
        }
    }

    pub fn ensure_writable(&self) -> Result<(), TerminalSurfaceNotWritable> {
        if self.process_state.is_exited() {
            return Err(TerminalSurfaceNotWritable);
        }
        Ok(())
    }

    pub fn record_output(
        &mut self,
        runtime_generation: TerminalRuntimeGeneration,
        now: Instant,
    ) -> Option<u64> {
        let sequence = self.advance_running_sequence(runtime_generation)?;
        self.last_output_at = Some(now);
        Some(sequence)
    }

    pub fn record_resize(&mut self, runtime_generation: TerminalRuntimeGeneration) -> Option<u64> {
        self.advance_running_sequence(runtime_generation)
    }

    pub fn apply_checkpoint(
        &mut self,
        runtime_generation: impl Into<TerminalRuntimeGeneration>,
        checkpoint: TerminalSurfaceCheckpoint,
    ) -> bool {
        let runtime_generation = runtime_generation.into();
        if self.runtime_generation != runtime_generation
            || checkpoint.sequence < self.checkpoint.sequence
        {
            return false;
        }
        self.latest_sequence = self.latest_sequence.max(checkpoint.sequence);
        self.checkpoint = checkpoint;
        true
    }

    pub fn mark_exited(
        &mut self,
        runtime_generation: TerminalRuntimeGeneration,
        exit_code: Option<i32>,
    ) -> Option<u64> {
        let sequence = self.advance_running_sequence(runtime_generation)?;
        self.process_state = TerminalProcessState::Exited { exit_code };
        Some(sequence)
    }

    fn advance_running_sequence(
        &mut self,
        runtime_generation: TerminalRuntimeGeneration,
    ) -> Option<u64> {
        if self.runtime_generation != runtime_generation || self.process_state.is_exited() {
            return None;
        }
        let sequence = self.latest_sequence.checked_add(1)?;
        self.latest_sequence = sequence;
        Some(sequence)
    }

    #[cfg(test)]
    pub fn new_with_session_key(
        runtime_generation: impl Into<TerminalRuntimeGeneration>,
        session_key: String,
        owner: TerminalSurfaceOwner,
        label: Option<String>,
    ) -> Self {
        let mut session = Self::new(runtime_generation, owner, label);
        session.session_key = session_key;
        session
    }
}

#[cfg(test)]
#[path = "terminal_surface_test.rs"]
mod terminal_surface_tests;
