mod terminal_surface;
mod terminal_surface_attachment;
mod terminal_surface_registry;
mod terminal_surface_runtime_lifecycle;

pub use terminal_surface::{TerminalSurface, TerminalSurfaceSummary};
pub use terminal_surface_attachment::{TerminalSurfaceAttachment, TerminalSurfaceSequenceDecision};
pub use terminal_surface_registry::{
    TerminalSurfaceRegistry, TerminalSurfaceSpawnReservation, TerminalSurfaceSpawnReservationError,
};
pub use terminal_surface_runtime_lifecycle::{
    TerminalSurfaceMutationRejected, TerminalSurfaceRuntimeLifecycle,
};
