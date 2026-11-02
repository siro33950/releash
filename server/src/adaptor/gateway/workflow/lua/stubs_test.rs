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
