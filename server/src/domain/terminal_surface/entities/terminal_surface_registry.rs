use std::collections::{HashMap, HashSet};

use crate::domain::terminal_surface::entities::{TerminalSurface, TerminalSurfaceSummary};
use crate::domain::terminal_surface::TerminalSurfaceCheckpoint;

pub struct TerminalSurfaceRegistry {
    sessions: HashMap<u64, TerminalSurface>,
    reserved_spawn_session_keys: HashSet<String>,
    next_runtime_generation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalSurfaceSpawnReservation {
    pub session_key: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalSurfaceSpawnReservationError {
    OwnerOccupied(String),
}

impl Default for TerminalSurfaceRegistry {
    fn default() -> Self {
        Self {
            sessions: HashMap::new(),
            reserved_spawn_session_keys: HashSet::new(),
            next_runtime_generation: 1,
        }
    }
}

impl TerminalSurfaceRegistry {
    pub fn next_runtime_generation(&mut self) -> u64 {
        let runtime_generation = self.next_runtime_generation;
        self.next_runtime_generation += 1;
        runtime_generation
    }

    pub fn insert(&mut self, session: TerminalSurface) -> Option<TerminalSurface> {
        self.sessions
            .insert(session.runtime_generation.value(), session)
    }

    pub fn remove(&mut self, runtime_generation: u64) -> Option<TerminalSurface> {
        self.sessions.remove(&runtime_generation)
    }

    pub fn get(&self, runtime_generation: u64) -> Option<&TerminalSurface> {
        self.sessions.get(&runtime_generation)
    }

    pub fn find_by_session_key(&self, session_key: &str) -> Option<&TerminalSurface> {
        self.sessions
            .values()
            .find(|session| session.session_key == session_key)
    }

    pub fn list_summaries(&self) -> Vec<TerminalSurfaceSummary> {
        self.sessions
            .values()
            .map(TerminalSurface::summary)
            .collect()
    }

    pub fn select_kill_targets_by_worktree(&self, worktree_path: &str) -> Vec<u64> {
        self.sessions
            .values()
            .filter(|session| session.worktree_path.as_deref() == Some(worktree_path))
            .map(|session| session.runtime_generation.value())
            .collect()
    }

    pub fn record_output(
        &mut self,
        runtime_generation: u64,
        now: std::time::Instant,
    ) -> Option<u64> {
        let surface = self.sessions.get_mut(&runtime_generation)?;
        surface.record_output(surface.runtime_generation, now)
    }

    pub fn record_resize(&mut self, runtime_generation: u64) -> Option<u64> {
        let surface = self.sessions.get_mut(&runtime_generation)?;
        surface.record_resize(surface.runtime_generation)
    }

    pub fn mark_exited(&mut self, runtime_generation: u64, exit_code: Option<i32>) -> Option<u64> {
        let session = self.sessions.get_mut(&runtime_generation)?;
        session.mark_exited(session.runtime_generation, exit_code)
    }

    pub fn apply_checkpoint(
        &mut self,
        runtime_generation: u64,
        checkpoint: TerminalSurfaceCheckpoint,
    ) -> bool {
        let Some(surface) = self.sessions.get_mut(&runtime_generation) else {
            return false;
        };
        surface.apply_checkpoint(surface.runtime_generation, checkpoint)
    }

    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    pub fn reserve_spawn_slot(
        &mut self,
        session_key: &str,
    ) -> Result<TerminalSurfaceSpawnReservation, TerminalSurfaceSpawnReservationError> {
        if self.find_by_session_key(session_key).is_some()
            || self.reserved_spawn_session_keys.contains(session_key)
        {
            return Err(TerminalSurfaceSpawnReservationError::OwnerOccupied(
                session_key.to_string(),
            ));
        }
        self.reserved_spawn_session_keys
            .insert(session_key.to_string());

        Ok(TerminalSurfaceSpawnReservation {
            session_key: session_key.to_string(),
        })
    }

    pub fn is_spawn_reserved(&self, session_key: &str) -> bool {
        self.reserved_spawn_session_keys.contains(session_key)
    }

    pub fn complete_spawn_slot(&mut self, reservation: &TerminalSurfaceSpawnReservation) {
        self.release_spawn_slot(reservation);
    }

    pub fn rollback_spawn_slot(&mut self, reservation: &TerminalSurfaceSpawnReservation) {
        self.release_spawn_slot(reservation);
    }

    fn release_spawn_slot(&mut self, reservation: &TerminalSurfaceSpawnReservation) {
        self.reserved_spawn_session_keys
            .remove(&reservation.session_key);
    }
}

#[cfg(test)]
#[path = "terminal_surface_registry_test.rs"]
mod terminal_surface_registry_tests;
