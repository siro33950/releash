use super::*;
use notify_debouncer_mini::DebouncedEventKind;

fn make_event(path: &str) -> DebouncedEvent {
    DebouncedEvent {
        path: PathBuf::from(path),
        kind: DebouncedEventKind::Any,
    }
}

#[test]
fn test_worktree登録変更_最初のdirectory作成と配下の追加削除を検出する() {
    for path in [
        "/repo/.git/worktrees",
        "/repo/.git/worktrees/first",
        "/repo/.git/worktrees/first/gitdir",
    ] {
        assert_eq!(classify_git_dir_events(&[make_event(path)]), (true, false));
    }
}

#[test]
fn classify_index_change() {
    let events = vec![make_event("/repo/.git/index")];
    let (branch, index) = classify_git_dir_events(&events);
    assert!(!branch);
    assert!(index);
}

#[test]
fn classify_index_lock() {
    let events = vec![make_event("/repo/.git/index.lock")];
    let (branch, index) = classify_git_dir_events(&events);
    assert!(!branch);
    assert!(index);
}

#[test]
fn classify_head_change() {
    let events = vec![make_event("/repo/.git/HEAD")];
    let (branch, index) = classify_git_dir_events(&events);
    assert!(branch);
    assert!(!index);
}

#[test]
fn classify_refs_heads_change() {
    let events = vec![make_event("/repo/.git/refs/heads/main")];
    let (branch, index) = classify_git_dir_events(&events);
    assert!(branch);
    assert!(!index);
}

#[test]
fn classify_commit_editmsg() {
    let events = vec![make_event("/repo/.git/COMMIT_EDITMSG")];
    let (branch, index) = classify_git_dir_events(&events);
    assert!(!branch);
    assert!(index);
}

#[test]
fn classify_mixed_events() {
    let events = vec![
        make_event("/repo/.git/refs/heads/feature"),
        make_event("/repo/.git/index"),
    ];
    let (branch, index) = classify_git_dir_events(&events);
    assert!(branch);
    assert!(index);
}

#[test]
fn classify_unrelated_event() {
    let events = vec![make_event("/repo/.git/config")];
    let (branch, index) = classify_git_dir_events(&events);
    assert!(!branch);
    assert!(!index);
}

#[test]
fn classify_worktree_index() {
    let events = vec![make_event("/repo/.git/worktrees/feat/index")];
    let (branch, index) = classify_git_dir_events(&events);
    assert!(!branch);
    assert!(index);
}
