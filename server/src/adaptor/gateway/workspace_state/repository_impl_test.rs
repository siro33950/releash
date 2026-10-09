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
