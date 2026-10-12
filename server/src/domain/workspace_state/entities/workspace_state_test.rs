use super::*;
use crate::domain::workspace_state::value_objects::pane_layout::PaneLayout;

#[test]
fn test_配置の保存_省略だけ前の配置を保持し明示した空の配置で更新する() {
    // Given
    let original = WorkspaceState {
        panes: Some(PaneLayout::pane("original".into(), vec![], None).unwrap()),
        version: 1,
        tabs: WorkspaceTabsState {
            editors: vec![],
            active_editor_path: None,
        },
        layout: WorkspaceLayoutState {
            center_tab: "editor".into(),
            active_view: "git".into(),
            left_nav_collapsed: false,
            right_collapsed: false,
            right_bottom_collapsed: false,
            right_bottom_active_tab: None,
            selected_diff_file: None,
        },
    };
    let mut update = original.clone();
    update.panes = None;
    update.layout.left_nav_collapsed = true;
    // When / Then
    let saved = update
        .clone()
        .preserve_omitted_panes(Some(original.clone()));
    assert_eq!(saved.panes, original.panes);
    assert!(saved.layout.left_nav_collapsed);
    assert!(update.preserve_omitted_panes(None).panes.is_none());
    let empty = PaneLayout::pane("empty".into(), vec![], None).unwrap();
    let mut update = original.clone();
    update.panes = Some(empty.clone());
    assert_eq!(
        update.preserve_omitted_panes(Some(original)).panes,
        Some(empty)
    );
}
