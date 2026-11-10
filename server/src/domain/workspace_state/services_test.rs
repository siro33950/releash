use super::*;

#[test]
fn test_表示状態のファイル整理_未保存設定と未選択状態を保持する() {
    // Given
    let state = WorkspaceState::default();
    // When / Then
    assert_eq!(
        filter_missing_files(state.clone(), |_| panic!("no file reference")),
        state
    );
    let mut state = state;
    state.tabs = Some(crate::domain::workspace_state::value_objects::WorkspaceTabsState {
        editors: vec![crate::domain::workspace_state::value_objects::workspace_tabs_state::WorkspaceTabEntry { path: "live".into(), name: "live".into() }],
    });
    assert_eq!(filter_missing_files(state.clone(), |_| true), state);
}
