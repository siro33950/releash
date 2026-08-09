use std::collections::{HashMap, HashSet};

use crate::domain::terminal_surface::entities::{TerminalSurface, TerminalSurfaceSummary};
use crate::domain::terminal_surface::{TerminalSurfaceCheckpoint, TerminalSurfaceLifecycleConfig};

pub struct TerminalSurfaceRegistry {
    sessions: HashMap<u64, TerminalSurface>,
    reserved_spawn_total: usize,
    reserved_spawn_by_worktree: HashMap<String, usize>,
    reserved_spawn_session_keys: HashSet<String>,
    next_runtime_generation: u64,
    config: TerminalSurfaceLifecycleConfig,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalSurfaceSpawnReservation {
    pub session_key: String,
    pub worktree_path: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalSurfaceSpawnReservationError {
    OwnerOccupied(String),
    WorktreeCapReached(String),
    TotalCapReached,
}

impl Default for TerminalSurfaceRegistry {
    fn default() -> Self {
        Self {
            sessions: HashMap::new(),
            reserved_spawn_total: 0,
            reserved_spawn_by_worktree: HashMap::new(),
            reserved_spawn_session_keys: HashSet::new(),
            next_runtime_generation: 1,
            config: TerminalSurfaceLifecycleConfig::default(),
        }
    }
}

impl TerminalSurfaceRegistry {
    #[cfg(test)]
    pub fn with_config(config: TerminalSurfaceLifecycleConfig) -> Self {
        Self {
            config,
            ..Self::default()
        }
    }

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

    #[cfg(test)]
    pub fn count_total_alive(&self) -> usize {
        self.sessions
            .values()
            .filter(|session| !session.process_state.is_exited())
            .count()
    }

    #[cfg(test)]
    pub fn count_alive_for_worktree(&self, worktree_path: &str) -> usize {
        self.sessions
            .values()
            .filter(|session| {
                !session.process_state.is_exited()
                    && session.worktree_path.as_deref() == Some(worktree_path)
            })
            .count()
    }

    #[cfg(test)]
    pub fn would_exceed_worktree_cap(&self, worktree_path: &str) -> bool {
        self.count_alive_for_worktree(worktree_path) >= self.config.per_worktree_cap
    }

    #[cfg(test)]
    pub fn would_exceed_total_cap(&self) -> bool {
        self.count_total_alive() >= self.config.max_panes_total
    }

    pub fn reserve_spawn_slot(
        &mut self,
        session_key: &str,
        worktree_path: Option<&str>,
    ) -> Result<TerminalSurfaceSpawnReservation, TerminalSurfaceSpawnReservationError> {
        if self.find_by_session_key(session_key).is_some()
            || self.reserved_spawn_session_keys.contains(session_key)
        {
            return Err(TerminalSurfaceSpawnReservationError::OwnerOccupied(
                session_key.to_string(),
            ));
        }
        if let Some(worktree_path) = worktree_path {
            if self.effective_alive_for_worktree(worktree_path)
                + self.reserved_spawn_for_worktree(worktree_path)
                >= self.config.per_worktree_cap
            {
                return Err(TerminalSurfaceSpawnReservationError::WorktreeCapReached(
                    worktree_path.to_string(),
                ));
            }
        }

        if self.effective_total_alive() + self.reserved_spawn_total >= self.config.max_panes_total {
            return Err(TerminalSurfaceSpawnReservationError::TotalCapReached);
        }

        self.reserved_spawn_total += 1;
        self.reserved_spawn_session_keys
            .insert(session_key.to_string());
        if let Some(worktree_path) = worktree_path {
            *self
                .reserved_spawn_by_worktree
                .entry(worktree_path.to_string())
                .or_insert(0) += 1;
        }

        Ok(TerminalSurfaceSpawnReservation {
            session_key: session_key.to_string(),
            worktree_path: worktree_path.map(str::to_string),
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
        self.reserved_spawn_total = self.reserved_spawn_total.saturating_sub(1);
        self.reserved_spawn_session_keys
            .remove(&reservation.session_key);
        if let Some(worktree_path) = &reservation.worktree_path {
            if let Some(count) = self.reserved_spawn_by_worktree.get_mut(worktree_path) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    self.reserved_spawn_by_worktree.remove(worktree_path);
                }
            }
        }
    }

    fn effective_total_alive(&self) -> usize {
        self.sessions
            .values()
            .filter(|session| !session.process_state.is_exited())
            .count()
    }

    fn effective_alive_for_worktree(&self, worktree_path: &str) -> usize {
        self.sessions
            .values()
            .filter(|session| {
                !session.process_state.is_exited()
                    && session.worktree_path.as_deref() == Some(worktree_path)
            })
            .count()
    }

    fn reserved_spawn_for_worktree(&self, worktree_path: &str) -> usize {
        self.reserved_spawn_by_worktree
            .get(worktree_path)
            .copied()
            .unwrap_or(0)
    }
}

#[cfg(test)]
#[path = "terminal_surface_registry_test.rs"]
mod terminal_surface_registry_tests;
