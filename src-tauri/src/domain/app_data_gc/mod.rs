use std::collections::BTreeMap;

/// Application-data families that remain eligible for GC after the canonical
/// SQLite cutover.
///
/// Agent Session and Workflow file-store families deliberately do not appear
/// here. They are legacy sources under issue #1499 B-070 and are never GC
/// inputs, even when they coexist with the fixed SQLite store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum GcCategory {
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
pub(crate) struct RetentionPolicy {
    pub(crate) cache_secs: u64,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            cache_secs: 7 * 24 * 60 * 60,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CategoryStat {
    pub(crate) deleted: u64,
    pub(crate) reclaimed_bytes: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct GcReport {
    pub(crate) categories: BTreeMap<GcCategory, CategoryStat>,
    pub(crate) total_files: u64,
    pub(crate) total_bytes: u64,
    pub(crate) errors: u64,
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
mod tests {
    use super::*;

    #[test]
    fn test_旧実行木の所属repo_標準worktree配置から対象だけを特定する() {
        // Given
        let repositories = vec!["/repos/a".to_string(), "/repos/b".to_string()];
        let live = std::collections::HashSet::new();
        // When
        let owner =
            repository_for_worktree("/repos/a-worktrees/feature", "/isolated", &repositories);
        // Then
        assert_eq!(owner, Some("/repos/a"));
        assert!(worktree_removed(
            "/repos/a-worktrees/feature",
            owner,
            &live,
            &["/repos/b".into()]
        ));
        assert!(!worktree_removed(
            "/repos/a-worktrees/feature",
            owner,
            &live,
            &["/repos/a".into()]
        ));
        assert_eq!(
            repository_for_worktree(
                "/repos/a-worktrees-other/feature",
                "/unknown",
                &repositories
            ),
            None
        );
    }

    #[test]
    fn test_worktree消失判定_登録済みと読めないリポジトリを保持する() {
        let live = std::collections::HashSet::from(["/live".to_string()]);
        let unresolved = vec!["/unreadable".to_string()];
        assert!(!worktree_removed("/live", Some("/repo"), &live, &[]));
        assert!(worktree_removed("/gone", Some("/repo"), &live, &unresolved));
        assert!(!worktree_removed(
            "/gone",
            Some("/unreadable"),
            &live,
            &unresolved
        ));
        assert!(!worktree_removed("/gone", None, &live, &unresolved));
        assert!(worktree_removed("/gone", None, &live, &[]));
    }

    #[test]
    fn retention_uses_the_strict_seven_day_boundary() {
        assert!(!is_expired(604_800.0, 0.0, 604_800));
        assert!(is_expired(604_800.001, 0.0, 604_800));
        assert!(!is_expired(f64::NAN, 0.0, 604_800));
    }

    #[test]
    fn report_keeps_category_and_total_accounting() {
        let mut report = GcReport::default();
        report.record_deleted(GcCategory::RegenerableCache, 10);
        report.record_deleted(GcCategory::RegenerableCache, 5);
        report.record_error();

        assert_eq!(report.total_files, 2);
        assert_eq!(report.total_bytes, 15);
        assert_eq!(report.errors, 1);
        assert_eq!(
            report.categories[&GcCategory::RegenerableCache],
            CategoryStat {
                deleted: 2,
                reclaimed_bytes: 15,
            }
        );
        assert!(report.log_summary().contains("reclaimed_bytes=15"));
    }
}
