use crate::usecase::repository_dto::FileStatusDto;

pub(crate) fn staged_statuses(status: &[FileStatusDto]) -> impl Iterator<Item = &FileStatusDto> {
    status.iter().filter(|entry| is_staged_status(entry))
}

pub(crate) fn changed_statuses(status: &[FileStatusDto]) -> impl Iterator<Item = &FileStatusDto> {
    status.iter().filter(|entry| is_changed_status(entry))
}

pub(crate) fn split_staged_changed_statuses(
    status: &[FileStatusDto],
) -> (Vec<FileStatusDto>, Vec<FileStatusDto>) {
    (
        staged_statuses(status).cloned().collect(),
        changed_statuses(status).cloned().collect(),
    )
}

fn is_staged_status(entry: &FileStatusDto) -> bool {
    entry.index_status != "none"
}

fn is_changed_status(entry: &FileStatusDto) -> bool {
    entry.worktree_status != "none" && entry.worktree_status != "ignored"
}

#[cfg(test)]
mod tests {
    use super::*;

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
