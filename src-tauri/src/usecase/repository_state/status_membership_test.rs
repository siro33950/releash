pub(crate) mod tests {
    use super::super::*;

    fn status(path: &str, index_status: &str, worktree_status: &str) -> FileStatusDto {
        FileStatusDto {
            path: path.to_string(),
            index_status: index_status.to_string(),
            worktree_status: worktree_status.to_string(),
        }
    }

    #[test]
    fn staged_and_changed_membership_share_file_status_rules() {
        let statuses = vec![
            status("staged.rs", "modified", "none"),
            status("changed.rs", "none", "modified"),
            status("both.rs", "new", "deleted"),
            status("ignored.rs", "none", "ignored"),
            status("clean.rs", "none", "none"),
        ];

        let (staged, changed) = split_staged_changed_statuses(&statuses);

        assert_eq!(
            staged
                .iter()
                .map(|entry| entry.path.as_str())
                .collect::<Vec<_>>(),
            vec!["staged.rs", "both.rs"]
        );
        assert_eq!(
            changed
                .iter()
                .map(|entry| entry.path.as_str())
                .collect::<Vec<_>>(),
            vec!["changed.rs", "both.rs"]
        );
    }
}
