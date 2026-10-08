#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TerminalSurfaceRuntimePhase {
    AcceptingMutations,
    ShuttingDown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalSurfaceMutationRejected;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalSurfaceRuntimeLifecycle {
    runtime_id: String,
    phase: TerminalSurfaceRuntimePhase,
}

impl TerminalSurfaceRuntimeLifecycle {
    pub fn new(runtime_id: String) -> Self {
        Self {
            runtime_id,
            phase: TerminalSurfaceRuntimePhase::AcceptingMutations,
        }
    }

    #[cfg(test)]
    pub fn runtime_id(&self) -> &str {
        &self.runtime_id
    }

    pub fn admit_mutation(&self) -> Result<(), TerminalSurfaceMutationRejected> {
        match self.phase {
            TerminalSurfaceRuntimePhase::AcceptingMutations => Ok(()),
            TerminalSurfaceRuntimePhase::ShuttingDown => Err(TerminalSurfaceMutationRejected),
        }
    }

    pub fn begin_shutdown(&mut self) {
        self.phase = TerminalSurfaceRuntimePhase::ShuttingDown;
    }
}

#[cfg(test)]
#[path = "terminal_surface_runtime_lifecycle_test.rs"]
mod terminal_surface_runtime_lifecycle_tests;
