use std::collections::BTreeMap;

/// Application-data families that remain eligible for GC after the canonical
/// SQLite cutover.
///
/// Agent Session and Workflow file-store families deliberately do not appear
/// here. They are legacy sources under issue #1499 B-070 and are never GC
/// inputs, even when they coexist with the fixed SQLite store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum GcCategory {
    DeletedWorkspace,
    RegenerableCache,
    LegacyComments,
}

impl GcCategory {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::DeletedWorkspace => "deleted_workspace",
            Self::RegenerableCache => "regenerable_cache",
            Self::LegacyComments => "legacy_comments",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionPolicy {
    pub cache_secs: u64,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            cache_secs: 7 * 24 * 60 * 60,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CategoryStat {
    pub deleted: u64,
    pub(crate) reclaimed_bytes: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GcReport {
    pub categories: BTreeMap<GcCategory, CategoryStat>,
    pub(crate) total_files: u64,
    pub(crate) total_bytes: u64,
    pub errors: u64,
}

impl GcReport {
    pub(crate) fn record_deleted(&mut self, category: GcCategory, reclaimed_bytes: u64) {
        let stat = self.categories.entry(category).or_default();
        stat.deleted = stat.deleted.saturating_add(1);
        stat.reclaimed_bytes = stat.reclaimed_bytes.saturating_add(reclaimed_bytes);
        self.total_files = self.total_files.saturating_add(1);
        self.total_bytes = self.total_bytes.saturating_add(reclaimed_bytes);
    }

    pub(crate) fn record_error(&mut self) {
        self.errors = self.errors.saturating_add(1);
    }

    pub(crate) fn log_summary(&self) -> String {
        let categories = self
            .categories
            .iter()
            .map(|(category, stat)| {
                format!(
                    "{}:deleted={},bytes={}",
                    category.as_str(),
                    stat.deleted,
                    stat.reclaimed_bytes
                )
            })
            .collect::<Vec<_>>()
            .join(" ");
        format!(
            "app data gc deleted={} reclaimed_bytes={} errors={} categories=[{}]",
            self.total_files, self.total_bytes, self.errors, categories
        )
    }
}

pub(crate) fn is_expired(now_secs: f64, updated_at_secs: f64, threshold_secs: u64) -> bool {
    now_secs.is_finite()
        && updated_at_secs.is_finite()
        && now_secs - updated_at_secs > threshold_secs as f64
}

pub(crate) fn repository_for_worktree<'a>(
    workspace: &str,
    worktree: &str,
    repositories: &'a [String],
) -> Option<&'a str> {
    repositories
        .iter()
        .find(|repository| {
            let directory = crate::domain::repository::worktree_dir(repository);
            [workspace, worktree].iter().any(|path| {
                *path == repository.as_str()
                    || path
                        .strip_prefix(&directory)
                        .is_some_and(|suffix| suffix.starts_with('/'))
            })
        })
        .map(String::as_str)
}

pub(crate) fn worktree_removed(
    worktree_path: &str,
    repository_root: Option<&str>,
    live_paths: &std::collections::HashSet<String>,
    unresolved_repositories: &[String],
) -> bool {
    !live_paths.contains(worktree_path)
        && match repository_root {
            Some(root) => !unresolved_repositories
                .iter()
                .any(|unresolved| unresolved == root),
            None => unresolved_repositories.is_empty(),
        }
}

#[cfg(test)]
#[path = "mod_test.rs"]
pub(crate) mod mod_tests;
