mod terminal_activity;
mod terminal_process_launch;
#[cfg(test)]
#[path = "terminal_process_launch_test.rs"]
mod terminal_process_launch_tests;
mod terminal_process_state;
mod terminal_runtime_generation;
mod terminal_surface_checkpoint;
mod terminal_surface_lifecycle_config;
mod terminal_surface_owner;
mod terminal_surface_startup_command;

pub use terminal_activity::{TerminalActivity, TERMINAL_ACTIVITY_RUNNING_WINDOW};
pub use terminal_process_launch::TerminalProcessLaunch;
#[cfg(test)]
pub use terminal_process_launch::TerminalProcessLaunchError;
pub use terminal_process_state::TerminalProcessState;
pub use terminal_runtime_generation::TerminalRuntimeGeneration;
pub use terminal_surface_checkpoint::{
    TerminalSurfaceCheckpoint, TERMINAL_SURFACE_SCROLLBACK_ROWS,
};
pub use terminal_surface_lifecycle_config::TerminalSurfaceLifecycleConfig;
pub use terminal_surface_owner::TerminalSurfaceOwner;
pub use terminal_surface_startup_command::TerminalSurfaceStartupCommand;
