use super::*;

fn status(path: &str, index_status: &str, worktree_status: &str) -> FileStatusDto {
    FileStatusDto {
        path: path.to_string(),
        index_status: index_status.to_string(),
        worktree_status: worktree_status.to_string(),
    }
}

fn node(path: &str) -> DiffTreeNodeDto {
    DiffTreeNodeDto {
        id: path.to_string(),
        name: path.to_string(),
        path: path.to_string(),
        node_type: "file".to_string(),
        status: Some("modified".to_string()),
        additions: Some(1),
        deletions: Some(0),
        children: Vec::new(),
    }
}

fn parts() -> RepositorySnapshotParts {
    RepositorySnapshotParts {
        status: Vec::new(),
        diff_stats: Vec::new(),
        branch_cards: Vec::new(),
        diff_file_tree: vec![node("combined.rs")],
        staged_diff_file_tree: vec![node("staged.rs")],
        changes_diff_file_tree: vec![node("changes.rs")],
    }
}

#[test]
fn head_diff_file_tree_dto_exposes_combined_tree_from_snapshot() {
    let snapshot = parts().into_snapshot(7);

    let dto = RepositoryHeadDiffFileTreeSnapshotDto::from_snapshot(&snapshot);

    assert_eq!(dto.version, 7);
    assert_eq!(dto.combined_tree.len(), 1);
    assert_eq!(dto.combined_tree[0].path, "combined.rs");
    assert_eq!(dto.staged_tree[0].path, "staged.rs");
    assert_eq!(dto.changes_tree[0].path, "changes.rs");
}

#[test]
fn head_diff_file_tree_counts_staged_changes_and_ignored_boundaries() {
    let mut snapshot = parts().into_snapshot(3);
    snapshot.status = vec![
        status("staged-only.rs", "modified", "none"),
        status("changes-only.rs", "none", "modified"),
        status("both.rs", "new", "deleted"),
        status("ignored", "none", "ignored"),
        status("clean.rs", "none", "none"),
    ];

    let dto = RepositoryHeadDiffFileTreeSnapshotDto::from_snapshot(&snapshot);

    assert_eq!(dto.staged_file_count, 2);
    assert_eq!(dto.changes_file_count, 2);
}

#[test]
fn test_snapshot公開形式_limitedを含まずstaleとloadingを保持する() {
    // Given
    let snapshot = RepositorySnapshot::loading().with_read_flags(true, true);

    // When
    let values = [
        serde_json::to_value(&snapshot.flags).unwrap(),
        serde_json::to_value(RepositoryStatusSnapshotDto::from_snapshot(&snapshot)).unwrap(),
        serde_json::to_value(RepositoryDiffStatsSnapshotDto::from_snapshot(&snapshot)).unwrap(),
        serde_json::to_value(RepositoryBranchCardsSnapshotDto::from_snapshot(&snapshot)).unwrap(),
        serde_json::to_value(RepositoryHeadDiffFileTreeSnapshotDto::from_snapshot(
            &snapshot,
        ))
        .unwrap(),
    ];

    // Then
    for value in values {
        assert!(value.get("limited").is_none());
        assert_eq!(value["stale"], true);
        assert_eq!(value["loading"], true);
    }
}
