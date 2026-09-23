use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WorkspaceListFailure(String);

impl From<String> for WorkspaceListFailure {
    fn from(message: String) -> Self {
        Self(message)
    }
}
impl From<&str> for WorkspaceListFailure {
    fn from(message: &str) -> Self {
        Self(message.to_owned())
    }
}
impl std::fmt::Display for WorkspaceListFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for WorkspaceListFailure {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WorkspaceListState {
    Loading,
    InitialFailed,
    Empty,
    Ready,
    RefreshFailed,
}

#[derive(Debug)]
pub(crate) struct WorkspaceListEntry<T> {
    generation: u64,
    value: Option<T>,
    error: Option<WorkspaceListFailure>,
}

impl<T> Default for WorkspaceListEntry<T> {
    fn default() -> Self {
        Self {
            generation: 0,
            value: None,
            error: None,
        }
    }
}

impl<T> WorkspaceListEntry<T> {
    fn complete(&mut self, generation: u64, result: Result<T, WorkspaceListFailure>) -> bool {
        if self.generation != generation {
            return false;
        }
        match result {
            Ok(value) => {
                self.value = Some(value);
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
        true
    }

    pub fn state(&self, empty: bool) -> WorkspaceListState {
        match (self.loaded(), self.error.is_some(), empty) {
            (false, false, _) => WorkspaceListState::Loading,
            (false, true, _) => WorkspaceListState::InitialFailed,
            (true, true, _) => WorkspaceListState::RefreshFailed,
            (true, false, true) => WorkspaceListState::Empty,
            (true, false, false) => WorkspaceListState::Ready,
        }
    }

    pub fn value(&self) -> Option<&T> {
        self.value.as_ref()
    }
    pub fn loaded(&self) -> bool {
        self.value.is_some()
    }
    pub fn error(&self) -> Option<&str> {
        self.error.as_ref().map(|error| error.0.as_str())
    }
}

pub(crate) struct WorkspaceListRefresh<B, N> {
    generation: u64,
    snapshot_generation: u64,
    full_requested: u64,
    full_started: u64,
    full_completed: u64,
    repositories: WorkspaceListEntry<Vec<String>>,
    branches: HashMap<String, WorkspaceListEntry<(B, Vec<String>)>>,
    nodes: HashMap<String, WorkspaceListEntry<N>>,
}

impl<B, N> Default for WorkspaceListRefresh<B, N> {
    fn default() -> Self {
        Self {
            generation: 0,
            snapshot_generation: 0,
            full_requested: 0,
            full_started: 0,
            full_completed: 0,
            repositories: WorkspaceListEntry::default(),
            branches: HashMap::new(),
            nodes: HashMap::new(),
        }
    }
}

impl<B, N> WorkspaceListRefresh<B, N> {
    pub fn request_full(&mut self) -> u64 {
        self.full_requested = self.full_started + 1;
        self.full_requested
    }

    pub fn start_full(&mut self) -> Option<(u64, u64)> {
        if self.full_started != self.full_completed || self.full_requested == self.full_completed {
            return None;
        }
        self.full_started = self.full_requested;
        Some((self.full_started, self.begin()))
    }

    pub fn complete_full(&mut self, request: u64) {
        self.full_completed = request;
    }

    pub fn begin(&mut self) -> u64 {
        self.generation += 1;
        self.repositories.generation = self.generation;
        for list in self.branches.values_mut() {
            list.generation = self.generation;
        }
        for list in self.nodes.values_mut() {
            list.generation = self.generation;
        }
        self.generation
    }

    pub fn is_current(&self, generation: u64) -> bool {
        self.repositories.generation == generation
    }

    pub fn complete_repositories(
        &mut self,
        generation: u64,
        result: Result<Vec<String>, WorkspaceListFailure>,
    ) -> Vec<String> {
        if !self.is_current(generation) || !self.repositories.complete(generation, result) {
            return Vec::new();
        }
        let paths = self.repositories.value.clone().unwrap_or_default();
        self.branches.retain(|path, _| paths.contains(path));
        for path in &paths {
            let entry = self.branches.entry(path.clone()).or_default();
            entry.generation = entry.generation.max(generation);
        }
        self.release_removed_worktrees();
        paths
    }

    pub fn complete_branches(
        &mut self,
        path: &str,
        generation: u64,
        result: Result<(B, Vec<String>), WorkspaceListFailure>,
    ) -> Vec<String> {
        let Some(list) = self.branches.get_mut(path) else {
            return Vec::new();
        };
        if !list.complete(generation, result) {
            return Vec::new();
        }
        let paths = list
            .value
            .as_ref()
            .map(|value| value.1.clone())
            .unwrap_or_default();
        for path in &paths {
            self.nodes
                .entry(path.clone())
                .or_insert_with(|| WorkspaceListEntry {
                    generation,
                    ..Default::default()
                });
        }
        self.release_removed_worktrees();
        paths
    }

    pub fn update_branches(
        &mut self,
        path: &str,
        generation: u64,
        update: impl FnOnce(&mut B),
    ) -> bool {
        let Some(list) = self.branches.get_mut(path) else {
            return false;
        };
        if list.generation != generation {
            return false;
        }
        let Some((branches, _)) = list.value.as_mut() else {
            return false;
        };
        update(branches);
        true
    }

    pub fn begin_repository(&mut self, path: &str) -> Option<u64> {
        let list = self.branches.get_mut(path)?;
        self.generation += 1;
        list.generation = self.generation;
        if let Some((_, paths)) = &list.value {
            for path in paths {
                if let Some(nodes) = self.nodes.get_mut(path) {
                    nodes.generation = self.generation;
                }
            }
        }
        Some(self.generation)
    }

    pub fn is_worktree_current(&self, path: &str, generation: u64) -> bool {
        self.nodes
            .get(path)
            .is_some_and(|list| list.generation == generation)
    }

    pub fn begin_worktree(&mut self, path: &str) -> Option<u64> {
        let list = self.nodes.get_mut(path)?;
        self.generation += 1;
        list.generation = self.generation;
        Some(self.generation)
    }

    pub fn complete_worktree(
        &mut self,
        path: &str,
        generation: u64,
        result: Result<N, WorkspaceListFailure>,
    ) -> bool {
        self.nodes
            .get_mut(path)
            .is_some_and(|list| list.complete(generation, result))
    }

    fn release_removed_worktrees(&mut self) {
        let paths: HashSet<_> = self
            .branches
            .values()
            .filter_map(|list| list.value.as_ref())
            .flat_map(|(_, paths)| paths.iter())
            .collect();
        self.nodes.retain(|path, _| paths.contains(path));
    }

    pub fn next_snapshot_generation(&mut self) -> u64 {
        self.snapshot_generation += 1;
        self.snapshot_generation
    }

    pub fn repositories(&self) -> &WorkspaceListEntry<Vec<String>> {
        &self.repositories
    }
    pub fn branches(&self, path: &str) -> Option<&WorkspaceListEntry<(B, Vec<String>)>> {
        self.branches.get(path)
    }
    pub fn nodes(&self, path: &str) -> Option<&WorkspaceListEntry<N>> {
        self.nodes.get(path)
    }
}

#[cfg(test)]
#[path = "refresh_test.rs"]
mod refresh_tests;
