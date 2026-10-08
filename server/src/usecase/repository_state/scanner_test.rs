pub(crate) mod tests {
    use super::super::*;

    #[test]
    fn diff_tree_entries_use_index_status_before_worktree_status() {
        let status = vec![FileStatusDto {
            path: "src/lib.rs".to_string(),
            index_status: "modified".to_string(),
            worktree_status: "deleted".to_string(),
        }];
        let stats = vec![FileDiffStatDto {
            path: "src/lib.rs".to_string(),
            index_additions: 2,
            index_deletions: 1,
            wt_additions: 3,
            wt_deletions: 4,
        }];

        let entries = diff_tree_entries(&status, &stats);

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, "modified");
        assert_eq!(entries[0].additions, 5);
        assert_eq!(entries[0].deletions, 5);
    }

    #[test]
    fn diff_tree_entries_skip_ignored_status() {
        let status = vec![FileStatusDto {
            path: "target".to_string(),
            index_status: "none".to_string(),
            worktree_status: "ignored".to_string(),
        }];

        assert!(diff_tree_entries(&status, &[]).is_empty());
    }

    #[test]
    fn staged_and_changes_entries_are_split_from_same_status_and_stats() {
        let status = vec![
            FileStatusDto {
                path: "both.rs".to_string(),
                index_status: "modified".to_string(),
                worktree_status: "deleted".to_string(),
            },
            FileStatusDto {
                path: "ignored".to_string(),
                index_status: "none".to_string(),
                worktree_status: "ignored".to_string(),
            },
        ];
        let stats = vec![FileDiffStatDto {
            path: "both.rs".to_string(),
            index_additions: 2,
            index_deletions: 1,
            wt_additions: 0,
            wt_deletions: 4,
        }];

        let staged = staged_diff_tree_entries(&status, &stats);
        let changes = changes_diff_tree_entries(&status, &stats);

        assert_eq!(staged.len(), 1);
        assert_eq!(staged[0].status, "modified");
        assert_eq!(staged[0].additions, 2);
        assert_eq!(staged[0].deletions, 1);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].status, "deleted");
        assert_eq!(changes[0].additions, 0);
        assert_eq!(changes[0].deletions, 4);
    }
}
