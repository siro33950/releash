use super::*;

#[test]
fn test_旧表示状態の移行先_hashキーを優先し末尾名が一意なworktreeだけ選ぶ() {
    // Given
    let candidates = vec![
        ("/repo/feature/a".into(), "hash-one".into()),
        ("/repo/bugfix/a".into(), "hash-two".into()),
        ("/repo/unique".into(), "hash-three".into()),
    ];
    // When / Then
    assert_eq!(
        legacy_state_path("hash-one", &candidates),
        Some("/repo/feature/a")
    );
    assert_eq!(
        legacy_state_path("unique", &candidates),
        Some("/repo/unique")
    );
    assert_eq!(legacy_state_path("a", &candidates), None);
    assert_eq!(legacy_state_path("missing", &candidates), None);
    assert_eq!(legacy_state_path("unique", &[]), None);
    assert_eq!(legacy_state_path("_repo_feature_a", &candidates), None);
    let distinct = vec![
        ("/repo/_repo_a".into(), "one".into()),
        ("/repo/a".into(), "two".into()),
    ];
    assert_eq!(
        legacy_state_path("_repo_a", &distinct),
        Some("/repo/_repo_a")
    );
    let ambiguous = vec![
        ("/repo/a/b".into(), "one".into()),
        ("/repo/a_b".into(), "two".into()),
    ];
    assert_eq!(legacy_state_path("_repo_a_b", &ambiguous), None);
}

#[test]
fn test_旧表示状態の移行先_未解決repositoryがあれば名前の解決を見送る() {
    // Given / When / Then
    assert_eq!(
        migration_worktree_paths(vec![
            Some(vec!["/repo/a".into()]),
            Some(vec!["/other/b".into()])
        ]),
        vec!["/repo/a", "/other/b"]
    );
    assert!(migration_worktree_paths(vec![Some(vec!["/repo/a".into()]), None]).is_empty());
}

#[test]
fn test_表示状態のファイル整理_消えた選択を直し全件消失なら選択を空にする() {
    use crate::domain::workspace_state::value_objects::{
        workspace_tabs_state::WorkspaceTabEntry, WorkspaceLayoutState, WorkspaceTabsState,
    };
    // Given
    let state = WorkspaceState {
        panes: None,
        version: 1,
        tabs: WorkspaceTabsState {
            editors: ["gone", "live"]
                .map(|path| WorkspaceTabEntry {
                    path: path.into(),
                    name: path.into(),
                })
                .into(),
            active_editor_path: Some("gone".into()),
        },
        layout: WorkspaceLayoutState {
            center_tab: "editor".into(),
            active_view: "git".into(),
            left_nav_collapsed: false,
            right_collapsed: false,
            right_bottom_collapsed: false,
            right_bottom_active_tab: None,
            selected_diff_file: Some("gone".into()),
        },
    };
    // When / Then
    let filtered = filter_missing_files(state.clone(), |path| path == "live");
    assert_eq!(filtered.tabs.editors.len(), 1);
    assert_eq!(filtered.tabs.active_editor_path.as_deref(), Some("live"));
    assert!(filtered.layout.selected_diff_file.is_none());
    let empty = filter_missing_files(state.clone(), |_| false);
    assert!(empty.tabs.editors.is_empty());
    assert!(empty.tabs.active_editor_path.is_none());
    assert_eq!(filter_missing_files(state.clone(), |_| true), state);
}
