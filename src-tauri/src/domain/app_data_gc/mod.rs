use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum GcCategory {
    DeletedWorkspace,
    #[cfg(test)]
    UnrecoverableSession,
    #[cfg(test)]
    RecoverableExpired,
    RegenerableCache,
    LegacyComments,
    #[cfg(test)]
    OrphanBlob,
    StaleProcessRecord,
}

impl GcCategory {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::DeletedWorkspace => "deleted_workspace",
            #[cfg(test)]
            Self::UnrecoverableSession => "unrecoverable_session",
            #[cfg(test)]
            Self::RecoverableExpired => "recoverable_expired",
            Self::RegenerableCache => "regenerable_cache",
            Self::LegacyComments => "legacy_comments",
            #[cfg(test)]
            Self::OrphanBlob => "orphan_blob",
            Self::StaleProcessRecord => "stale_process_record",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RetentionPolicy {
    pub(crate) archived_log_secs: u64,
    pub(crate) cache_secs: u64,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            archived_log_secs: 30 * 24 * 60 * 60,
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
        stat.deleted += 1;
        stat.reclaimed_bytes += reclaimed_bytes;
        self.total_files += 1;
        self.total_bytes += reclaimed_bytes;
    }

    pub(crate) fn record_error(&mut self) {
        self.errors += 1;
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
    if !now_secs.is_finite() || !updated_at_secs.is_finite() {
        return false;
    }
    now_secs - updated_at_secs > threshold_secs as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_expired_uses_strict_boundary() {
        assert!(!is_expired(130.0, 100.0, 30));
        assert!(is_expired(130.001, 100.0, 30));
    }

    #[test]
    fn report_tracks_category_and_total() {
        let mut report = GcReport::default();
        report.record_deleted(GcCategory::LegacyComments, 10);
        report.record_deleted(GcCategory::LegacyComments, 5);
        report.record_deleted(GcCategory::OrphanBlob, 7);
        report.record_error();

        assert_eq!(report.total_files, 3);
        assert_eq!(report.total_bytes, 22);
        assert_eq!(report.errors, 1);
        assert_eq!(
            report.categories[&GcCategory::LegacyComments],
            CategoryStat {
                deleted: 2,
                reclaimed_bytes: 15
            }
        );
    }

    #[test]
    fn log_summary_includes_deleted_count_and_reclaimed_bytes() {
        let mut report = GcReport::default();
        report.record_deleted(GcCategory::LegacyComments, 10);
        report.record_deleted(GcCategory::OrphanBlob, 7);

        let summary = report.log_summary();

        assert_eq!(report.total_files, 2);
        assert_eq!(report.total_bytes, 17);
        assert!(summary.contains("deleted=2"));
        assert!(summary.contains("reclaimed_bytes=17"));
    }
}
