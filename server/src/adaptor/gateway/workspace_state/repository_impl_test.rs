use super::*;

#[test]
fn test_pane形式_既存jsonと全種類と分割軸をdomainとの往復で保つ() {
    // Given
    for axis in ["horizontal", "vertical"] {
        let json = format!(
            r#"{{"kind":"split","id":"root","axis":"{axis}","ratio":0.7,"first":{{"kind":"pane","id":"a","tabs":[{{"id":"terminal","kind":"terminal"}}],"active_tab":"terminal"}},"second":{{"kind":"pane","id":"b","tabs":[{{"id":"workflow","kind":"workflow"}},{{"id":"file","kind":"file"}}],"active_tab":"file"}}}}"#
        );
        // When
        let dto: StoredPaneLayout = serde_json::from_str(&json).unwrap();
        let domain: crate::domain::workspace_state::value_objects::pane_layout::PaneLayout =
            dto.clone().into();
        // Then
        assert!(domain.validate().is_ok());
        assert_eq!(StoredPaneLayout::from(domain), dto);
        assert_eq!(
            serde_json::to_value(dto).unwrap(),
            serde_json::from_str::<serde_json::Value>(&json).unwrap()
        );
    }
}

#[test]
fn test_workspace保存形式_panesのない旧jsonと現在のjsonを直接往復する() {
    // Given
    let legacy = serde_json::json!({
        "version": 1, "tabs": {"editors": [{"path":"file", "name":"File"}], "activeEditorPath":"file"},
        "layout": {"centerTab":"agent", "activeView":"git", "leftNavCollapsed":true, "rightCollapsed":false, "rightBottomCollapsed":false}
    });
    for panes in [
        None,
        Some(serde_json::json!({"kind":"pane", "id":"root", "tabs":[], "active_tab":null})),
    ] {
        let mut json = legacy.clone();
        if let Some(panes) = &panes {
            json["panes"] = panes.clone();
        }
        // When
        let stored: StoredWorkspaceState = serde_json::from_value(json.clone()).unwrap();
        let domain: WorkspaceState = stored.into();
        // Then
        assert_eq!(domain.panes.is_some(), panes.is_some());
        assert_eq!(domain.tabs.editors[0].path, "file");
        assert!(domain.layout.left_nav_collapsed);
        assert_eq!(
            serde_json::to_value(StoredWorkspaceState::from(domain)).unwrap(),
            json
        );
    }
}
