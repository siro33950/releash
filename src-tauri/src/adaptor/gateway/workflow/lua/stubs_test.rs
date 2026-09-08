use super::*;

#[test]
fn test_lua述語stub_生成物の型とbuilderが公開apiと一致する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    // When
    generate_editor_support(directory.path()).unwrap();
    let stub = std::fs::read_to_string(directory.path().join(".releash/releash.lua")).unwrap();
    // Then
    assert!(stub.contains("---@class ReleashPredicate"));
    assert!(
        stub.contains("---@class ReleashWhenOptions\n---@field on ReleashSource|ReleashPredicate")
    );
    for builder in ["all", "any"] {
        assert!(stub.contains(&format!("---@field {builder} fun(elements: (ReleashSource|ReleashPredicate)[]): ReleashPredicate")));
    }
    assert!(stub.contains("---@class ReleashSwitchOptions\n---@field on ReleashSource\n"));
}
