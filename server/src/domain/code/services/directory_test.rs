use super::*;

#[test]
fn test_ディレクトリ状態_子の変更の優先度で決まる() {
    // Given
    let statuses = [
        GitStatus::New,
        GitStatus::Modified,
        GitStatus::Deleted,
        GitStatus::Conflicted,
    ];
    // When / Then
    for count in 0..=statuses.len() {
        assert_eq!(
            aggregate_status(statuses[..count].iter().copied()),
            [
                GitStatus::None,
                GitStatus::New,
                GitStatus::Modified,
                GitStatus::Deleted,
                GitStatus::Conflicted
            ][count]
        );
    }
}

#[test]
fn test_ディレクトリ状態_renameも変更として色を付ける() {
    // Given / When / Then
    assert_eq!(
        aggregate_status([GitStatus::None, GitStatus::Renamed].into_iter()),
        GitStatus::Modified
    );
}

#[test]
fn test_worktree相対path_通常名と現在位置だけを受け付けgitを隠す() {
    // Given / When / Then
    for path in [".", "src/file", "src/./file", ".gitignore"] {
        assert!(worktree_path_is_visible(path));
    }
    for path in ["/tmp", "../file", "src/../file", ".git", "src/.git/config"] {
        assert!(!worktree_path_is_visible(path));
    }
}

#[test]
fn test_ディレクトリ状態_同じpathと子孫だけを集約する() {
    // Given
    let statuses = [
        ("src/a", GitStatus::None, GitStatus::New),
        ("src2/b", GitStatus::Conflicted, GitStatus::None),
        ("src", GitStatus::Modified, GitStatus::None),
    ];
    // When / Then
    assert_eq!(
        entry_status("src", true, statuses.into_iter()),
        GitStatus::Modified
    );
    assert_eq!(
        entry_status("src/a", false, statuses.into_iter()),
        GitStatus::New
    );
    assert_eq!(
        entry_status("src", false, statuses.into_iter()),
        GitStatus::Modified
    );
    assert_eq!(
        entry_status("other", true, statuses.into_iter()),
        GitStatus::None
    );
}

#[test]
fn test_直下一覧_ディレクトリを先にして名前順にする() {
    // Given
    let mut entries = [(false, "a"), (true, "z"), (true, "b"), (false, "c")];
    // When
    entries.sort_by(|a, b| entry_order(a.0, a.1, b.0, b.1));
    // Then
    assert_eq!(
        entries,
        [(true, "b"), (true, "z"), (false, "a"), (false, "c")]
    );
}

#[test]
fn test_git状態_既知の状態を型に変換し未知の状態を拒否する() {
    // Given / When / Then
    for (value, expected) in [
        ("none", GitStatus::None),
        ("new", GitStatus::New),
        ("modified", GitStatus::Modified),
        ("renamed", GitStatus::Renamed),
        ("deleted", GitStatus::Deleted),
        ("conflicted", GitStatus::Conflicted),
    ] {
        assert_eq!(GitStatus::parse(value).unwrap(), expected);
    }
    assert!(GitStatus::parse("unknown").is_err());
}
