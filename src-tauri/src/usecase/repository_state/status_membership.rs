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
#[path = "status_membership_test.rs"]
mod status_membership_tests;
