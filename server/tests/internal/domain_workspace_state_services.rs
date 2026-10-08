pub(crate) mod tests {

    use releashd::test_support::integration::platform::filter_missing_files;
    use releashd::test_support::integration::platform::WorkspaceLayoutState;
    use releashd::test_support::integration::platform::WorkspaceState;
    use releashd::test_support::integration::platform::WorkspaceTabEntry;
    use releashd::test_support::integration::platform::WorkspaceTabsState;

    fn make_state() -> WorkspaceState {
        WorkspaceState {
            version: 1,
            tabs: WorkspaceTabsState {
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
                active_editor_path: Some("src/main.rs".to_string()),
            },
            layout: WorkspaceLayoutState {
                center_tab: "editor".to_string(),
                active_view: "git".to_string(),
                left_nav_collapsed: false,
                right_collapsed: false,
                right_bottom_collapsed: false,
                right_bottom_active_tab: None,
                selected_diff_file: Some("src/deleted.rs".to_string()),
            },
        }
    }

    #[test]
    pub fn filters_deleted_files_and_falls_back_active_path() {
        let dir = tempfile::TempDir::new().unwrap();
        let worktree = dir.path().join("worktree");
        std::fs::create_dir_all(worktree.join("src")).unwrap();
        std::fs::write(worktree.join("src/lib.rs"), "// lib").unwrap();

        let state = filter_missing_files(make_state(), worktree.to_str().unwrap());

        assert_eq!(state.tabs.editors.len(), 1);
        assert_eq!(state.tabs.active_editor_path.as_deref(), Some("src/lib.rs"));
        assert_eq!(state.layout.selected_diff_file, None);
    }
}
