pub(crate) mod tests {

    use releashd::test_support::integration::platform::filter_missing_files;
    use releashd::test_support::integration::platform::WorkspaceLayoutState;
    use releashd::test_support::integration::platform::WorkspaceState;
    use releashd::test_support::integration::platform::WorkspaceTabEntry;
    use releashd::test_support::integration::platform::WorkspaceTabsState;

    fn make_state() -> WorkspaceState {
        WorkspaceState {
            panes: None,
            version: 1,
            tabs: Some(WorkspaceTabsState {
                editors: vec![
                    WorkspaceTabEntry {
                        path: "src/main.rs".to_string(),
                        name: "main.rs".to_string(),
                    },
                    WorkspaceTabEntry {
                        path: "src/lib.rs".to_string(),
                        name: "lib.rs".to_string(),
                    },
                ],
            }),
            layout: Some(WorkspaceLayoutState {
                center_tab: "editor".to_string(),
                active_view: "git".to_string(),
                left_nav_collapsed: false,
                right_collapsed: false,
                right_bottom_collapsed: false,
            }),
        }
    }

    #[test]
    pub fn filters_deleted_files() {
        let dir = tempfile::TempDir::new().unwrap();
        let worktree = dir.path().join("worktree");
        std::fs::create_dir_all(worktree.join("src")).unwrap();
        std::fs::write(worktree.join("src/lib.rs"), "// lib").unwrap();

        let state = filter_missing_files(make_state(), |path| worktree.join(path).exists());

        assert_eq!(state.tabs.as_ref().unwrap().editors.len(), 1);
    }
}
