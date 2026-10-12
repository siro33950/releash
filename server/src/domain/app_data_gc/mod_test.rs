use super::*;
pub(crate) mod tests {
    use super::super::*;

    #[test]
    fn test_ui状態のgc判定_現存と両時点の保護と不足した前提は保持する() {
        // Given / When / Then
        assert!(workspace_state_removable(WorkspaceStateRetention {
            live: false,
            protected_at_scan: false,
            protected_at_sweep: false,
            prerequisites_complete: true
        }));
        for (live, scan, sweep, complete) in [
            (true, false, false, true),
            (false, true, false, true),
            (false, false, true, true),
            (false, false, false, false),
        ] {
            assert!(!workspace_state_removable(WorkspaceStateRetention {
                live,
                protected_at_scan: scan,
                protected_at_sweep: sweep,
                prerequisites_complete: complete
            }));
        }
    }

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

#[test]
fn test_gc報告の合算_片側と共通のカテゴリを足し上限で飽和する() {
    // Given
    let mut first = GcReport::default();
    first.record_deleted(GcCategory::DeletedWorkspace, 10);
    first.record_deleted(GcCategory::RegenerableCache, 20);
    first.errors = u64::MAX;
    let mut second = GcReport::default();
    second.record_deleted(GcCategory::DeletedWorkspace, 30);
    second.record_deleted(GcCategory::LegacyComments, 40);
    second.record_error();
    // When
    first.merge(second);
    // Then
    assert_eq!(first.total_files, 4);
    assert_eq!(first.total_bytes, 100);
    assert_eq!(first.errors, u64::MAX);
    assert_eq!(
        first.categories[&GcCategory::DeletedWorkspace],
        CategoryStat {
            deleted: 2,
            reclaimed_bytes: 40
        }
    );
    assert_eq!(first.categories[&GcCategory::RegenerableCache].deleted, 1);
    assert_eq!(first.categories[&GcCategory::LegacyComments].deleted, 1);
    let mut saturated = GcReport {
        total_files: u64::MAX,
        total_bytes: u64::MAX,
        errors: u64::MAX,
        ..Default::default()
    };
    saturated.categories.insert(
        GcCategory::DeletedWorkspace,
        CategoryStat {
            deleted: u64::MAX,
            reclaimed_bytes: u64::MAX,
        },
    );
    saturated.merge(first);
    assert_eq!(saturated.total_files, u64::MAX);
    assert_eq!(saturated.total_bytes, u64::MAX);
    assert_eq!(
        saturated.categories[&GcCategory::DeletedWorkspace],
        CategoryStat {
            deleted: u64::MAX,
            reclaimed_bytes: u64::MAX
        }
    );
}
