use super::*;
use crate::adaptor::gateway::workflow::workflow_host::prompt_rendering;

#[test]
fn valid_keys() {
    assert!(validate_facet_key("coder").is_ok());
    assert!(validate_facet_key("my-facet").is_ok());
    assert!(validate_facet_key("test_123").is_ok());
    assert!(validate_facet_key("a").is_ok());
    assert!(validate_facet_key("A1-b_c").is_ok());
}

#[test]
fn invalid_keys() {
    assert!(validate_facet_key("").is_err());
    assert!(validate_facet_key("-start").is_err());
    assert!(validate_facet_key("_start").is_err());
    assert!(validate_facet_key("a/b").is_err());
    assert!(validate_facet_key("a b").is_err());
    assert!(validate_facet_key("../evil").is_err());
}

#[test]
fn extract_description_heading() {
    assert_eq!(extract_description("# My Facet\nContent here"), "My Facet");
}

#[test]
fn extract_description_no_heading() {
    assert_eq!(
        extract_description("Some content\nMore content"),
        "Some content"
    );
}

#[test]
fn extract_description_empty() {
    assert_eq!(extract_description(""), "");
}

#[test]
fn extract_description_leading_blank_lines() {
    assert_eq!(extract_description("\n\n# Title\nBody"), "Title");
}

#[test]
fn find_undefined_template_variables_returns_only_invalid_reference_syntax() {
    let content = "{{ goal }} {{ plan.summary }} {{ plan.a.b }} {{ bad ref }}";
    let undefined = prompt_rendering::find_undefined_template_variables(content);
    assert_eq!(undefined, vec!["bad ref".to_string()]);
}

#[test]
fn render_template_variables_treats_surrounding_whitespace_inside_refs_as_equivalent() {
    let mut vars = std::collections::HashMap::new();
    vars.insert("request".to_string(), "do".to_string());
    let out = prompt_rendering::render_template_variables("a {{ request }} b", &vars);
    assert_eq!(out, "a do b");
}

#[test]
fn render_template_variables_keeps_unresolved_ref_verbatim_including_whitespace() {
    // 解決できない参照は元の `{{ ... }}` をそのまま残し、内側のスペースを変更しない。
    let vars = std::collections::HashMap::new();
    let out = prompt_rendering::render_template_variables("x {{ unknown }} y", &vars);
    assert_eq!(out, "x {{ unknown }} y");
}
