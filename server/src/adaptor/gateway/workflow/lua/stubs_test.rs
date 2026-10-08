use super::*;

#[test]
fn lua_doc_field_quotes_keys_that_are_not_plain_identifiers() {
    assert_eq!(lua_doc_field("coding"), "coding");
    assert_eq!(lua_doc_field("_private"), "_private");
    assert_eq!(
        lua_doc_field("releash-thread-cli"),
        "[\"releash-thread-cli\"]"
    );
    assert_eq!(lua_doc_field("with\"quote"), "[\"with\\\"quote\"]");
}
