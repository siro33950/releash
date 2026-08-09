pub(crate) mod entities;
pub(crate) mod gateway;
pub(crate) mod value_objects;

pub use value_objects::{
    TerminalActivity, TerminalProcessLaunch, TerminalProcessState, TerminalRuntimeGeneration,
    TerminalSurfaceCheckpoint, TerminalSurfaceLifecycleConfig, TerminalSurfaceOwner,
    TerminalSurfaceStartupCommand, TERMINAL_ACTIVITY_RUNNING_WINDOW,
    TERMINAL_SURFACE_SCROLLBACK_ROWS,
};
