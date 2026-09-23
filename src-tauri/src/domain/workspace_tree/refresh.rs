use std::collections::{HashMap, HashSet};

#[derive(Debug)]
pub(crate) struct WorkspaceListEntry<T> {
    generation: u64,
    value: Option<T>,
    error: Option<String>,
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
    fn complete(&mut self, generation: u64, result: Result<T, String>) -> bool {
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

    pub fn value(&self) -> Option<&T> {
        self.value.as_ref()
    }
    pub fn loaded(&self) -> bool {
        self.value.is_some()
    }
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

pub(crate) struct WorkspaceListRefresh<B, N> {
    generation: u64,
    snapshot_generation: u64,
    repositories: WorkspaceListEntry<Vec<String>>,
    branches: HashMap<String, WorkspaceListEntry<(B, Vec<String>)>>,
    nodes: HashMap<String, WorkspaceListEntry<N>>,
}

impl<B, N> Default for WorkspaceListRefresh<B, N> {
    fn default() -> Self {
        Self {
            generation: 0,
            snapshot_generation: 0,
            repositories: WorkspaceListEntry::default(),
            branches: HashMap::new(),
            nodes: HashMap::new(),
        }
    }
}

impl<B, N> WorkspaceListRefresh<B, N> {
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
        result: Result<Vec<String>, String>,
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
        result: Result<(B, Vec<String>), String>,
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
        result: Result<N, String>,
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
