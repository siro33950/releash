use std::collections::BTreeMap;

use crate::domain::repository::{normalize_repo_path, worktree_dir};

use super::{IsolatedWorktreeCreatedFact, NodeFact, NodeFactMeta, NodeFactRecord};

const ISOLATED_BRANCH_PREFIX: &str = "releash/isolated/";
const ISOLATED_DIRECTORY: &str = ".releash-isolated";

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IsolatedWorktreeIdentity {
    pub tree_id: String,
    pub node_execution_id: String,
    pub attempt: u32,
}

impl IsolatedWorktreeIdentity {
    pub fn from_meta(meta: &NodeFactMeta) -> Self {
        Self {
            tree_id: meta.tree_id.clone(),
            node_execution_id: meta.node_execution_id.clone(),
            attempt: meta.attempt,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsolatedWorktreeLifecycle {
    Created,
    Released,
    Lost,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IsolatedWorktreeLedgerEntry {
    pub owner: NodeFactMeta,
    pub repository_root: String,
    pub worktree_path: String,
    pub branch: String,
    pub lifecycle: IsolatedWorktreeLifecycle,
}

impl IsolatedWorktreeLedgerEntry {
    pub fn identity(&self) -> IsolatedWorktreeIdentity {
        IsolatedWorktreeIdentity::from_meta(&self.owner)
    }

    pub fn recovery_cause(&self) -> Option<IsolatedWorktreeRecoveryCause> {
        (self.lifecycle == IsolatedWorktreeLifecycle::Lost)
            .then(|| IsolatedWorktreeRecoveryCause::new(self.worktree_path.clone()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IsolatedWorktreeLedgerSnapshot {
    entries: BTreeMap<IsolatedWorktreeIdentity, IsolatedWorktreeLedgerEntry>,
}

impl IsolatedWorktreeLedgerSnapshot {
    pub fn from_records(records: &[NodeFactRecord]) -> Result<Self, String> {
        let mut snapshot = Self::default();
        for record in records {
            snapshot.apply_record(record)?;
        }
        Ok(snapshot)
    }

    pub fn entries(&self) -> impl Iterator<Item = &IsolatedWorktreeLedgerEntry> {
        self.entries.values()
    }

    pub fn entry(
        &self,
        identity: &IsolatedWorktreeIdentity,
    ) -> Option<&IsolatedWorktreeLedgerEntry> {
        self.entries.get(identity)
    }

    pub fn entry_for_path(
        &self,
        repository_root: &str,
        worktree_path: &str,
    ) -> Option<&IsolatedWorktreeLedgerEntry> {
        let repository_root = normalize_repo_path(repository_root);
        let worktree_path = normalize_repo_path(worktree_path);
        self.entries.values().find(|entry| {
            entry.repository_root == repository_root && entry.worktree_path == worktree_path
        })
    }

    pub fn recovery_cause(
        &self,
        identity: &IsolatedWorktreeIdentity,
    ) -> Option<IsolatedWorktreeRecoveryCause> {
        self.entry(identity)
            .and_then(IsolatedWorktreeLedgerEntry::recovery_cause)
    }

    pub fn recovery_cause_for_node(
        &self,
        tree_id: &str,
        node_execution_id: &str,
    ) -> Option<IsolatedWorktreeRecoveryCause> {
        self.entries.values().find_map(|entry| {
            (entry.owner.tree_id == tree_id && entry.owner.node_execution_id == node_execution_id)
                .then(|| entry.recovery_cause())
                .flatten()
        })
    }

    pub fn recovery_cause_for_tree(&self, tree_id: &str) -> Option<IsolatedWorktreeRecoveryCause> {
        self.entries.values().find_map(|entry| {
            (entry.owner.tree_id == tree_id)
                .then(|| entry.recovery_cause())
                .flatten()
        })
    }

    pub fn merge(&mut self, other: &Self) -> Result<(), String> {
        for entry in other.entries() {
            let identity = entry.identity();
            if self
                .entries
                .insert(identity.clone(), entry.clone())
                .is_some()
            {
                return Err(format!(
                    "duplicate isolated worktree owner {} attempt {} in tree {}",
                    identity.node_execution_id, identity.attempt, identity.tree_id
                ));
            }
        }
        Ok(())
    }

    pub fn apply_record(&mut self, record: &NodeFactRecord) -> Result<(), String> {
        let identity = IsolatedWorktreeIdentity::from_meta(&record.meta);
        match &record.fact {
            NodeFact::IsolatedWorktreeCreated(fact) => {
                let entry = entry_from_created(&record.meta, fact);
                match self.entries.get(&identity) {
                    Some(existing) if existing == &entry => Ok(()),
                    Some(_) => Err(format!(
                        "isolated worktree owner {} attempt {} has conflicting creation facts",
                        identity.node_execution_id, identity.attempt
                    )),
                    None => {
                        self.entries.insert(identity, entry);
                        Ok(())
                    }
                }
            }
            NodeFact::IsolatedWorktreeReleased => {
                self.transition(&identity, IsolatedWorktreeLifecycle::Released)
            }
            NodeFact::IsolatedWorktreeLost => {
                self.transition(&identity, IsolatedWorktreeLifecycle::Lost)
            }
            _ => Ok(()),
        }
    }

    fn transition(
        &mut self,
        identity: &IsolatedWorktreeIdentity,
        lifecycle: IsolatedWorktreeLifecycle,
    ) -> Result<(), String> {
        let entry = self.entries.get_mut(identity).ok_or_else(|| {
            format!(
                "isolated worktree {} fact precedes its creation fact",
                match lifecycle {
                    IsolatedWorktreeLifecycle::Created => "creation",
                    IsolatedWorktreeLifecycle::Released => "release",
                    IsolatedWorktreeLifecycle::Lost => "loss",
                }
            )
        })?;
        if entry.lifecycle == lifecycle {
            return Ok(());
        }
        if entry.lifecycle != IsolatedWorktreeLifecycle::Created {
            return Err(format!(
                "isolated worktree owner {} attempt {} cannot transition from {:?} to {:?}",
                identity.node_execution_id, identity.attempt, entry.lifecycle, lifecycle
            ));
        }
        entry.lifecycle = lifecycle;
        Ok(())
    }
}

fn entry_from_created(
    meta: &NodeFactMeta,
    fact: &IsolatedWorktreeCreatedFact,
) -> IsolatedWorktreeLedgerEntry {
    IsolatedWorktreeLedgerEntry {
        owner: meta.clone(),
        repository_root: normalize_repo_path(&fact.repository_root),
        worktree_path: normalize_repo_path(&fact.worktree_path),
        branch: fact.branch.clone(),
        lifecycle: IsolatedWorktreeLifecycle::Created,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeInventoryEntry {
    pub repository_root: String,
    pub worktree_path: String,
    pub branch: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryWorktreeInventory {
    pub repository_root: String,
    pub worktrees: Vec<WorktreeInventoryEntry>,
}

impl RepositoryWorktreeInventory {
    pub fn new(repository_root: impl AsRef<str>, worktrees: Vec<WorktreeInventoryEntry>) -> Self {
        let repository_root = normalize_repo_path(repository_root.as_ref());
        debug_assert!(worktrees
            .iter()
            .all(|worktree| worktree.repository_root == repository_root));
        Self {
            repository_root,
            worktrees,
        }
    }
}

impl WorktreeInventoryEntry {
    pub fn new(
        repository_root: impl AsRef<str>,
        worktree_path: impl AsRef<str>,
        branch: impl Into<String>,
    ) -> Self {
        Self {
            repository_root: normalize_repo_path(repository_root.as_ref()),
            worktree_path: normalize_repo_path(worktree_path.as_ref()),
            branch: branch.into(),
        }
    }

    pub fn matches_isolated_identity_rule(&self) -> bool {
        let Some(token) = self.branch.strip_prefix(ISOLATED_BRANCH_PREFIX) else {
            return false;
        };
        let Some((node_execution_id, attempt)) = parse_identity_token(token) else {
            return false;
        };
        self.branch == isolated_worktree_branch(node_execution_id, attempt)
            && self.worktree_path
                == isolated_worktree_path(&self.repository_root, node_execution_id, attempt)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorktreeManagementKind {
    WorkingArea,
    IsolatedOwned,
    CleanupCandidate,
    UntrackedCleanupCandidate,
}

impl WorktreeManagementKind {
    pub fn as_public_str(self) -> &'static str {
        match self {
            Self::WorkingArea => "working_area",
            Self::IsolatedOwned => "isolated_owned",
            Self::CleanupCandidate => "cleanup_candidate",
            Self::UntrackedCleanupCandidate => "untracked_cleanup_candidate",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IsolatedWorktreeRecoveryCause {
    worktree_path: String,
}

impl IsolatedWorktreeRecoveryCause {
    pub fn new(worktree_path: impl AsRef<str>) -> Self {
        Self {
            worktree_path: normalize_repo_path(worktree_path.as_ref()),
        }
    }
}

impl std::fmt::Display for IsolatedWorktreeRecoveryCause {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "isolated worktree is missing: {}",
            self.worktree_path
        )
    }
}

pub fn isolated_worktree_identity_token(node_execution_id: &str, attempt: u32) -> String {
    format!("{node_execution_id}-a{attempt}")
}

pub fn isolated_worktree_path(
    repository_root: &str,
    node_execution_id: &str,
    attempt: u32,
) -> String {
    isolated_worktree_path_for_token(
        repository_root,
        &isolated_worktree_identity_token(node_execution_id, attempt),
    )
}

pub fn isolated_worktree_branch(node_execution_id: &str, attempt: u32) -> String {
    format!(
        "{ISOLATED_BRANCH_PREFIX}{}",
        isolated_worktree_identity_token(node_execution_id, attempt)
    )
}

fn isolated_worktree_path_for_token(repository_root: &str, token: &str) -> String {
    normalize_repo_path(&format!(
        "{}/{ISOLATED_DIRECTORY}/{token}",
        worktree_dir(repository_root)
    ))
}

fn parse_identity_token(token: &str) -> Option<(&str, u32)> {
    let (node_execution_id, attempt) = token.rsplit_once("-a")?;
    if node_execution_id.is_empty() {
        return None;
    }
    let attempt = attempt.parse().ok()?;
    Some((node_execution_id, attempt))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_identity_uses_sibling_isolated_directory_and_branch() {
        assert_eq!(
            isolated_worktree_path("/projects/repo", "node-1", 2),
            "/projects/repo-worktrees/.releash-isolated/node-1-a2"
        );
        assert_eq!(
            isolated_worktree_branch("node-1", 2),
            "releash/isolated/node-1-a2"
        );
    }

    #[test]
    fn fallback_requires_matching_canonical_path_and_branch_token() {
        let canonical = WorktreeInventoryEntry::new(
            "/projects/repo",
            "/projects/repo-worktrees/.releash-isolated/node-1-a2",
            "releash/isolated/node-1-a2",
        );
        assert!(canonical.matches_isolated_identity_rule());

        let wrong_branch = WorktreeInventoryEntry::new(
            "/projects/repo",
            &canonical.worktree_path,
            "feature/node-1-a2",
        );
        assert!(!wrong_branch.matches_isolated_identity_rule());

        let wrong_path = WorktreeInventoryEntry::new(
            "/projects/repo",
            "/projects/repo-worktrees/node-1-a2",
            &canonical.branch,
        );
        assert!(!wrong_path.matches_isolated_identity_rule());
    }
}
