use super::*;

#[test]
fn test_lua補完_識別子以外のkeyを引用する() {
    // Given
    let cases = [
        ("coding", "coding"),
        ("_private", "_private"),
        ("releash-thread-cli", "[\"releash-thread-cli\"]"),
        ("with\"quote", "[\"with\\\"quote\"]"),
    ];
    // When
    let fields = cases.map(|(key, _)| lua_doc_field(key));
    // Then
    assert_eq!(fields, cases.map(|(_, expected)| expected.to_string()));
}

#[test]
fn test_lua補完_無効なfacetキーを出力しない() {
    // Given
    let dir = tempfile::tempdir().unwrap();
    let policies = dir.path().join("policies");
    fs::create_dir_all(&policies).unwrap();
    let invalid_keys = ["bad key!", "bad\nkey", "bad\\"];
    for key in invalid_keys {
        fs::write(policies.join(format!("{key}.md")), "invalid policy").unwrap();
    }
    fs::write(policies.join("valid-policy.md"), "valid policy").unwrap();

    // When
    let stub = generate_facet_stub(dir.path()).unwrap();

    // Then
    assert!(stub.contains("---@field [\"valid-policy\"] ReleashFacet "));
    for key in invalid_keys {
        assert!(!stub.contains(key));
    }
}
