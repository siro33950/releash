pub(crate) mod tests {
    use super::super::*;

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
